//! Claude Desktop integration via its local plugin store (`rpm/`).
//!
//! Uploading a plugin ZIP in Desktop extracts it to
//! `…/local-agent-mode-sessions/<account>/<workspace>/rpm/<pluginId>/`
//! (`.claude-plugin/plugin.json` + `skills/<slug>/SKILL.md`) and registers it in
//! `rpm/manifest.json`. This module reproduces that exact result so entitled
//! skills show up in Desktop's *Plugins* panel without a manual upload.
//!
//! We own a single plugin named [`PLUGIN_NAME`]; every skill folder inside it is
//! ours, so removals are scoped to that plugin and never touch other plugins.

use crate::error::Result;
use crate::manifest::SkillManifest;
use crate::sync::SKILL_FILE;
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The plugin the app manages in Desktop's store.
pub const PLUGIN_NAME: &str = "customer-skills";
/// The deprecated plugin to uninstall from Desktop's store.
const LEGACY_PLUGIN_NAME: &str = "csm-loader";
/// Stable plugin id used when the plugin isn't already registered.
const PLUGIN_ID_FALLBACK: &str = "plugin_csm_customer_skills";
/// Marketplace the plugin is filed under (same bucket Desktop uses for uploads).
const MARKETPLACE_NAME: &str = "My Uploads";
const MARKETPLACE_ID_FALLBACK: &str = "marketplace_csm_uploads";

/// A skill to publish into the Desktop plugin.
#[derive(Debug, Clone)]
pub struct DesktopSkill {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub md: String,
}

/// Aggregate result of a Desktop plugin sync across all stores.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesktopOutcome {
    pub stores: usize,
    pub installed: usize,
    pub removed: usize,
    /// Number of stores the legacy `csm-loader` plugin was removed from.
    pub legacy_removed: usize,
    pub errors: Vec<(String, String)>,
}

/// Discover Desktop plugin stores: `…/local-agent-mode-sessions/<account>/<workspace>/rpm/`
/// directories that contain a `manifest.json`.
pub fn discover_rpm_stores(appdata_roaming: &Path) -> Vec<PathBuf> {
    let base = appdata_roaming
        .join("Claude")
        .join("local-agent-mode-sessions");

    let mut stores = Vec::new();
    let Ok(accounts) = std::fs::read_dir(&base) else {
        return stores;
    };
    for account in accounts.flatten() {
        let apath = account.path();
        // `skills-plugin` is a different subsystem; skip it.
        if !apath.is_dir() || account.file_name() == "skills-plugin" {
            continue;
        }
        let Ok(workspaces) = std::fs::read_dir(&apath) else {
            continue;
        };
        for ws in workspaces.flatten() {
            let rpm = ws.path().join("rpm");
            if rpm.join("manifest.json").is_file() {
                stores.push(rpm);
            }
        }
    }
    stores.sort();
    stores
}

/// Mirror the entitled skills (from `manifest`) into the Desktop plugin in every
/// discovered `rpm_stores`. Skill bodies are read from `read_dir/<slug>/SKILL.md`.
pub fn sync_desktop(
    manifest: &SkillManifest,
    read_dir: &Path,
    rpm_stores: &[PathBuf],
    now_ms: i64,
    now_iso: &str,
) -> DesktopOutcome {
    let mut desired = Vec::new();
    for entry in &manifest.skills {
        let md_path = read_dir.join(&entry.slug).join(SKILL_FILE);
        if let Ok(md) = std::fs::read_to_string(&md_path) {
            desired.push(DesktopSkill {
                slug: entry.slug.clone(),
                name: entry.slug.clone(),
                description: entry.display_description().to_string(),
                md,
            });
        }
    }

    let mut outcome = DesktopOutcome::default();
    for store in rpm_stores {
        // Uninstall the deprecated csm-loader plugin, best-effort.
        if remove_legacy_plugin(store).unwrap_or(false) {
            outcome.legacy_removed += 1;
        }
        match apply_plugin(store, &desired, now_ms, now_iso) {
            Ok((installed, removed)) => {
                outcome.stores += 1;
                outcome.installed += installed;
                outcome.removed += removed;
            }
            Err(e) => outcome
                .errors
                .push((store.display().to_string(), e.to_string())),
        }
    }
    outcome
}

