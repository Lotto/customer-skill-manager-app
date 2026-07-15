use crate::error::{CoreError, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Default sync interval when the user has not overridden it.
pub const DEFAULT_INTERVAL_MINUTES: u64 = 30;

/// A named destination directory that skills can be installed into.
///
/// The manifest refers to targets by name (e.g. `"global"` or `"acme-project"`);
/// the config maps each name to an absolute path on this machine.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TargetDir {
    pub name: String,
    pub path: PathBuf,
}

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
    /// Extra named target directories (beyond the implicit `global`).
    pub targets: Vec<TargetDir>,
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            backend_url: String::new(),
            license_key: String::new(),
            interval_minutes: DEFAULT_INTERVAL_MINUTES,
            log_level: "info".to_string(),
            auto_apply_updates: false,
            targets: Vec::new(),
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

    /// Resolve a target name to an absolute directory.
    ///
    /// The special name `"global"` resolves to `global_default` (typically
    /// `~/.claude/skills`) unless the user has explicitly overridden it in
    /// `targets`. Any other name must be declared in `targets`.
    pub fn resolve_target(&self, name: &str, global_default: &Path) -> Result<PathBuf> {
        if let Some(t) = self.targets.iter().find(|t| t.name == name) {
            return Ok(t.path.clone());
        }
        if name == "global" {
            return Ok(global_default.to_path_buf());
        }
        Err(CoreError::UnknownTarget(name.to_string()))
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
            license_key: "KEY".into(),
            ..Default::default()
        };
        assert!(!c.is_activated());
        c.backend_url = "https://x".into();
        assert!(c.is_activated());
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
            targets: vec![TargetDir {
                name: "acme".into(),
                path: PathBuf::from("/opt/acme/skills"),
            }],
            ..Default::default()
        };
        c.save(&p).unwrap();
        assert_eq!(AppConfig::load(&p).unwrap(), c);
    }

    #[test]
    fn global_resolves_to_default_when_not_overridden() {
        let c = AppConfig::default();
        let def = PathBuf::from("/home/u/.claude/skills");
        assert_eq!(c.resolve_target("global", &def).unwrap(), def);
    }

    #[test]
    fn global_can_be_overridden() {
        let mut c = AppConfig::default();
        c.targets.push(TargetDir {
            name: "global".into(),
            path: PathBuf::from("/custom/global"),
        });
        let def = PathBuf::from("/home/u/.claude/skills");
        assert_eq!(
            c.resolve_target("global", &def).unwrap(),
            PathBuf::from("/custom/global")
        );
    }

    #[test]
    fn unknown_target_errors() {
        let c = AppConfig::default();
        let def = PathBuf::from("/home/u/.claude/skills");
        assert!(matches!(
            c.resolve_target("missing", &def),
            Err(CoreError::UnknownTarget(_))
        ));
    }
}
