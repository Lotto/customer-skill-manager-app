//! Claude Desktop integration.
//!
//! Claude Desktop surfaces skills from an app-managed plugin store under
//! `%APPDATA%\Claude\local-agent-mode-sessions\skills-plugin\<workspace>\<account>\`,
//! consisting of a `manifest.json` registry plus a `skills/<id>/` folder per
//! skill. This module mirrors the customer's entitled skills into that store,
//! silently, so they appear in Desktop's *Customize → Skills* panel.
//!
//! We only ever touch entries we own: our manifest entries carry
//! `source = "customer-skill-manager"`, and our skill folders carry the
//! [`crate::sync::MANAGED_MARKER`]. Anthropic-managed skills and any the user
//! added by hand are left untouched.

use crate::error::Result;
use crate::manifest::SkillManifest;
use crate::sync::{MANAGED_MARKER, SKILL_FILE};
use serde_json::{json, Value};
use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// Value written to `source` on the manifest entries we manage, so we can find
/// and clean up exactly our own skills.
pub const CSM_SOURCE: &str = "customer-skill-manager";

/// A skill to publish into a Desktop store.
#[derive(Debug, Clone)]
pub struct DesktopSkill {
    pub slug: String,
    pub name: String,
    pub description: String,
    pub md: String,
}

/// Aggregate result of a Desktop sync across all stores.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DesktopOutcome {
    pub stores: usize,
    pub installed: usize,
    pub removed: usize,
    pub errors: Vec<(String, String)>,
}

/// Discover Desktop skill stores: `.../skills-plugin/<workspace>/<account>/`
/// directories that contain a `manifest.json`.
///
/// `appdata_roaming` is the OS roaming app-data dir (Windows `%APPDATA%`).
pub fn discover_desktop_stores(appdata_roaming: &Path) -> Vec<PathBuf> {
    let base = appdata_roaming
        .join("Claude")
        .join("local-agent-mode-sessions")
        .join("skills-plugin");

    let mut stores = Vec::new();
    let Ok(workspaces) = std::fs::read_dir(&base) else {
        return stores;
    };
    for ws in workspaces.flatten() {
        if !ws.path().is_dir() {
            continue;
        }
        let Ok(accounts) = std::fs::read_dir(ws.path()) else {
            continue;
        };
        for acc in accounts.flatten() {
            let dir = acc.path();
            if dir.join("manifest.json").is_file() {
                stores.push(dir);
            }
        }
    }
    stores.sort();
    stores
}

