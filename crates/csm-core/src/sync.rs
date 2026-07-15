use crate::config::AppConfig;
use crate::diff::plan_sync;
use crate::error::Result;
use crate::hash::sha256_hex;
use crate::manifest::{SkillEntry, SkillManifest};
use crate::state::{InstalledSkill, InstalledState};
use std::path::Path;

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
    pub installed: Vec<String>,
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

/// Run one full sync cycle: fetch manifest, install new/changed skills, remove
/// skills dropped from the manifest, and persist the updated state.
///
/// Errors for a single skill are collected in [`SyncOutcome::errors`] rather
/// than aborting the whole run, so one bad skill cannot block the rest. The
/// state file is saved even on partial failure so successful work is not lost.
pub fn run_sync(
    source: &impl SkillSource,
    config: &AppConfig,
    global_dir: &Path,
    state_path: &Path,
) -> Result<SyncOutcome> {
    let mut state = InstalledState::load(state_path)?;
    let manifest = source.fetch_manifest()?;
    let plan = plan_sync(&manifest, &state);
    let mut outcome = SyncOutcome::default();

    for slug in &plan.to_install {
        let entry = match manifest.get(slug) {
            Some(e) => e,
            None => continue, // plan is derived from manifest; unreachable in practice
        };
        match install_one(source, config, global_dir, entry) {
            Ok(content_hash) => {
                state.upsert(
                    entry.slug.clone(),
                    InstalledSkill {
                        version: entry.version.clone(),
                        target: entry.target.clone(),
                        content_hash,
                    },
                );
                outcome.installed.push(slug.clone());
            }
            Err(e) => outcome.errors.push((slug.clone(), e.to_string())),
        }
    }

    for slug in &plan.to_remove {
        let target = state
            .skills
            .get(slug)
            .map(|s| s.target.clone())
            .unwrap_or_else(|| "global".to_string());
        let target_dir = match config.resolve_target(&target, global_dir) {
            Ok(d) => d,
            Err(e) => {
                outcome.errors.push((slug.clone(), e.to_string()));
                continue;
            }
        };
        match remove_managed_skill(&target_dir, slug) {
            Ok(_) => {
                state.remove(slug);
                outcome.removed.push(slug.clone());
            }
            Err(e) => outcome.errors.push((slug.clone(), e.to_string())),
        }
    }

    state.save(state_path)?;
    Ok(outcome)
}

/// Fetch, materialize and install one skill. Returns the content hash.
fn install_one(
    source: &impl SkillSource,
    config: &AppConfig,
    global_dir: &Path,
    entry: &SkillEntry,
) -> Result<String> {
    let instructions = source.fetch_instructions(&entry.slug)?;
    let md = build_skill_md(entry, &instructions);
    let content_hash = sha256_hex(md.as_bytes());
    let target_dir = config.resolve_target(&entry.target, global_dir)?;
    install_skill_atomic(&md, &target_dir, &entry.slug)?;
    Ok(content_hash)
}