/// Remove the deprecated `csm-loader` plugin (entry + folder) from an rpm store.
/// Returns whether anything was removed.
fn remove_legacy_plugin(rpm_dir: &Path) -> Result<bool> {
    let manifest_path = rpm_dir.join("manifest.json");
    let mut root: Value = serde_json::from_str(&std::fs::read_to_string(&manifest_path)?)?;
    let Some(plugins) = root.get_mut("plugins").and_then(|v| v.as_array_mut()) else {
        return Ok(false);
    };

    let mut removed = false;
    let mut i = 0;
    while i < plugins.len() {
        if plugins[i].get("name").and_then(|v| v.as_str()) == Some(LEGACY_PLUGIN_NAME) {
            if let Some(id) = plugins[i].get("id").and_then(|v| v.as_str()) {
                let _ = std::fs::remove_dir_all(rpm_dir.join(id));
            }
            plugins.remove(i);
            removed = true;
        } else {
            i += 1;
        }
    }

    if removed {
        let text = serde_json::to_string_pretty(&root)?;
        let tmp = manifest_path.with_extension("json.tmp");
        std::fs::write(&tmp, text)?;
        std::fs::rename(&tmp, &manifest_path)?;
    }
    Ok(removed)
}

/// Write our plugin folder and upsert its `rpm/manifest.json` entry.
fn apply_plugin(
    rpm_dir: &Path,
    desired: &[DesktopSkill],
    now_ms: i64,
    now_iso: &str,
) -> Result<(usize, usize)> {
    let manifest_path = rpm_dir.join("manifest.json");
    let mut root: Value = serde_json::from_str(&std::fs::read_to_string(&manifest_path)?)?;

    let plugins: Vec<Value> = root
        .get("plugins")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Reuse the ids of an already-registered "customer-skills" plugin (e.g. from
    // a prior upload) so we update it in place rather than duplicating.
    let existing = plugins
        .iter()
        .find(|p| p.get("name").and_then(|v| v.as_str()) == Some(PLUGIN_NAME));
    let plugin_id = existing
        .and_then(|p| p.get("id").and_then(|v| v.as_str()))
        .unwrap_or(PLUGIN_ID_FALLBACK)
        .to_string();
    let marketplace_id = existing
        .and_then(|p| p.get("marketplaceId").and_then(|v| v.as_str()))
        .or_else(|| {
            // otherwise borrow whatever id Desktop uses for the uploads bucket.
            plugins
                .iter()
                .find(|p| {
                    p.get("marketplaceName").and_then(|v| v.as_str()) == Some(MARKETPLACE_NAME)
                })
                .and_then(|p| p.get("marketplaceId").and_then(|v| v.as_str()))
        })
        .unwrap_or(MARKETPLACE_ID_FALLBACK)
        .to_string();

    // --- Write the plugin folder: .claude-plugin/plugin.json + skills/<slug>/ ---
    let plugin_dir = rpm_dir.join(&plugin_id);
    let cp_dir = plugin_dir.join(".claude-plugin");
    std::fs::create_dir_all(&cp_dir)?;
    std::fs::write(
        cp_dir.join("plugin.json"),
        serde_json::to_string_pretty(&json!({
            "name": PLUGIN_NAME,
            "version": "1.0.0",
            "description": "Skills métier synchronisés sous licence par Customer Skill Manager.",
            "author": { "name": "Customer Skill Manager" },
        }))?,
    )?;

    let skills_dir = plugin_dir.join("skills");
    std::fs::create_dir_all(&skills_dir)?;

    let desired_slugs: HashSet<&str> = desired.iter().map(|d| d.slug.as_str()).collect();

    let mut installed = 0usize;
    for d in desired {
        write_skill(&skills_dir, d)?;
        installed += 1;
    }

    // Remove skill folders no longer entitled. The whole plugin is ours, so any
    // sub-folder not in `desired` is stale.
    let mut removed = 0usize;
    if let Ok(entries) = std::fs::read_dir(&skills_dir) {
        for e in entries.flatten() {
            if !e.path().is_dir() {
                continue;
            }
            let slug = e.file_name().to_string_lossy().to_string();
            if !desired_slugs.contains(slug.as_str()) {
                let _ = std::fs::remove_dir_all(e.path());
                removed += 1;
            }
        }
    }

    // --- Upsert the manifest entry ---
    let entry = json!({
        "id": plugin_id,
        "name": PLUGIN_NAME,
        "updatedAt": now_iso,
        "marketplaceId": marketplace_id,
        "marketplaceName": MARKETPLACE_NAME,
        "installedBy": "user",
    });
    let mut plugins = plugins;
    match plugins
        .iter()
        .position(|p| p.get("name").and_then(|v| v.as_str()) == Some(PLUGIN_NAME))
    {
        Some(i) => plugins[i] = entry,
        None => plugins.push(entry),
    }
    root["plugins"] = Value::Array(plugins);
    root["lastUpdated"] = json!(now_ms);

    let text = serde_json::to_string_pretty(&root)?;
    let tmp = manifest_path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &manifest_path)?;
    Ok((installed, removed))
}

