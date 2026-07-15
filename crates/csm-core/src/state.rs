use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use std::path::Path;

/// A skill the app has installed on this machine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct InstalledSkill {
    /// Version last installed; compared against the manifest to detect updates.
    pub version: String,
    /// Target directory name this skill was installed into.
    pub target: String,
    /// SHA-256 of the materialized `SKILL.md`, used as an idempotency guard so
    /// an unchanged skill is never needlessly rewritten.
    pub content_hash: String,
}

/// The persistent record of everything CSM manages locally.
///
/// This is the authority for what CSM may remove: the sync engine only ever
/// deletes skills present in this state (and carrying the on-disk managed
/// marker), never arbitrary files the user placed in the target directories.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct InstalledState {
    /// skill slug -> installed record (BTreeMap for deterministic output).
    pub skills: BTreeMap<String, InstalledSkill>,
}

impl InstalledState {
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        Ok(serde_json::from_str(&text)?)
    }

    /// Persist atomically (temp file + rename).
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = serde_json::to_string_pretty(self)?;
        let tmp = path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    pub fn upsert(&mut self, slug: impl Into<String>, skill: InstalledSkill) {
        self.skills.insert(slug.into(), skill);
    }

    pub fn remove(&mut self, slug: &str) -> Option<InstalledSkill> {
        self.skills.remove(slug)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> InstalledSkill {
        InstalledSkill {
            version: "1.0.0".into(),
            target: "global".into(),
            content_hash: "abc".into(),
        }
    }

    #[test]
    fn load_missing_is_default() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("state.json");
        assert_eq!(InstalledState::load(&p).unwrap(), InstalledState::default());
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("sub").join("state.json");
        let mut s = InstalledState::default();
        s.upsert("bonjour", sample());
        s.save(&p).unwrap();
        assert_eq!(InstalledState::load(&p).unwrap(), s);
    }

    #[test]
    fn remove_returns_previous() {
        let mut s = InstalledState::default();
        s.upsert("x", sample());
        assert!(s.remove("x").is_some());
        assert!(s.remove("x").is_none());
    }
}