/// Write `<target_dir>/<slug>/SKILL.md` atomically.
///
/// Content is written to a staging directory and swapped into place only once
/// complete, so a consumer never observes a half-written skill directory.
fn install_skill_atomic(md: &str, target_dir: &Path, slug: &str) -> Result<()> {
    std::fs::create_dir_all(target_dir)?;
    let final_dir = target_dir.join(slug);
    let staging = target_dir.join(format!(".csm-staging-{slug}"));

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

/// Delete `<target_dir>/<slug>/` only if it carries the managed marker.
/// Returns whether anything was removed.
fn remove_managed_skill(target_dir: &Path, slug: &str) -> Result<bool> {
    let dir = target_dir.join(slug);
    if dir.join(MANAGED_MARKER).is_file() {
        std::fs::remove_dir_all(&dir)?;
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
    use std::path::PathBuf;

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
        config: AppConfig,
    }

    fn fixture() -> Fixture {
        let tmp = tempfile::tempdir().unwrap();
        Fixture {
            global_dir: tmp.path().join("global-skills"),
            state_path: tmp.path().join("state.json"),
            config: AppConfig::default(),
            _tmp: tmp,
        }
    }

    #[test]
    fn build_skill_md_has_frontmatter() {
        let e = entry("bonjour", "1.0.0", Some("répond bonjour"));
        let md = build_skill_md(&e, "# Contexte client\n\nfais X\n");
        assert!(md.starts_with("---\nname: bonjour\ndescription: \"répond bonjour\"\n---\n"));
        assert!(md.contains("fais X"));
    }

    #[test]
    fn yaml_quote_escapes_specials() {
        assert_eq!(yaml_quote("a: b"), "\"a: b\"");
        assert_eq!(yaml_quote("say \"hi\""), "\"say \\\"hi\\\"\"");
    }

    #[test]
    fn fresh_install_writes_skill_and_marker() {
        let f = fixture();
        let source = MockSource {
            manifest: SkillManifest {
                skills: vec![entry("bonjour", "1.0.0", Some("répond bonjour"))],
            },
            instructions: HashMap::from([("bonjour".to_string(), "répond bonjour".to_string())]),
        };

        let out = run_sync(&source, &f.config, &f.global_dir, &f.state_path).unwrap();
        assert_eq!(out.installed, vec!["bonjour"]);
        assert!(out.is_clean());

        let dir = f.global_dir.join("bonjour");
        let md = std::fs::read_to_string(dir.join(SKILL_FILE)).unwrap();
        assert!(md.contains("name: bonjour"));
        assert!(md.contains("répond bonjour"));
        assert!(dir.join(MANAGED_MARKER).is_file());

        let state = InstalledState::load(&f.state_path).unwrap();
        assert_eq!(state.skills["bonjour"].version, "1.0.0");
    }

    #[test]
    fn same_version_second_sync_is_noop() {
        let f = fixture();
        let source = MockSource {
            manifest: SkillManifest {
                skills: vec![entry("bonjour", "1.0.0", None)],
            },
            instructions: HashMap::from([("bonjour".to_string(), "hi".to_string())]),
        };
        run_sync(&source, &f.config, &f.global_dir, &f.state_path).unwrap();
        let out = run_sync(&source, &f.config, &f.global_dir, &f.state_path).unwrap();
        assert!(!out.changed());
    }

    #[test]
    fn version_bump_reinstalls() {
        let f = fixture();
        let v1 = MockSource {
            manifest: SkillManifest {
                skills: vec![entry("bonjour", "1.0.0", None)],
            },
            instructions: HashMap::from([("bonjour".to_string(), "v1 body".to_string())]),
        };
        run_sync(&v1, &f.config, &f.global_dir, &f.state_path).unwrap();

        let v2 = MockSource {
            manifest: SkillManifest {
                skills: vec![entry("bonjour", "2.0.0", None)],
            },
            instructions: HashMap::from([("bonjour".to_string(), "v2 body".to_string())]),
        };
        let out = run_sync(&v2, &f.config, &f.global_dir, &f.state_path).unwrap();
        assert_eq!(out.installed, vec!["bonjour"]);

        let md = std::fs::read_to_string(f.global_dir.join("bonjour").join(SKILL_FILE)).unwrap();
        assert!(md.contains("v2 body"));
        assert!(!md.contains("v1 body"));
    }

    #[test]
    fn fetch_error_is_reported_not_fatal() {
        let f = fixture();
        // Manifest lists two skills but only one has instructions available.
        let source = MockSource {
            manifest: SkillManifest {
                skills: vec![entry("ok", "1", None), entry("broken", "1", None)],
            },
            instructions: HashMap::from([("ok".to_string(), "fine".to_string())]),
        };
        let out = run_sync(&source, &f.config, &f.global_dir, &f.state_path).unwrap();
        assert_eq!(out.installed, vec!["ok"]);
        assert_eq!(out.errors.len(), 1);
        assert_eq!(out.errors[0].0, "broken");
    }

    #[test]
    fn removed_from_manifest_deletes_managed_dir() {
        let f = fixture();
        let install = MockSource {
            manifest: SkillManifest {
                skills: vec![entry("bonjour", "1", None)],
            },
            instructions: HashMap::from([("bonjour".to_string(), "hi".to_string())]),
        };
        run_sync(&install, &f.config, &f.global_dir, &f.state_path).unwrap();
        assert!(f.global_dir.join("bonjour").exists());

        let empty = MockSource {
            manifest: SkillManifest::default(),
            instructions: HashMap::new(),
        };
        let out = run_sync(&empty, &f.config, &f.global_dir, &f.state_path).unwrap();
        assert_eq!(out.removed, vec!["bonjour"]);
        assert!(!f.global_dir.join("bonjour").exists());
        assert!(InstalledState::load(&f.state_path).unwrap().skills.is_empty());
    }

    #[test]
    fn removal_never_touches_unmanaged_dir() {
        let f = fixture();
        let user_dir = f.global_dir.join("handmade");
        std::fs::create_dir_all(&user_dir).unwrap();
        std::fs::write(user_dir.join("keep.txt"), b"precious").unwrap();

        let mut state = InstalledState::default();
        state.upsert(
            "handmade",
            InstalledSkill {
                version: "1".into(),
                target: "global".into(),
                content_hash: "h".into(),
            },
        );
        state.save(&f.state_path).unwrap();

        let empty = MockSource {
            manifest: SkillManifest::default(),
            instructions: HashMap::new(),
        };
        let out = run_sync(&empty, &f.config, &f.global_dir, &f.state_path).unwrap();
        assert_eq!(out.removed, vec!["handmade"]);
        assert!(user_dir.join("keep.txt").is_file());
    }
}
