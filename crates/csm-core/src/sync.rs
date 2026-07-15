use crate::config::AppConfig;
use crate::diff::plan_sync;
use crate::error::{CoreError, Result};
use crate::hash::sha256_hex;
use crate::manifest::{SkillEntry, SkillManifest};
use crate::state::{InstalledSkill, InstalledState};
use flate2::read::GzDecoder;
use std::path::Path;
use tar::Archive;

/// Marker file written at the root of every skill directory CSM installs.
///
/// Removal is gated on this marker's presence, so the app can never delete a
/// directory a user created by hand in a target folder.
pub const MANAGED_MARKER: &str = ".csm-managed";

/// Where skill archives come from. Abstracted so the engine can be tested with
/// an in-memory source, and so the reqwest-based HTTP client can live in the
/// GUI crate without dragging its dependencies into the core.
pub trait SkillSource {
    /// Fetch the current manifest for this license.
    fn fetch_manifest(&self) -> Result<SkillManifest>;
    /// Download the `.tar.gz` archive bytes for one skill.
    fn download_skill(&self, entry: &SkillEntry) -> Result<Vec<u8>>;
}

/// What a sync actually did.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct SyncOutcome {
    pub installed: Vec<String>,
    pub removed: Vec<String>,
    /// Per-skill failures: `(skill_id, message)`. A failure on one skill does
    /// not abort the others.
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

/// Run one full sync cycle: fetch manifest, install new/changed skills, remove
/// skills dropped from the manifest, and persist the updated state.
///
/// Network/extraction errors for a single skill are collected in
/// [`SyncOutcome::errors`] rather than aborting the whole run, so one bad skill
/// cannot block the rest. The state file is saved even on partial failure so
/// successful work is not lost.
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

    for id in &plan.to_install {
        let entry = match manifest.get(id) {
            Some(e) => e,
            None => continue, // plan is derived from manifest; unreachable in practice
        };
        match install_one(source, config, global_dir, entry) {
            Ok(()) => {
                state.upsert(
                    entry.id.clone(),
                    InstalledSkill {
                        version: entry.version.clone(),
                        hash: entry.hash.clone(),
                        target: entry.target.clone(),
                    },
                );
                outcome.installed.push(id.clone());
            }
            Err(e) => outcome.errors.push((id.clone(), e.to_string())),
        }
    }

    for id in &plan.to_remove {
        let target = state
            .skills
            .get(id)
            .map(|s| s.target.clone())
            .unwrap_or_else(|| "global".to_string());
        let target_dir = match config.resolve_target(&target, global_dir) {
            Ok(d) => d,
            Err(e) => {
                outcome.errors.push((id.clone(), e.to_string()));
                continue;
            }
        };
        match remove_managed_skill(&target_dir, id) {
            Ok(_) => {
                state.remove(id);
                outcome.removed.push(id.clone());
            }
            Err(e) => outcome.errors.push((id.clone(), e.to_string())),
        }
    }

    state.save(state_path)?;
    Ok(outcome)
}

fn install_one(
    source: &impl SkillSource,
    config: &AppConfig,
    global_dir: &Path,
    entry: &SkillEntry,
) -> Result<()> {
    let bytes = source.download_skill(entry)?;
    let actual = sha256_hex(&bytes);
    if actual != entry.hash {
        return Err(CoreError::HashMismatch {
            id: entry.id.clone(),
            expected: entry.hash.clone(),
            actual,
        });
    }
    let target_dir = config.resolve_target(&entry.target, global_dir)?;
    install_skill_atomic(&bytes, &target_dir, &entry.id)
}

