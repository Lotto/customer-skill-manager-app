use crate::manifest::SkillManifest;
use crate::state::InstalledState;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// What a sync should do, at (skill, directory) granularity.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncPlan {
    /// `(slug, dir)` pairs to (re)materialize — new skill, changed version, or a
    /// newly-added directory that doesn't have the skill yet.
    pub installs: Vec<(String, PathBuf)>,
    /// `(slug, dir)` pairs to delete — the skill left the manifest, or the
    /// directory is no longer configured.
    pub removes: Vec<(String, PathBuf)>,
}

impl SyncPlan {
    pub fn is_noop(&self) -> bool {
        self.installs.is_empty() && self.removes.is_empty()
    }
}

fn dir_key(p: &Path) -> String {
    p.to_string_lossy().to_string()
}

/// Compute the sync plan from the manifest, installed state and the currently
/// effective destination directories.
///
/// A `(slug, dir)` is installed when the skill is entitled but that directory
/// does not already hold the current version. A `(slug, dir)` is removed when
/// the skill is no longer entitled, or the directory is no longer configured —
/// and only for directories the state records, so nothing outside CSM's
/// bookkeeping is ever touched. Both lists are sorted for deterministic output.
pub fn plan_sync(
    manifest: &SkillManifest,
    state: &InstalledState,
    effective_dirs: &[PathBuf],
) -> SyncPlan {
    let mut installs = Vec::new();
    for entry in &manifest.skills {
        let installed = state.skills.get(&entry.slug);
        for dir in effective_dirs {
            let present_current = installed.is_some_and(|s| {
                s.version == entry.version && s.dirs.iter().any(|d| d == &dir_key(dir))
            });
            if !present_current {
                installs.push((entry.slug.clone(), dir.clone()));
            }
        }
    }

    let manifest_slugs: HashSet<&String> = manifest.skills.iter().map(|s| &s.slug).collect();
    let effective_keys: HashSet<String> = effective_dirs.iter().map(|d| dir_key(d)).collect();

    let mut removes = Vec::new();
    for (slug, installed) in &state.skills {
        for dir_str in &installed.dirs {
            let gone_from_manifest = !manifest_slugs.contains(slug);
            let gone_from_config = !effective_keys.contains(dir_str);
            if gone_from_manifest || gone_from_config {
                removes.push((slug.clone(), PathBuf::from(dir_str)));
            }
        }
    }

    installs.sort();
    removes.sort();
    SyncPlan { installs, removes }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::SkillEntry;
    use crate::state::InstalledSkill;

    fn entry(slug: &str, version: &str) -> SkillEntry {
        SkillEntry {
            slug: slug.into(),
            description: None,
            version: version.into(),
            target: "global".into(),
        }
    }

    fn installed(version: &str, dirs: &[&str]) -> InstalledSkill {
        InstalledSkill {
            version: version.into(),
            content_hash: "h".into(),
            dirs: dirs.iter().map(|d| d.to_string()).collect(),
        }
    }

    fn dirs(list: &[&str]) -> Vec<PathBuf> {
        list.iter().map(PathBuf::from).collect()
    }

    #[test]
    fn empty_is_noop() {
        let plan = plan_sync(
            &SkillManifest::default(),
            &InstalledState::default(),
            &dirs(&["/a"]),
        );
        assert!(plan.is_noop());
    }

    #[test]
    fn new_skill_installs_in_every_dir() {
        let manifest = SkillManifest {
            skills: vec![entry("a", "1")],
        };
        let plan = plan_sync(&manifest, &InstalledState::default(), &dirs(&["/x", "/y"]));
        assert_eq!(
            plan.installs,
            vec![
                ("a".into(), PathBuf::from("/x")),
                ("a".into(), PathBuf::from("/y"))
            ]
        );
        assert!(plan.removes.is_empty());
    }

    #[test]
    fn same_version_all_dirs_present_is_noop() {
        let manifest = SkillManifest {
            skills: vec![entry("a", "1")],
        };
        let mut state = InstalledState::default();
        state.upsert("a", installed("1", &["/x", "/y"]));
        assert!(plan_sync(&manifest, &state, &dirs(&["/x", "/y"])).is_noop());
    }

    #[test]
    fn new_dir_installs_only_the_missing_dir() {
        let manifest = SkillManifest {
            skills: vec![entry("a", "1")],
        };
        let mut state = InstalledState::default();
        state.upsert("a", installed("1", &["/x"]));
        let plan = plan_sync(&manifest, &state, &dirs(&["/x", "/y"]));
        assert_eq!(plan.installs, vec![("a".into(), PathBuf::from("/y"))]);
        assert!(plan.removes.is_empty());
    }

    #[test]
    fn version_bump_reinstalls_all_dirs() {
        let manifest = SkillManifest {
            skills: vec![entry("a", "2")],
        };
        let mut state = InstalledState::default();
        state.upsert("a", installed("1", &["/x", "/y"]));
        let plan = plan_sync(&manifest, &state, &dirs(&["/x", "/y"]));
        assert_eq!(plan.installs.len(), 2);
    }

    #[test]
    fn dropped_skill_is_removed_from_all_its_dirs() {
        let manifest = SkillManifest::default();
        let mut state = InstalledState::default();
        state.upsert("gone", installed("1", &["/x", "/y"]));
        let plan = plan_sync(&manifest, &state, &dirs(&["/x", "/y"]));
        assert_eq!(
            plan.removes,
            vec![
                ("gone".into(), PathBuf::from("/x")),
                ("gone".into(), PathBuf::from("/y"))
            ]
        );
    }

    #[test]
    fn dropped_dir_removes_skill_from_that_dir_and_reinstalls_nowhere_new() {
        let manifest = SkillManifest {
            skills: vec![entry("a", "1")],
        };
        let mut state = InstalledState::default();
        state.upsert("a", installed("1", &["/x", "/y"]));
        // /y removed from config.
        let plan = plan_sync(&manifest, &state, &dirs(&["/x"]));
        assert!(plan.installs.is_empty());
        assert_eq!(plan.removes, vec![("a".into(), PathBuf::from("/y"))]);
    }
}