/// Write `<skills_dir>/<slug>/SKILL.md` atomically (staging dir + swap).
fn write_skill(skills_dir: &Path, d: &DesktopSkill) -> Result<()> {
    let final_dir = skills_dir.join(&d.slug);
    let staging = skills_dir.join(format!(".csm-staging-{}", d.slug));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;
    std::fs::write(staging.join(SKILL_FILE), &d.md)?;
    if final_dir.exists() {
        std::fs::remove_dir_all(&final_dir)?;
    }
    std::fs::rename(&staging, &final_dir)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manifest::SkillEntry;

    fn entry(slug: &str, desc: &str) -> SkillEntry {
        SkillEntry {
            slug: slug.into(),
            description: Some(desc.into()),
            version: "1.0.0".into(),
            target: "global".into(),
        }
    }

    /// Fake rpm store with a manifest holding one unrelated plugin, plus a source
    /// dir with materialized SKILL.md files.
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let rpm = tmp.path().join("rpm");
        std::fs::create_dir_all(&rpm).unwrap();
        std::fs::write(
            rpm.join("manifest.json"),
            serde_json::to_string_pretty(&json!({
                "lastUpdated": 1,
                "plugins": [
                    { "id": "plugin_other", "name": "data", "marketplaceName": "knowledge-work-plugins",
                      "marketplaceId": "marketplace_kw", "installedBy": "user" }
                ]
            }))
            .unwrap(),
        )
        .unwrap();

        let src = tmp.path().join("src");
        for slug in ["bonjour", "bye"] {
            let d = src.join(slug);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(d.join(SKILL_FILE), format!("---\nname: {slug}\n---\nbody")).unwrap();
        }
        (tmp, rpm, src)
    }

    fn plugins(rpm: &Path) -> Vec<Value> {
        let root: Value =
            serde_json::from_str(&std::fs::read_to_string(rpm.join("manifest.json")).unwrap())
                .unwrap();
        root["plugins"].as_array().unwrap().clone()
    }

    #[test]
    fn discovers_rpm_stores() {
        let tmp = tempfile::tempdir().unwrap();
        let rpm = tmp
            .path()
            .join("Claude/local-agent-mode-sessions/acc/ws/rpm");
        std::fs::create_dir_all(&rpm).unwrap();
        std::fs::write(rpm.join("manifest.json"), "{}").unwrap();
        // skills-plugin is skipped.
        std::fs::create_dir_all(
            tmp.path()
                .join("Claude/local-agent-mode-sessions/skills-plugin/a/b"),
        )
        .unwrap();
        assert_eq!(discover_rpm_stores(tmp.path()), vec![rpm]);
    }

    #[test]
    fn creates_plugin_folder_and_registers_it() {
        let (_t, rpm, src) = setup();
        let m = SkillManifest {
            skills: vec![
                entry("bonjour", "répond bonjour"),
                entry("bye", "répond bye"),
            ],
        };
        let out = sync_desktop(
            &m,
            &src,
            std::slice::from_ref(&rpm),
            100,
            "2026-07-16T00:00:00Z",
        );
        assert_eq!(out.stores, 1);
        assert_eq!(out.installed, 2);

        let pdir = rpm.join(PLUGIN_ID_FALLBACK);
        assert!(pdir.join(".claude-plugin/plugin.json").is_file());
        assert!(pdir.join("skills/bonjour/SKILL.md").is_file());
        assert!(pdir.join("skills/bye/SKILL.md").is_file());

        let ps = plugins(&rpm);
        assert_eq!(ps.len(), 2); // the pre-existing "data" + ours
        let ours = ps.iter().find(|p| p["name"] == "customer-skills").unwrap();
        assert_eq!(ours["id"], PLUGIN_ID_FALLBACK);
        assert_eq!(ours["marketplaceName"], "My Uploads");
        assert_eq!(ours["installedBy"], "user");
    }

    #[test]
    fn reuses_existing_upload_id() {
        let (_t, rpm, src) = setup();
        // Pretend the user already uploaded: an entry with a server id.
        let mut root: Value =
            serde_json::from_str(&std::fs::read_to_string(rpm.join("manifest.json")).unwrap())
                .unwrap();
        root["plugins"].as_array_mut().unwrap().push(json!({
            "id": "plugin_SERVER123", "name": "customer-skills",
            "marketplaceId": "marketplace_uploads", "marketplaceName": "My Uploads",
            "installedBy": "user"
        }));
        std::fs::write(
            rpm.join("manifest.json"),
            serde_json::to_string_pretty(&root).unwrap(),
        )
        .unwrap();

        let m = SkillManifest {
            skills: vec![entry("bonjour", "x")],
        };
        sync_desktop(&m, &src, std::slice::from_ref(&rpm), 1, "t");
        // Updated in place: still one "customer-skills" entry, keeping the server id.
        let ps = plugins(&rpm);
        let ours: Vec<_> = ps
            .iter()
            .filter(|p| p["name"] == "customer-skills")
            .collect();
        assert_eq!(ours.len(), 1);
        assert_eq!(ours[0]["id"], "plugin_SERVER123");
        assert!(rpm
            .join("plugin_SERVER123/skills/bonjour/SKILL.md")
            .is_file());
    }

    #[test]
    fn removes_legacy_csm_loader() {
        let (_t, rpm, src) = setup();
        // Add a legacy csm-loader plugin (entry + folder).
        let mut root: Value =
            serde_json::from_str(&std::fs::read_to_string(rpm.join("manifest.json")).unwrap())
                .unwrap();
        root["plugins"].as_array_mut().unwrap().push(json!({
            "id": "plugin_legacy", "name": "csm-loader",
            "marketplaceName": "customer-skill-manager-marketplace", "installedBy": "user"
        }));
        std::fs::write(
            rpm.join("manifest.json"),
            serde_json::to_string_pretty(&root).unwrap(),
        )
        .unwrap();
        std::fs::create_dir_all(rpm.join("plugin_legacy/skills/csm-loader")).unwrap();

        let m = SkillManifest {
            skills: vec![entry("bonjour", "x")],
        };
        let out = sync_desktop(&m, &src, std::slice::from_ref(&rpm), 1, "t");
        assert_eq!(out.legacy_removed, 1);
        assert!(!rpm.join("plugin_legacy").exists());
        assert!(!plugins(&rpm).iter().any(|p| p["name"] == "csm-loader"));
        // Our plugin + the unrelated "data" plugin remain.
        assert!(plugins(&rpm).iter().any(|p| p["name"] == "customer-skills"));
        assert!(plugins(&rpm).iter().any(|p| p["name"] == "data"));
    }

    #[test]
    fn removes_dropped_skills_keeps_other_plugins() {
        let (_t, rpm, src) = setup();
        let full = SkillManifest {
            skills: vec![entry("bonjour", "x"), entry("bye", "y")],
        };
        sync_desktop(&full, &src, std::slice::from_ref(&rpm), 1, "t");
        let pdir = rpm.join(PLUGIN_ID_FALLBACK);
        assert!(pdir.join("skills/bye").exists());

        let reduced = SkillManifest {
            skills: vec![entry("bonjour", "x")],
        };
        let out = sync_desktop(&reduced, &src, std::slice::from_ref(&rpm), 2, "t");
        assert_eq!(out.removed, 1);
        assert!(!pdir.join("skills/bye").exists());
        assert!(pdir.join("skills/bonjour").exists());
        // The unrelated "data" plugin is untouched.
        assert!(plugins(&rpm).iter().any(|p| p["name"] == "data"));
    }
}
