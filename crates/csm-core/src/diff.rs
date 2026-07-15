use crate::manifest::SkillManifest;
use crate::state::InstalledState;
use std::collections::HashSet;

/// What a sync should do, derived purely from (manifest, installed state).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncPlan {
    /// Skills to download and install — either new, or whose hash changed.
    pub to_install: Vec<String>,
    /// Skills to remove — present in local state but gone from the manifest.
    pub to_remove: Vec<String>,
}

impl SyncPlan {
    pub fn is_noop(&self) -> bool {
        self.to_install.is_empty() && self.to_remove.is_empty()
    }
}

/// Compute the sync plan.
///
/// A skill is (re)installed when it is absent locally or its manifest hash
/// differs from the installed hash. A skill is removed when it exists in local
/// state but no longer appears in the manifest — this is why the app never
/// deletes files it did not install: removal is keyed off `InstalledState`.
///
/// Both output lists are sorted for deterministic, log-friendly ordering.
pub fn plan_sync(manifest: &SkillManifest, state: &InstalledState) -> SyncPlan {
    let mut to_install: Vec<String> = manifest
        .skills
        .iter()
        .filter(|entry| match state.skills.get(&entry.id) {
            Some(installed) => installed.hash != entry.hash,
            None => true,
        })
        .map(|entry| entry.id.clone())
        .collect();

    let manifest_ids: HashSet<&String> = manifest.skills.iter().map(|s| &s.id).collect();
    let mut to_remove: Vec<String> = state
        .skills
        .keys()
        .filter(|id| !manifest_ids.contains(id))
        .cloned()
        .collect();

    to_install.sort();
    to_remove.sort();
    SyncPlan {
        to_install,
        to_remove,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::SkillEntry;
    use crate::state::InstalledSkill;

    fn entry(id: &str, hash: &str) -> SkillEntry {
        SkillEntry {
            id: id.into(),
            name: id.into(),
            version: "1.0.0".into(),
            hash: hash.into(),
            target: "global".into(),
        }
    }

    fn installed(hash: &str) -> InstalledSkill {
        InstalledSkill {
            version: "1.0.0".into(),
            hash: hash.into(),
            target: "global".into(),
        }
    }

    #[test]
    fn empty_manifest_empty_state_is_noop() {
        let plan = plan_sync(&SkillManifest::default(), &InstalledState::default());
        assert!(plan.is_noop());
    }

    #[test]
    fn new_skill_is_installed() {
        let manifest = SkillManifest {
            skills: vec![entry("a", "h1")],
        };
        let plan = plan_sync(&manifest, &InstalledState::default());
        assert_eq!(plan.to_install, vec!["a"]);
        assert!(plan.to_remove.is_empty());
    }

    #[test]
    fn unchanged_hash_is_skipped() {
        let manifest = SkillManifest {
            skills: vec![entry("a", "h1")],
        };
        let mut state = InstalledState::default();
        state.upsert("a", installed("h1"));
        assert!(plan_sync(&manifest, &state).is_noop());
    }

    #[test]
    fn changed_hash_is_reinstalled() {
        let manifest = SkillManifest {
            skills: vec![entry("a", "h2")],
        };
        let mut state = InstalledState::default();
        state.upsert("a", installed("h1"));
        assert_eq!(plan_sync(&manifest, &state).to_install, vec!["a"]);
    }

    #[test]
    fn missing_from_manifest_is_removed() {
        let manifest = SkillManifest {
            skills: vec![entry("a", "h1")],
        };
        let mut state = InstalledState::default();
        state.upsert("a", installed("h1"));
        state.upsert("gone", installed("hx"));
        let plan = plan_sync(&manifest, &state);
        assert!(plan.to_install.is_empty());
        assert_eq!(plan.to_remove, vec!["gone"]);
    }

    #[test]
    fn outputs_are_sorted() {
        let manifest = SkillManifest {
            skills: vec![entry("c", "n"), entry("a", "n"), entry("b", "n")],
        };
        let plan = plan_sync(&manifest, &InstalledState::default());
        assert_eq!(plan.to_install, vec!["a", "b", "c"]);
    }

    #[test]
    fn mixed_scenario() {
        // a: unchanged, b: changed, c: new, d: removed
        let manifest = SkillManifest {
            skills: vec![entry("a", "h"), entry("b", "h2"), entry("c", "h")],
        };
        let mut state = InstalledState::default();
        state.upsert("a", installed("h"));
        state.upsert("b", installed("h1"));
        state.upsert("d", installed("h"));
        let plan = plan_sync(&manifest, &state);
        assert_eq!(plan.to_install, vec!["b", "c"]);
        assert_eq!(plan.to_remove, vec!["d"]);
    }
}
