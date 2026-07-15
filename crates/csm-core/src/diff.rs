use crate::manifest::SkillManifest;
use crate::state::InstalledState;
use std::collections::HashSet;

/// What a sync should do, derived purely from (manifest, installed state).
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SyncPlan {
    /// Skills to (re)install — either new, or whose version changed.
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
/// A skill is (re)installed when it is absent locally or its manifest version
/// differs from the installed version — the CSM backend is version-driven, so
/// version is the change signal. A skill is removed when it exists in local
/// state but no longer appears in the manifest; because removal is keyed off
/// [`InstalledState`], the app never deletes files it did not install.
///
/// Both output lists are sorted for deterministic, log-friendly ordering.
pub fn plan_sync(manifest: &SkillManifest, state: &InstalledState) -> SyncPlan {
    let mut to_install: Vec<String> = manifest
        .skills
        .iter()
        .filter(|entry| match state.skills.get(&entry.slug) {
            Some(installed) => installed.version != entry.version,
            None => true,
        })
        .map(|entry| entry.slug.clone())
        .collect();

    let manifest_slugs: HashSet<&String> = manifest.skills.iter().map(|s| &s.slug).collect();
    let mut to_remove: Vec<String> = state
        .skills
        .keys()
        .filter(|slug| !manifest_slugs.contains(slug))
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

    fn entry(slug: &str, version: &str) -> SkillEntry {
        SkillEntry {
            slug: slug.into(),
            description: None,
            version: version.into(),
            target: "global".into(),
        }
    }

    fn installed(version: &str) -> InstalledSkill {
        InstalledSkill {
            version: version.into(),
            target: "global".into(),
            content_hash: "h".into(),
        }
    }

    #[test]
    fn empty_manifest_empty_state_is_noop() {
        assert!(plan_sync(&SkillManifest::default(), &InstalledState::default()).is_noop());
    }

    #[test]
    fn new_skill_is_installed() {
        let manifest = SkillManifest {
            skills: vec![entry("a", "1.0.0")],
        };
        let plan = plan_sync(&manifest, &InstalledState::default());
        assert_eq!(plan.to_install, vec!["a"]);
        assert!(plan.to_remove.is_empty());
    }

    #[test]
    fn same_version_is_skipped() {
        let manifest = SkillManifest {
            skills: vec![entry("a", "1.0.0")],
        };
        let mut state = InstalledState::default();
        state.upsert("a", installed("1.0.0"));
        assert!(plan_sync(&manifest, &state).is_noop());
    }

    #[test]
    fn changed_version_is_reinstalled() {
        let manifest = SkillManifest {
            skills: vec![entry("a", "2.0.0")],
        };
        let mut state = InstalledState::default();
        state.upsert("a", installed("1.0.0"));
        assert_eq!(plan_sync(&manifest, &state).to_install, vec!["a"]);
    }

    #[test]
    fn missing_from_manifest_is_removed() {
        let manifest = SkillManifest {
            skills: vec![entry("a", "1.0.0")],
        };
        let mut state = InstalledState::default();
        state.upsert("a", installed("1.0.0"));
        state.upsert("gone", installed("1.0.0"));
        let plan = plan_sync(&manifest, &state);
        assert!(plan.to_install.is_empty());
        assert_eq!(plan.to_remove, vec!["gone"]);
    }

    #[test]
    fn mixed_scenario() {
        // a: unchanged, b: bumped, c: new, d: removed
        let manifest = SkillManifest {
            skills: vec![entry("a", "1"), entry("b", "2"), entry("c", "1")],
        };
        let mut state = InstalledState::default();
        state.upsert("a", installed("1"));
        state.upsert("b", installed("1"));
        state.upsert("d", installed("1"));
        let plan = plan_sync(&manifest, &state);
        assert_eq!(plan.to_install, vec!["b", "c"]);
        assert_eq!(plan.to_remove, vec!["d"]);
    }
}
