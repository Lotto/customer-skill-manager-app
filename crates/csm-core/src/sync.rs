use crate::config::AppConfig;
use crate::diff::plan_sync;
use crate::error::Result;
use crate::hash::sha256_hex;
use crate::manifest::{SkillEntry, SkillManifest};
use crate::state::{InstalledSkill, InstalledState};
use std::collections::{BTreeMap, BTreeSet, HashSet};
use std::path::{Path, PathBuf};

/// Marker file written at the root of every skill directory CSM installs.
///
/// Removal is gated on this marker's presence, so the app can never delete a
/// directory a user created by hand in a target folder.
pub const MANAGED_MARKER: &str = ".csm-managed";

/// Filename of the materialized skill instructions.
pub const SKILL_FILE: &str = "SKILL.md";

/// Where skill content comes from. Abstracted so the engine can be tested with
/// an in-memory source, and so the reqwest-based HTTP client can be swapped in
/// without the core depending on it.
pub trait SkillSource {
    /// Fetch and parse the `__list` resource into a manifest.
    fn fetch_manifest(&self) -> Result<SkillManifest>;
    /// Fetch the `instructions` markdown for one skill slug.
    fn fetch_instructions(&self, slug: &str) -> Result<String>;
}

/// What a sync actually did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncOutcome {
    /// Slugs that were (re)written into at least one directory.
    pub installed: Vec<String>,
    /// Slugs removed entirely (no longer entitled).
    pub removed: Vec<String>,
    /// Per-skill failures: `(slug, message)`. A failure on one skill does not
    /// abort the others.
    pub errors: Vec<(String, String)>,
}

impl SyncOutcome {
    pub fn is_clean(&self) -> bool {
        self.errors.is_empty()
    }
    pub fn changed(&self) -> bool {
        !self.installed.is_empty() || !self.removed.is_empty()
    }
}

/// Build the on-disk `SKILL.md` for a skill: YAML frontmatter (so it is a valid
/// Claude Code skill) followed by the instructions body from the backend.
pub fn build_skill_md(entry: &SkillEntry, instructions: &str) -> String {
    format!(
        "---\nname: {name}\ndescription: {desc}\n---\n\n{body}\n",
        name = entry.slug,
        desc = yaml_quote(entry.display_description()),
        body = instructions.trim_end(),
    )
}

/// Quote a string for safe use as a YAML scalar value (handles colons, `#`,
/// leading indicators, etc. by always double-quoting and escaping).
fn yaml_quote(s: &str) -> String {
    let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
    format!("\"{escaped}\"")
}

