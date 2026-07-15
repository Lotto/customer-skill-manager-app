//! One-time removal of the legacy CSM delivery mechanism.
//!
//! Earlier, business skills were delivered through the `csm-loader` plugin from
//! the `customer-skill-manager` marketplace (a remote git marketplace). The app
//! now syncs skills directly, so on startup it uninstalls that legacy plugin:
//! its cached files, its marketplace clone, and its two registry entries.

use crate::error::Result;
use serde_json::Value;
use std::path::Path;

/// Name of the deprecated marketplace (dir + `known_marketplaces.json` key).
pub const LEGACY_MARKETPLACE: &str = "customer-skill-manager";
/// Key of the deprecated plugin in `installed_plugins.json`.
pub const LEGACY_PLUGIN_KEY: &str = "csm-loader@customer-skill-manager";

/// What the cleanup removed (for logging).
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct CleanupOutcome {
    pub removed_dirs: Vec<String>,
    pub removed_marketplace_entry: bool,
    pub removed_plugin_entry: bool,
}

impl CleanupOutcome {
    pub fn did_anything(&self) -> bool {
        !self.removed_dirs.is_empty()
            || self.removed_marketplace_entry
            || self.removed_plugin_entry
    }
}

/// Remove the legacy `csm-loader` plugin from a Claude config dir (`~/.claude`).
///
/// Idempotent: absent pieces are skipped. Other marketplaces/plugins are left
/// untouched.
pub fn remove_legacy_plugin(claude_dir: &Path) -> Result<CleanupOutcome> {
    let plugins = claude_dir.join("plugins");
    let mut out = CleanupOutcome::default();

    for sub in ["cache", "marketplaces"] {
        let dir = plugins.join(sub).join(LEGACY_MARKETPLACE);
        if dir.exists() {
            std::fs::remove_dir_all(&dir)?;
            out.removed_dirs.push(dir.display().to_string());
        }
    }

    let km = plugins.join("known_marketplaces.json");
    if km.is_file() {
        if let Ok(mut v) = load_json(&km) {
            if let Some(obj) = v.as_object_mut() {
                if obj.remove(LEGACY_MARKETPLACE).is_some() {
                    save_json(&km, &v)?;
                    out.removed_marketplace_entry = true;
                }
            }
        }
    }

    let ip = plugins.join("installed_plugins.json");
    if ip.is_file() {
        if let Ok(mut v) = load_json(&ip) {
            if let Some(plugins_obj) = v.get_mut("plugins").and_then(|p| p.as_object_mut()) {
                if plugins_obj.remove(LEGACY_PLUGIN_KEY).is_some() {
                    save_json(&ip, &v)?;
                    out.removed_plugin_entry = true;
                }
            }
        }
    }

    Ok(out)
}

fn load_json(path: &Path) -> Result<Value> {
    Ok(serde_json::from_str(&std::fs::read_to_string(path)?)?)
}

fn save_json(path: &Path, v: &Value) -> Result<()> {
    let text = serde_json::to_string_pretty(v)?;
    let tmp = path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn setup() -> (tempfile::TempDir, std::path::PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let claude = tmp.path().join(".claude");
        let plugins = claude.join("plugins");
        // Legacy dirs.
        for sub in ["cache", "marketplaces"] {
            std::fs::create_dir_all(plugins.join(sub).join(LEGACY_MARKETPLACE)).unwrap();
            // A sibling that must be preserved.
            std::fs::create_dir_all(plugins.join(sub).join("keep-me")).unwrap();
        }
        // Registry files with the legacy entries plus others.
        std::fs::write(
            plugins.join("known_marketplaces.json"),
            serde_json::to_string_pretty(&json!({
                "claude-plugins-official": { "source": {} },
                LEGACY_MARKETPLACE: { "source": {} }
            }))
            .unwrap(),
        )
        .unwrap();
        std::fs::write(
            plugins.join("installed_plugins.json"),
            serde_json::to_string_pretty(&json!({
                "version": 2,
                "plugins": {
                    "figma@claude-plugins-official": [],
                    LEGACY_PLUGIN_KEY: []
                }
            }))
            .unwrap(),
        )
        .unwrap();
        (tmp, claude)
    }

    #[test]
    fn removes_legacy_and_preserves_the_rest() {
        let (_tmp, claude) = setup();
        let out = remove_legacy_plugin(&claude).unwrap();

        assert_eq!(out.removed_dirs.len(), 2);
        assert!(out.removed_marketplace_entry);
        assert!(out.removed_plugin_entry);
        assert!(out.did_anything());

        let plugins = claude.join("plugins");
        assert!(!plugins.join("cache").join(LEGACY_MARKETPLACE).exists());
        assert!(!plugins.join("marketplaces").join(LEGACY_MARKETPLACE).exists());
        assert!(plugins.join("cache/keep-me").exists());
        assert!(plugins.join("marketplaces/keep-me").exists());

        let km: Value =
            serde_json::from_str(&std::fs::read_to_string(plugins.join("known_marketplaces.json")).unwrap())
                .unwrap();
        assert!(km.get(LEGACY_MARKETPLACE).is_none());
        assert!(km.get("claude-plugins-official").is_some());

        let ip: Value =
            serde_json::from_str(&std::fs::read_to_string(plugins.join("installed_plugins.json")).unwrap())
                .unwrap();
        assert!(ip["plugins"].get(LEGACY_PLUGIN_KEY).is_none());
        assert!(ip["plugins"].get("figma@claude-plugins-official").is_some());
    }

    #[test]
    fn idempotent_second_run_is_noop() {
        let (_tmp, claude) = setup();
        remove_legacy_plugin(&claude).unwrap();
        let out = remove_legacy_plugin(&claude).unwrap();
        assert!(!out.did_anything());
    }

    #[test]
    fn missing_config_is_ok() {
        let tmp = tempfile::tempdir().unwrap();
        let out = remove_legacy_plugin(&tmp.path().join(".claude")).unwrap();
        assert!(!out.did_anything());
    }
}