/// Mirror the entitled skills (from `manifest`) into every Desktop `store`.
///
/// Skill bodies are read from `read_dir/<slug>/SKILL.md` (already materialized
/// by the normal sync). `now_ms`/`now_iso` timestamp the manifest.
pub fn sync_desktop(
    manifest: &SkillManifest,
    read_dir: &Path,
    stores: &[PathBuf],
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
    for store in stores {
        match apply_to_store(store, &desired, now_ms, now_iso) {
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

/// Apply the desired skill set to one store: write folders, merge the manifest,
/// and drop our own stale entries. Returns `(installed, removed)`.
fn apply_to_store(
    store: &Path,
    desired: &[DesktopSkill],
    now_ms: i64,
    now_iso: &str,
) -> Result<(usize, usize)> {
    let manifest_path = store.join("manifest.json");
    let mut root: Value = serde_json::from_str(&std::fs::read_to_string(&manifest_path)?)?;
    let skills_dir = store.join("skills");
    std::fs::create_dir_all(&skills_dir)?;

    let desired_slugs: HashSet<&str> = desired.iter().map(|d| d.slug.as_str()).collect();

    let mut entries: Vec<Value> = root
        .get("skills")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    // Drop our stale entries (ours = our source tag) and delete their folders.
    let mut removed = 0usize;
    entries.retain(|e| {
        if entry_source(e) != Some(CSM_SOURCE) {
            return true; // not ours — keep (Anthropic / user-added)
        }
        let slug = entry_slug(e).unwrap_or_default();
        if desired_slugs.contains(slug.as_str()) {
            return true; // still wanted; will be refreshed below
        }
        remove_managed_folder(&skills_dir, &slug);
        removed += 1;
        false
    });

    // Upsert every desired skill: folder + manifest entry.
    let mut installed = 0usize;
    for d in desired {
        write_skill_folder(&skills_dir, d)?;
        let new_entry = json!({
            "skillId": d.slug,
            "name": d.name,
            "description": d.description,
            "creatorType": "user",
            "enabled": true,
            "updatedAt": now_iso,
            "source": CSM_SOURCE,
        });
        match entries.iter().position(|e| {
            entry_source(e) == Some(CSM_SOURCE) && entry_slug(e).as_deref() == Some(&d.slug)
        }) {
            Some(pos) => entries[pos] = new_entry,
            None => entries.push(new_entry),
        }
        installed += 1;
    }

    root["skills"] = Value::Array(entries);
    root["lastUpdated"] = json!(now_ms);

    let text = serde_json::to_string_pretty(&root)?;
    let tmp = manifest_path.with_extension("json.tmp");
    std::fs::write(&tmp, text)?;
    std::fs::rename(&tmp, &manifest_path)?;
    Ok((installed, removed))
}

fn entry_source(e: &Value) -> Option<&str> {
    e.get("source").and_then(|v| v.as_str())
}
fn entry_slug(e: &Value) -> Option<String> {
    e.get("skillId").and_then(|v| v.as_str()).map(String::from)
}

/// Write `<skills_dir>/<slug>/SKILL.md` + managed marker, atomically.
fn write_skill_folder(skills_dir: &Path, d: &DesktopSkill) -> Result<()> {
    let final_dir = skills_dir.join(&d.slug);
    let staging = skills_dir.join(format!(".csm-staging-{}", d.slug));
    if staging.exists() {
        std::fs::remove_dir_all(&staging)?;
    }
    std::fs::create_dir_all(&staging)?;
    std::fs::write(staging.join(SKILL_FILE), &d.md)?;
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

/// Delete `<skills_dir>/<slug>/` only if it carries our managed marker.
fn remove_managed_folder(skills_dir: &Path, slug: &str) {
    let dir = skills_dir.join(slug);
    if dir.join(MANAGED_MARKER).is_file() {
        let _ = std::fs::remove_dir_all(&dir);
    }
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

    /// Create a fake Desktop store with a manifest holding one Anthropic skill,
    /// and a source dir holding materialized SKILL.md files.
    fn setup() -> (tempfile::TempDir, PathBuf, PathBuf) {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp.path().join("store");
        std::fs::create_dir_all(store.join("skills")).unwrap();
        let manifest = json!({
            "lastUpdated": 1,
            "skills": [
                { "skillId": "skill-creator", "name": "skill-creator",
                  "description": "anthropic one", "creatorType": "anthropic", "enabled": true }
            ]
        });
        std::fs::write(
            store.join("manifest.json"),
            serde_json::to_string_pretty(&manifest).unwrap(),
        )
        .unwrap();

        let read_dir = tmp.path().join("skills-src");
        for slug in ["bonjour", "bye"] {
            let d = read_dir.join(slug);
            std::fs::create_dir_all(&d).unwrap();
            std::fs::write(
                d.join(SKILL_FILE),
                format!("---\nname: {slug}\ndescription: \"d\"\n---\n\nbody {slug}\n"),
            )
            .unwrap();
        }
        (tmp, store, read_dir)
    }

    fn load_skills(store: &Path) -> Vec<Value> {
        let root: Value =
            serde_json::from_str(&std::fs::read_to_string(store.join("manifest.json")).unwrap())
                .unwrap();
        root["skills"].as_array().unwrap().clone()
    }

    #[test]
    fn discover_finds_stores_with_manifest() {
        let tmp = tempfile::tempdir().unwrap();
        let store = tmp
            .path()
            .join("Claude/local-agent-mode-sessions/skills-plugin/ws/acc");
        std::fs::create_dir_all(&store).unwrap();
        std::fs::write(store.join("manifest.json"), "{}").unwrap();
        // A sibling without a manifest is ignored.
        std::fs::create_dir_all(
            tmp.path()
                .join("Claude/local-agent-mode-sessions/skills-plugin/ws/empty"),
        )
        .unwrap();

        let found = discover_desktop_stores(tmp.path());
        assert_eq!(found, vec![store]);
    }

    #[test]
    fn injects_skills_and_preserves_anthropic() {
        let (_tmp, store, read_dir) = setup();
        let manifest = SkillManifest {
            skills: vec![entry("bonjour", "répond bonjour"), entry("bye", "répond bye")],
        };
        let out = sync_desktop(&manifest, &read_dir, std::slice::from_ref(&store), 1000, "2026-07-16T00:00:00Z");
        assert_eq!(out.stores, 1);
        assert_eq!(out.installed, 2);
        assert!(out.errors.is_empty());

        // Folders written with marker.
        assert!(store.join("skills/bonjour/SKILL.md").is_file());
        assert!(store.join("skills/bonjour").join(MANAGED_MARKER).is_file());

        // Manifest: anthropic entry preserved + our two added with our source.
        let skills = load_skills(&store);
        assert_eq!(skills.len(), 3);
        // The Anthropic entry (creatorType "anthropic", no `source`) is preserved.
        assert!(skills.iter().any(|e| {
            entry_slug(e).as_deref() == Some("skill-creator") && entry_source(e).is_none()
        }));
        let ours: Vec<_> = skills
            .iter()
            .filter(|e| entry_source(e) == Some(CSM_SOURCE))
            .collect();
        assert_eq!(ours.len(), 2);
        assert_eq!(ours[0]["creatorType"], "user");
    }

    #[test]
    fn second_sync_is_idempotent_no_duplicates() {
        let (_tmp, store, read_dir) = setup();
        let manifest = SkillManifest {
            skills: vec![entry("bonjour", "x")],
        };
        sync_desktop(&manifest, &read_dir, std::slice::from_ref(&store), 1, "t");
        sync_desktop(&manifest, &read_dir, std::slice::from_ref(&store), 2, "t");
        let ours = load_skills(&store)
            .into_iter()
            .filter(|e| entry_source(e) == Some(CSM_SOURCE))
            .count();
        assert_eq!(ours, 1);
    }

    #[test]
    fn dropped_skill_is_removed_but_others_kept() {
        let (_tmp, store, read_dir) = setup();
        let full = SkillManifest {
            skills: vec![entry("bonjour", "x"), entry("bye", "y")],
        };
        sync_desktop(&full, &read_dir, std::slice::from_ref(&store), 1, "t");
        assert!(store.join("skills/bye").exists());

        // Now only bonjour is entitled.
        let reduced = SkillManifest {
            skills: vec![entry("bonjour", "x")],
        };
        let out = sync_desktop(&reduced, &read_dir, std::slice::from_ref(&store), 2, "t");
        assert_eq!(out.removed, 1);
        assert!(!store.join("skills/bye").exists());
        assert!(store.join("skills/bonjour").exists());

        let skills = load_skills(&store);
        // anthropic + bonjour only.
        assert_eq!(skills.len(), 2);
        assert!(skills
            .iter()
            .any(|e| entry_slug(e).as_deref() == Some("skill-creator")));
    }

    #[test]
    fn never_removes_anthropic_or_handmade_user_entries() {
        let (_tmp, store, read_dir) = setup();
        // Add a hand-made user skill (no CSM source) to the manifest + a folder
        // WITHOUT our marker.
        let mut root: Value =
            serde_json::from_str(&std::fs::read_to_string(store.join("manifest.json")).unwrap())
                .unwrap();
        root["skills"].as_array_mut().unwrap().push(json!({
            "skillId": "handmade", "name": "handmade", "description": "mine",
            "creatorType": "user", "enabled": true
        }));
        std::fs::write(
            store.join("manifest.json"),
            serde_json::to_string_pretty(&root).unwrap(),
        )
        .unwrap();
        let hand = store.join("skills/handmade");
        std::fs::create_dir_all(&hand).unwrap();
        std::fs::write(hand.join("keep.txt"), b"x").unwrap();

        // Sync an empty entitlement: our (zero) skills change, others untouched.
        let out = sync_desktop(&SkillManifest::default(), &read_dir, std::slice::from_ref(&store), 1, "t");
        assert_eq!(out.removed, 0);
        assert!(hand.join("keep.txt").is_file());
        let skills = load_skills(&store);
        assert!(skills
            .iter()
            .any(|e| entry_slug(e).as_deref() == Some("handmade")));
        assert!(skills
            .iter()
            .any(|e| entry_slug(e).as_deref() == Some("skill-creator")));
    }
}