fn dir_key(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

/// Run one full sync cycle: fetch the manifest, install every entitled skill
/// into every configured directory, remove skills/dirs that are no longer
/// wanted, and persist the updated state.
///
/// Errors for a single skill are collected in [`SyncOutcome::errors`] rather
/// than aborting the whole run. The state file is saved even on partial failure
/// so successful work is not lost.
pub fn run_sync(
    source: &impl SkillSource,
    config: &AppConfig,
    global_dir: &Path,
    state_path: &Path,
) -> Result<SyncOutcome> {
    let state = InstalledState::load(state_path)?;
    let manifest = source.fetch_manifest()?;
    let effective_dirs = config.effective_skill_dirs(global_dir);
    let plan = plan_sync(&manifest, &state, &effective_dirs);

    let mut outcome = SyncOutcome::default();

    // Group installs by slug so each skill is fetched once and fanned out to all
    // the directories that need it.
    let mut installs_by_slug: BTreeMap<String, Vec<PathBuf>> = BTreeMap::new();
    for (slug, dir) in &plan.installs {
        installs_by_slug
            .entry(slug.clone())
            .or_default()
            .push(dir.clone());
    }

    let mut fetched_hash: BTreeMap<String, String> = BTreeMap::new();
    let mut failed: HashSet<(String, String)> = HashSet::new();

    for (slug, wanted_dirs) in &installs_by_slug {
        let entry = match manifest.get(slug) {
            Some(e) => e,
            None => continue, // plan derives from manifest; unreachable in practice
        };
        match source.fetch_instructions(slug) {
            Ok(instructions) => {
                let md = build_skill_md(entry, &instructions);
                fetched_hash.insert(slug.clone(), sha256_hex(md.as_bytes()));
                for dir in wanted_dirs {
                    if let Err(e) = install_skill_atomic(&md, dir, slug) {
                        failed.insert((slug.clone(), dir_key(dir)));
                        outcome
                            .errors
                            .push((slug.clone(), format!("{} @ {}: {e}", slug, dir.display())));
                    }
                }
            }
            Err(e) => {
                for dir in wanted_dirs {
                    failed.insert((slug.clone(), dir_key(dir)));
                }
                outcome.errors.push((slug.clone(), e.to_string()));
            }
        }
    }

    // Removals (skill dropped, or directory de-configured).
    for (slug, dir) in &plan.removes {
        if let Err(e) = remove_managed_skill(dir, slug) {
            outcome
                .errors
                .push((slug.clone(), format!("remove {} @ {}: {e}", slug, dir.display())));
        }
    }

    // Rebuild the state from the manifest and what is actually on disk now.
    let mut new_state = InstalledState::default();
    for entry in &manifest.skills {
        let old = state.skills.get(&entry.slug);
        let mut final_dirs = Vec::new();
        for dir in &effective_dirs {
            let key = dir_key(dir);
            let was_current = old
                .is_some_and(|o| o.version == entry.version && o.dirs.iter().any(|d| d == &key));
            let just_installed = installs_by_slug
                .get(&entry.slug)
                .is_some_and(|ds| ds.contains(dir))
                && !failed.contains(&(entry.slug.clone(), key.clone()));
            if was_current || just_installed {
                final_dirs.push(key);
            }
        }
        if final_dirs.is_empty() {
            continue;
        }
        let content_hash = fetched_hash
            .get(&entry.slug)
            .cloned()
            .or_else(|| old.map(|o| o.content_hash.clone()))
            .unwrap_or_default();
        new_state.upsert(
            entry.slug.clone(),
            InstalledSkill {
                version: entry.version.clone(),
                content_hash,
                dirs: final_dirs,
            },
        );
    }
    new_state.save(state_path)?;

    // Report: installed = slugs with at least one successful write; removed =
    // slugs that left the manifest entirely.
    for (slug, wanted_dirs) in &installs_by_slug {
        if wanted_dirs
            .iter()
            .any(|d| !failed.contains(&(slug.clone(), dir_key(d))))
        {
            outcome.installed.push(slug.clone());
        }
    }
    outcome.installed.sort();
    let removed: BTreeSet<String> = plan
        .removes
        .iter()
        .filter(|(slug, _)| manifest.get(slug).is_none())
        .map(|(slug, _)| slug.clone())
        .collect();
    outcome.removed = removed.into_iter().collect();

    Ok(outcome)
}

/// Write `<dir>/<slug>/SKILL.md` atomically (staging dir + swap), plus the
/// managed marker.
fn install_skill_atomic(md: &str, dir: &Path, slug: &str) -> Result<()> {
    std::fs::create_dir_all(dir)?;
    let final_dir = dir.join(slug);
    let staging = dir.join(format!(".csm-staging-{slug}"));

    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;
    std::fs::write(staging.join(SKILL_FILE), md)?;
    std::fs::write(
        staging.join(MANAGED_MARKER),
        b"Managed by Customer Skill Manager. Do not edit or remove.\n",
    )?;

    if final_dir.exists() {
        std::fs::remove_dir_all(&final_dir)?;
    }
    std::fs::rename(&staging, &final_dir)?;
    Ok(())
}

/// Delete `<dir>/<slug>/` only if it carries the managed marker.
fn remove_managed_skill(dir: &Path, slug: &str) -> Result<bool> {
    let target = dir.join(slug);
    if target.join(MANAGED_MARKER).is_file() {
        std::fs::remove_dir_all(&target)?;
        Ok(true)
    } else {
        Ok(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::error::CoreError;
    use std::collections::HashMap;

    struct MockSource {
        manifest: SkillManifest,
        instructions: HashMap<String, String>,
    }

    impl SkillSource for MockSource {
        fn fetch_manifest(&self) -> Result<SkillManifest> {
            Ok(self.manifest.clone())
        }
        fn fetch_instructions(&self, slug: &str) -> Result<String> {
            self.instructions
                .get(slug)
                .cloned()
                .ok_or_else(|| CoreError::Http(format!("no instructions for {slug}")))
        }
    }

    fn entry(slug: &str, version: &str, desc: Option<&str>) -> SkillEntry {
        SkillEntry {
            slug: slug.into(),
            description: desc.map(Into::into),
            version: version.into(),
            target: "global".into(),
        }
    }

    struct Fixture {
        _tmp: tempfile::TempDir,
        global_dir: PathBuf,
        state_path: PathBuf,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        Fixture {
            global_dir: tmp.path().join("global"),
            state_path: tmp.path().join("state.json"),
            _tmp: tmp,
        }
    }

    fn cfg_with_dirs(dirs: &[&Path]) -> AppConfig {
        AppConfig {
            skill_dirs: dirs.iter().map(|d| d.to_path_buf()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn build_skill_md_has_frontmatter() {
        let e = entry("bonjour", "1.0.0", Some("répond bonjour"));
        let md = build_skill_md(&e, "# Contexte\n\nfais X\n");
        assert!(md.starts_with("---\nname: bonjour\ndescription: \"répond bonjour\"\n---\n"));
        assert!(md.contains("fais X"));
    }

    #[test]
    fn fresh_install_into_default_global_dir() {
        let f = fixture();
        let source = MockSource {
            manifest: SkillManifest {
                skills: vec![entry("bonjour", "1.0.0", Some("répond bonjour"))],
            },
            instructions: HashMap::from([("bonjour".to_string(), "hi".to_string())]),
        };
        // No skill_dirs configured -> uses global_dir.
        let out = run_sync(&source, &AppConfig::default(), &f.global_dir, &f.state_path).unwrap();
        assert_eq!(out.installed, vec!["bonjour"]);
        assert!(f.global_dir.join("bonjour").join(SKILL_FILE).is_file());
        assert!(f.global_dir.join("bonjour").join(MANAGED_MARKER).is_file());

        let state = InstalledState::load(&f.state_path).unwrap();
        assert_eq!(state.skills["bonjour"].dirs.len(), 1);
    }

    #[test]
    fn installs_into_multiple_dirs() {
        let f = fixture();
        let d1 = f._tmp.path().join("dir1");
        let d2 = f._tmp.path().join("dir2");
        let source = MockSource {
            manifest: SkillManifest {
                skills: vec![entry("bonjour", "1.0.0", None)],
            },
            instructions: HashMap::from([("bonjour".to_string(), "hi".to_string())]),
        };
        let cfg = cfg_with_dirs(&[&d1, &d2]);
        let out = run_sync(&source, &cfg, &f.global_dir, &f.state_path).unwrap();
        assert_eq!(out.installed, vec!["bonjour"]);
        assert!(d1.join("bonjour").join(SKILL_FILE).is_file());
        assert!(d2.join("bonjour").join(SKILL_FILE).is_file());

        let state = InstalledState::load(&f.state_path).unwrap();
        assert_eq!(state.skills["bonjour"].dirs.len(), 2);
    }

    #[test]
    fn second_sync_same_version_is_noop() {
        let f = fixture();
        let d1 = f._tmp.path().join("dir1");
        let source = MockSource {
            manifest: SkillManifest {
                skills: vec![entry("bonjour", "1.0.0", None)],
            },
            instructions: HashMap::from([("bonjour".to_string(), "hi".to_string())]),
        };
        let cfg = cfg_with_dirs(&[&d1]);
        run_sync(&source, &cfg, &f.global_dir, &f.state_path).unwrap();
        let out = run_sync(&source, &cfg, &f.global_dir, &f.state_path).unwrap();
        assert!(!out.changed());
    }

    #[test]
    fn adding_a_dir_installs_only_there() {
        let f = fixture();
        let d1 = f._tmp.path().join("dir1");
        let d2 = f._tmp.path().join("dir2");
        let source = MockSource {
            manifest: SkillManifest {
                skills: vec![entry("bonjour", "1.0.0", None)],
            },
            instructions: HashMap::from([("bonjour".to_string(), "hi".to_string())]),
        };
        run_sync(&source, &cfg_with_dirs(&[&d1]), &f.global_dir, &f.state_path).unwrap();
        // Now add d2.
        let out = run_sync(&source, &cfg_with_dirs(&[&d1, &d2]), &f.global_dir, &f.state_path).unwrap();
        assert_eq!(out.installed, vec!["bonjour"]);
        assert!(d2.join("bonjour").join(SKILL_FILE).is_file());
        assert_eq!(
            InstalledState::load(&f.state_path).unwrap().skills["bonjour"].dirs.len(),
            2
        );
    }

    #[test]
    fn removing_a_dir_cleans_that_dir_only() {
        let f = fixture();
        let d1 = f._tmp.path().join("dir1");
        let d2 = f._tmp.path().join("dir2");
        let source = MockSource {
            manifest: SkillManifest {
                skills: vec![entry("bonjour", "1.0.0", None)],
            },
            instructions: HashMap::from([("bonjour".to_string(), "hi".to_string())]),
        };
        run_sync(&source, &cfg_with_dirs(&[&d1, &d2]), &f.global_dir, &f.state_path).unwrap();
        assert!(d2.join("bonjour").exists());
        // Drop d2.
        run_sync(&source, &cfg_with_dirs(&[&d1]), &f.global_dir, &f.state_path).unwrap();
        assert!(d1.join("bonjour").join(SKILL_FILE).is_file());
        assert!(!d2.join("bonjour").exists());
        assert_eq!(
            InstalledState::load(&f.state_path).unwrap().skills["bonjour"].dirs,
            vec![d1.to_string_lossy().to_string()]
        );
    }

    #[test]
    fn dropped_skill_removed_from_all_dirs() {
        let f = fixture();
        let d1 = f._tmp.path().join("dir1");
        let d2 = f._tmp.path().join("dir2");
        let install = MockSource {
            manifest: SkillManifest {
                skills: vec![entry("bonjour", "1.0.0", None)],
            },
            instructions: HashMap::from([("bonjour".to_string(), "hi".to_string())]),
        };
        run_sync(&install, &cfg_with_dirs(&[&d1, &d2]), &f.global_dir, &f.state_path).unwrap();

        let empty = MockSource {
            manifest: SkillManifest::default(),
            instructions: HashMap::new(),
        };
        let out = run_sync(&empty, &cfg_with_dirs(&[&d1, &d2]), &f.global_dir, &f.state_path).unwrap();
        assert_eq!(out.removed, vec!["bonjour"]);
        assert!(!d1.join("bonjour").exists());
        assert!(!d2.join("bonjour").exists());
        assert!(InstalledState::load(&f.state_path).unwrap().skills.is_empty());
    }

    #[test]
    fn removal_never_touches_unmanaged_dir() {
        let f = fixture();
        let d1 = f._tmp.path().join("dir1");
        let user_dir = d1.join("handmade");
        std::fs::create_dir_all(&user_dir).unwrap();
        std::fs::write(user_dir.join("keep.txt"), b"precious").unwrap();

        let mut state = InstalledState::default();
        state.upsert(
            "handmade",
            InstalledSkill {
                version: "1".into(),
                content_hash: "h".into(),
                dirs: vec![d1.to_string_lossy().to_string()],
            },
        );
        state.save(&f.state_path).unwrap();

        let empty = MockSource {
            manifest: SkillManifest::default(),
            instructions: HashMap::new(),
        };
        let out = run_sync(&empty, &cfg_with_dirs(&[&d1]), &f.global_dir, &f.state_path).unwrap();
        assert_eq!(out.removed, vec!["handmade"]);
        assert!(user_dir.join("keep.txt").is_file()); // untouched (no marker)
    }

    #[test]
    fn version_bump_replaces_content_everywhere() {
        let f = fixture();
        let d1 = f._tmp.path().join("dir1");
        let v1 = MockSource {
            manifest: SkillManifest {
                skills: vec![entry("bonjour", "1.0.0", None)],
            },
            instructions: HashMap::from([("bonjour".to_string(), "v1 body".to_string())]),
        };
        run_sync(&v1, &cfg_with_dirs(&[&d1]), &f.global_dir, &f.state_path).unwrap();

        let v2 = MockSource {
            manifest: SkillManifest {
                skills: vec![entry("bonjour", "2.0.0", None)],
            },
            instructions: HashMap::from([("bonjour".to_string(), "v2 body".to_string())]),
        };
        let out = run_sync(&v2, &cfg_with_dirs(&[&d1]), &f.global_dir, &f.state_path).unwrap();
        assert_eq!(out.installed, vec!["bonjour"]);
        let md = std::fs::read_to_string(d1.join("bonjour").join(SKILL_FILE)).unwrap();
        assert!(md.contains("v2 body"));
    }

    #[test]
    fn fetch_error_is_reported_not_fatal() {
        let f = fixture();
        let source = MockSource {
            manifest: SkillManifest {
                skills: vec![entry("ok", "1", None), entry("broken", "1", None)],
            },
            instructions: HashMap::from([("ok".to_string(), "fine".to_string())]),
        };
        let out = run_sync(&source, &AppConfig::default(), &f.global_dir, &f.state_path).unwrap();
        assert_eq!(out.installed, vec!["ok"]);
        assert_eq!(out.errors.len(), 1);
        assert!(f.global_dir.join("ok").exists());
        assert!(!f.global_dir.join("broken").exists());
    }
}
