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

use csm_core::http::HttpSkillSource;
use csm_core::sync::SkillSource;
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