/// Extract a `.tar.gz` into `<target_dir>/<id>/`, atomically.
///
/// The archive is expanded into a staging directory first; only once that
/// succeeds and the managed-marker is written do we swap it into place. This
/// guarantees a consumer never observes a half-written skill directory.
fn install_skill_atomic(bytes: &[u8], target_dir: &Path, id: &str) -> Result<()> {
    std::fs::create_dir_all(target_dir)?;
    let final_dir = target_dir.join(id);
    let staging = target_dir.join(format!(".csm-staging-{id}"));

    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;

    // tar's `unpack` refuses entries that escape the destination (path
    // traversal via `..` or absolute paths), so extraction is safe.
    let mut archive = Archive::new(GzDecoder::new(bytes));
    archive.unpack(&staging)?;

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

/// Delete `<target_dir>/<id>/` only if it carries the managed marker.
/// Returns whether anything was removed.
fn remove_managed_skill(target_dir: &Path, id: &str) -> Result<bool> {
    let dir = target_dir.join(id);
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
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::collections::HashMap;
    use std::io::Write;
    use std::path::PathBuf;

    /// Build a `.tar.gz` from `(path, contents)` pairs.
    fn make_targz(files: &[(&str, &[u8])]) -> Vec<u8> {
        let mut builder = tar::Builder::new(GzEncoder::new(Vec::new(), Compression::default()));
        for (name, data) in files {
            let mut header = tar::Header::new_gnu();
            header.set_size(data.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, *data).unwrap();
        }
        builder.into_inner().unwrap().finish().unwrap()
    }

    /// In-memory skill source backed by a fixed manifest and archive bytes.
    struct MockSource {
        manifest: SkillManifest,
        archives: HashMap<String, Vec<u8>>,
    }

    impl SkillSource for MockSource {
        fn fetch_manifest(&self) -> Result<SkillManifest> {
            Ok(self.manifest.clone())
        }
        fn download_skill(&self, entry: &SkillEntry) -> Result<Vec<u8>> {
            self.archives
                .get(&entry.id)
                .cloned()
                .ok_or_else(|| CoreError::Http(format!("no archive for {}", entry.id)))
        }
    }

    fn entry_for(id: &str, archive: &[u8], target: &str) -> SkillEntry {
        SkillEntry {
            id: id.into(),
            name: id.into(),
            version: "1.0.0".into(),
            hash: sha256_hex(archive),
            target: target.into(),
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
        let global_dir = tmp.path().join("global-skills");
        let state_path = tmp.path().join("state.json");
        Fixture {
            _tmp: tmp,
            global_dir,
            state_path,
            config: AppConfig::default(),
        }
    }

    #[test]
    fn fresh_install_writes_files_and_marker() {
        let f = fixture();
        let archive = make_targz(&[("SKILL.md", b"# Devis\n"), ("prompt.txt", b"hello")]);
        let source = MockSource {
            manifest: SkillManifest {
                skills: vec![entry_for("devis", &archive, "global")],
            },
            archives: HashMap::from([("devis".to_string(), archive)]),
        };

        let out = run_sync(&source, &f.config, &f.global_dir, &f.state_path).unwrap();
        assert_eq!(out.installed, vec!["devis"]);
        assert!(out.is_clean());

        let dir = f.global_dir.join("devis");
        assert_eq!(
            std::fs::read_to_string(dir.join("SKILL.md")).unwrap(),
            "# Devis\n"
        );
        assert!(dir.join(MANAGED_MARKER).is_file());

        // State now records the skill.
        let state = InstalledState::load(&f.state_path).unwrap();
        assert!(state.skills.contains_key("devis"));
    }

    #[test]
    fn hash_mismatch_is_reported_and_not_installed() {
        let f = fixture();
        let archive = make_targz(&[("SKILL.md", b"content")]);
        let mut entry = entry_for("devis", &archive, "global");
        entry.hash = "0000".into(); // wrong hash
        let source = MockSource {
            manifest: SkillManifest {
                skills: vec![entry],
            },
            archives: HashMap::from([("devis".to_string(), archive)]),
        };

        let out = run_sync(&source, &f.config, &f.global_dir, &f.state_path).unwrap();
        assert!(out.installed.is_empty());
        assert_eq!(out.errors.len(), 1);
        assert!(!f.global_dir.join("devis").exists());
    }

    #[test]
    fn second_sync_with_same_hash_is_noop() {
        let f = fixture();
        let archive = make_targz(&[("SKILL.md", b"v1")]);
        let source = MockSource {
            manifest: SkillManifest {
                skills: vec![entry_for("devis", &archive, "global")],
            },
            archives: HashMap::from([("devis".to_string(), archive)]),
        };
        run_sync(&source, &f.config, &f.global_dir, &f.state_path).unwrap();
        let out = run_sync(&source, &f.config, &f.global_dir, &f.state_path).unwrap();
        assert!(!out.changed());
    }

    #[test]
    fn changed_hash_replaces_content() {
        let f = fixture();
        let v1 = make_targz(&[("SKILL.md", b"v1"), ("old.txt", b"old")]);
        let source1 = MockSource {
            manifest: SkillManifest {
                skills: vec![entry_for("devis", &v1, "global")],
            },
            archives: HashMap::from([("devis".to_string(), v1)]),
        };
        run_sync(&source1, &f.config, &f.global_dir, &f.state_path).unwrap();

        let v2 = make_targz(&[("SKILL.md", b"v2")]);
        let source2 = MockSource {
            manifest: SkillManifest {
                skills: vec![entry_for("devis", &v2, "global")],
            },
            archives: HashMap::from([("devis".to_string(), v2)]),
        };
        let out = run_sync(&source2, &f.config, &f.global_dir, &f.state_path).unwrap();
        assert_eq!(out.installed, vec!["devis"]);

        let dir = f.global_dir.join("devis");
        assert_eq!(std::fs::read_to_string(dir.join("SKILL.md")).unwrap(), "v2");
        // Stale file from v1 is gone after the atomic swap.
        assert!(!dir.join("old.txt").exists());
    }

    #[test]
    fn removed_from_manifest_deletes_managed_dir() {
        let f = fixture();
        let archive = make_targz(&[("SKILL.md", b"x")]);
        let install = MockSource {
            manifest: SkillManifest {
                skills: vec![entry_for("devis", &archive, "global")],
            },
            archives: HashMap::from([("devis".to_string(), archive)]),
        };
        run_sync(&install, &f.config, &f.global_dir, &f.state_path).unwrap();
        assert!(f.global_dir.join("devis").exists());

        // Now the manifest no longer lists it.
        let empty = MockSource {
            manifest: SkillManifest::default(),
            archives: HashMap::new(),
        };
        let out = run_sync(&empty, &f.config, &f.global_dir, &f.state_path).unwrap();
        assert_eq!(out.removed, vec!["devis"]);
        assert!(!f.global_dir.join("devis").exists());
        assert!(InstalledState::load(&f.state_path).unwrap().skills.is_empty());
    }

    #[test]
    fn removal_never_touches_unmanaged_dir() {
        // Simulate a stale state entry whose on-disk dir has no marker (e.g. a
        // dir the user created by hand). The engine must not delete it.
        let f = fixture();
        let user_dir = f.global_dir.join("handmade");
        std::fs::create_dir_all(&user_dir).unwrap();
        std::fs::write(user_dir.join("keep.txt"), b"precious").unwrap();

        let mut state = InstalledState::default();
        state.upsert(
            "handmade",
            InstalledSkill {
                version: "1".into(),
                hash: "h".into(),
                target: "global".into(),
            },
        );
        state.save(&f.state_path).unwrap();

        let empty = MockSource {
            manifest: SkillManifest::default(),
            archives: HashMap::new(),
        };
        let out = run_sync(&empty, &f.config, &f.global_dir, &f.state_path).unwrap();
        // Reported as "removed" from state, but the files are left untouched.
        assert_eq!(out.removed, vec!["handmade"]);
        assert!(user_dir.join("keep.txt").is_file());
    }
}
