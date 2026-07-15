use crate::error::Result;
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Default sync interval when the user has not overridden it.
pub const DEFAULT_INTERVAL_MINUTES: u64 = 30;

/// Default backend endpoint, pre-filled on a fresh install so the customer only
/// has to enter their license key.
pub const DEFAULT_BACKEND_URL: &str =
    "https://hikyqslxoakwubxzdejd.supabase.co/functions/v1/skill-resource";

/// The on-disk application configuration (TOML in the app-data directory).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(default)]
pub struct AppConfig {
    /// Base URL of the backend, e.g. `https://backend.example.com`.
    pub backend_url: String,
    /// License key sent as a bearer credential to the backend.
    pub license_key: String,
    /// How often to run a sync, in minutes.
    pub interval_minutes: u64,
    /// `tracing` log level: error | warn | info | debug | trace.
    pub log_level: String,
    /// Whether to apply downloaded app updates automatically at next restart.
    pub auto_apply_updates: bool,
    /// Directories skills are installed into. Every entitled skill is written
    /// to every directory in this list. When empty, the global default
    /// (`~/.claude/skills`) is used.
    pub skill_dirs: Vec<PathBuf>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            backend_url: DEFAULT_BACKEND_URL.to_string(),
            license_key: String::new(),
            interval_minutes: DEFAULT_INTERVAL_MINUTES,
            log_level: "info".to_string(),
            auto_apply_updates: false,
            skill_dirs: Vec::new(),
        }
    }
}

impl AppConfig {
    /// Load config from `path`. A missing file yields [`AppConfig::default`],
    /// so a fresh install starts cleanly rather than erroring.
    pub fn load(path: &Path) -> Result<Self> {
        if !path.exists() {
            return Ok(Self::default());
        }
        let text = std::fs::read_to_string(path)?;
        let cfg: AppConfig = toml::from_str(&text)?;
        Ok(cfg)
    }

    /// Persist config to `path`, creating parent directories as needed.
    /// The write is atomic (temp file + rename) so a crash cannot leave a
    /// half-written config behind.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let text = toml::to_string_pretty(self)?;
        let tmp = path.with_extension("toml.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, path)?;
        Ok(())
    }

    /// The sync interval as a [`Duration`], with a 1-minute floor to avoid a
    /// hot loop from a misconfigured `0`.
    pub fn interval(&self) -> Duration {
        Duration::from_secs(self.interval_minutes.max(1) * 60)
    }

    /// The app is "activated" once it has both a backend URL and a license key.
    pub fn is_activated(&self) -> bool {
        !self.license_key.trim().is_empty() && !self.backend_url.trim().is_empty()
    }

    /// The directories skills should be installed into.
    ///
    /// Returns the configured [`AppConfig::skill_dirs`] (de-duplicated, order
    /// preserved), or `[global_default]` when none are configured.
    pub fn effective_skill_dirs(&self, global_default: &Path) -> Vec<PathBuf> {
        if self.skill_dirs.is_empty() {
            return vec![global_default.to_path_buf()];
        }
        let mut seen = HashSet::new();
        self.skill_dirs
            .iter()
            .filter(|d| seen.insert((*d).clone()))
            .cloned()
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_sane() {
        let c = AppConfig::default();
        assert_eq!(c.interval_minutes, DEFAULT_INTERVAL_MINUTES);
        assert_eq!(c.log_level, "info");
        assert!(!c.is_activated());
        assert_eq!(c.interval(), Duration::from_secs(30 * 60));
    }

    #[test]
    fn interval_has_floor() {
        let c = AppConfig {
            interval_minutes: 0,
            ..Default::default()
        };
        assert_eq!(c.interval(), Duration::from_secs(60));
    }

    #[test]
    fn activation_requires_both_fields() {
        let mut c = AppConfig {
            backend_url: String::new(),
            license_key: String::new(),
            ..Default::default()
        };
        assert!(!c.is_activated());
        c.license_key = "KEY".into();
        assert!(!c.is_activated()); // backend still empty
        c.backend_url = "https://x".into();
        assert!(c.is_activated());
    }

    #[test]
    fn default_prefills_backend_url() {
        assert_eq!(AppConfig::default().backend_url, DEFAULT_BACKEND_URL);
        // Pre-filled URL alone is not activation; a license is still required.
        assert!(!AppConfig::default().is_activated());
    }

    #[test]
    fn load_missing_file_is_default() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("does-not-exist.toml");
        assert_eq!(AppConfig::load(&p).unwrap(), AppConfig::default());
    }

    #[test]
    fn save_then_load_roundtrips() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("nested").join("config.toml");
        let c = AppConfig {
            backend_url: "https://backend.example.com".into(),
            license_key: "abc-123".into(),
            interval_minutes: 15,
            skill_dirs: vec![PathBuf::from("/opt/acme/skills")],
            ..Default::default()
        };
        c.save(&p).unwrap();
        assert_eq!(AppConfig::load(&p).unwrap(), c);
    }

    #[test]
    fn effective_dirs_default_to_global() {
        let c = AppConfig::default();
        let def = PathBuf::from("/home/u/.claude/skills");
        assert_eq!(c.effective_skill_dirs(&def), vec![def]);
    }

    #[test]
    fn effective_dirs_use_configured_and_dedup() {
        let c = AppConfig {
            skill_dirs: vec![
                PathBuf::from("/a"),
                PathBuf::from("/b"),
                PathBuf::from("/a"),
            ],
            ..Default::default()
        };
        let def = PathBuf::from("/home/u/.claude/skills");
        assert_eq!(
            c.effective_skill_dirs(&def),
            vec![PathBuf::from("/a"), PathBuf::from("/b")]
        );
    }
}
