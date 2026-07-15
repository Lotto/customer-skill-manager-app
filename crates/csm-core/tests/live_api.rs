//! Live integration test against the real CSM backend.
//!
//! Ignored by default (needs network + a real license key), so it never runs
//! in CI. Run manually with:
//!
//! ```text
//! CSM_ENDPOINT=... CSM_LICENSE_KEY=... \
//!   cargo test -p csm-core --features net --test live_api -- --ignored --nocapture
//! ```
#![cfg(feature = "net")]

use csm_core::config::AppConfig;
use csm_core::http::HttpSkillSource;
use csm_core::sync::{run_sync, SkillSource};
use std::time::Duration;

fn source() -> HttpSkillSource {
    let endpoint = std::env::var("CSM_ENDPOINT").expect("set CSM_ENDPOINT");
    let key = std::env::var("CSM_LICENSE_KEY").expect("set CSM_LICENSE_KEY");
    HttpSkillSource::new(endpoint, key, Duration::from_secs(15)).unwrap()
}

#[test]
#[ignore = "requires network and a real license key"]
fn live_list_and_instructions() {
    let src = source();

    let manifest = src.fetch_manifest().expect("fetch manifest");
    println!("Fetched {} skills:", manifest.skills.len());
    for s in &manifest.skills {
        println!(
            "  - {} @ {} ({})",
            s.slug,
            s.version,
            s.display_description()
        );
    }
    assert!(!manifest.skills.is_empty(), "backend returned no skills");

    let first = &manifest.skills[0];
    let instructions = src
        .fetch_instructions(&first.slug)
        .expect("fetch instructions");
    println!("\nInstructions for '{}':\n{}", first.slug, instructions);
    assert!(!instructions.is_empty());
    // The watermark comment must have been stripped.
    assert!(!instructions.trim_start().starts_with("<!--"));
}

/// End-to-end sync: fetch the real manifest, materialize every skill into an
/// isolated temp directory, and verify the on-disk `SKILL.md` files. This
/// exercises the exact pipeline the desktop app runs, minus the GUI.
#[test]
#[ignore = "requires network and a real license key"]
fn live_full_sync_materializes_skills() {
    let src = source();
    let dir = tempfile::tempdir().unwrap();
    let target = dir.path().to_path_buf();
    let state = target.join(".csm-state.json");

    let cfg = AppConfig {
        backend_url: std::env::var("CSM_ENDPOINT").unwrap(),
        license_key: std::env::var("CSM_LICENSE_KEY").unwrap(),
        ..Default::default()
    };

    let outcome = run_sync(&src, &cfg, &target, &state).expect("first sync");
    println!("installed: {:?}", outcome.installed);
    println!("errors:    {:?}", outcome.errors);
    assert!(!outcome.installed.is_empty(), "nothing was installed");
    assert!(outcome.errors.is_empty(), "unexpected sync errors");

    for slug in &outcome.installed {
        let skill_md = target.join(slug).join("SKILL.md");
        let marker = target.join(slug).join(".csm-managed");
        assert!(skill_md.is_file(), "missing SKILL.md for {slug}");
        assert!(marker.is_file(), "missing managed marker for {slug}");
        let body = std::fs::read_to_string(&skill_md).unwrap();
        assert!(body.starts_with("---\nname: "), "missing frontmatter for {slug}");
        println!("\n===== {slug} =====\n{body}");
    }

    // A second sync with no version changes must be a clean no-op.
    let again = run_sync(&src, &cfg, &target, &state).expect("second sync");
    assert!(!again.changed(), "second sync unexpectedly changed something");
    println!("\nSecond sync was a no-op (idempotent).");
}
