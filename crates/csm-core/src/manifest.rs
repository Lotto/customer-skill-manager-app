use serde::{Deserialize, Serialize};

fn default_target() -> String {
    "global".to_string()
}

/// One skill as advertised by the backend `__list` resource.
///
/// The CSM backend is version-driven: there is no per-skill archive hash, so
/// change detection keys off [`SkillEntry::version`]. Content is delivered as
/// markdown (the `instructions` resource), which the app materializes to disk.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillEntry {
    /// Stable identifier and on-disk directory name (e.g. `test-moi`).
    pub slug: String,
    /// Human-readable description used for the skill's frontmatter. May be
    /// absent in the listing.
    #[serde(default)]
    pub description: Option<String>,
    /// Version string; the change-detection signal.
    pub version: String,
    /// Named target directory this skill installs into. Defaults to `global`.
    #[serde(default = "default_target")]
    pub target: String,
}

impl SkillEntry {
    /// Description to use for the skill's frontmatter, falling back to the slug.
    pub fn display_description(&self) -> &str {
        self.description
            .as_deref()
            .filter(|d| !d.is_empty())
            .unwrap_or(&self.slug)
    }
}

/// The set of skills the backend says this license is entitled to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SkillManifest {
    pub skills: Vec<SkillEntry>,
}

impl SkillManifest {
    pub fn get(&self, slug: &str) -> Option<&SkillEntry> {
        self.skills.iter().find(|s| s.slug == slug)
    }
}

/// Extract the value of a `- **key** : value` markdown line, unwrapping any
/// surrounding backticks. Returns `None` if the line is not that field.
fn field_value(line: &str, key: &str) -> Option<String> {
    let marker = format!("**{key}**");
    let idx = line.find(&marker)?;
    let rest = line[idx + marker.len()..].trim_start();
    let rest = rest.strip_prefix(':').unwrap_or(rest).trim();
    let val = rest.trim_matches('`').trim();
    (!val.is_empty()).then(|| val.to_string())
}

#[derive(Default)]
struct EntryBuilder {
    slug: Option<String>,
    description: Option<String>,
    version: Option<String>,
}

impl EntryBuilder {
    fn build(self) -> Option<SkillEntry> {
        Some(SkillEntry {
            slug: self.slug?,
            version: self.version?,
            description: self.description,
            target: default_target(),
        })
    }
}

/// Parse the markdown returned by `?resource=__list` into a [`SkillManifest`].
///
/// The format is a sequence of `## <title>` blocks, each carrying
/// `- **slug** : \`...\``, `- **version** : ...` and an optional
/// `- **description** : ...`. The slug comes from the field, never the header,
/// because the header can be a prettified name (e.g. header `## test moi`,
/// slug `test-moi`). Any leading HTML comment metadata block is ignored.
pub fn parse_skill_list(md: &str) -> SkillManifest {
    let mut skills = Vec::new();
    let mut current: Option<EntryBuilder> = None;

    for line in md.lines() {
        let t = line.trim();
        if let Some(rest) = t.strip_prefix("## ") {
            if let Some(built) = current.take().and_then(EntryBuilder::build) {
                skills.push(built);
            }
            let _ = rest; // header title is intentionally unused
            current = Some(EntryBuilder::default());
            continue;
        }
        if let Some(b) = current.as_mut() {
            if let Some(v) = field_value(t, "slug") {
                b.slug = Some(v);
            } else if let Some(v) = field_value(t, "version") {
                b.version = Some(v);
            } else if let Some(v) = field_value(t, "description") {
                b.description = Some(v);
            }
        }
    }
    if let Some(built) = current.take().and_then(EntryBuilder::build) {
        skills.push(built);
    }
    SkillManifest { skills }
}

/// Result of a license verification, derived from the backend response.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct LicenseInfo {
    pub valid: bool,
    pub customer: Option<String>,
    pub message: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The exact `__list` body returned by the live backend (2026-07-15).
    const REAL_LIST: &str = r#"<!--
csm:served 2026-07-15T13:09:32.357Z
csm:license-mask csm_live_cdd7…a41e
csm:customer "Test"
csm:skill "__list@—"
-->
# Skills disponibles pour Test

## bonjour
- **slug** : `bonjour`
- **version** : 1.0.0
- **description** : répond bonjour
- **chargement** : `python fetch.py instructions bonjour`

## bye
- **slug** : `bye`
- **version** : 1.0.0
- **description** : répond bye
- **chargement** : `python fetch.py instructions bye`

## coucou
- **slug** : `coucou`
- **version** : 1.0.0
- **chargement** : `python fetch.py instructions coucou`

## test moi
- **slug** : `test-moi`
- **version** : 1.0.0
- **chargement** : `python fetch.py instructions test-moi`
"#;

    #[test]
    fn parses_real_backend_list() {
        let m = parse_skill_list(REAL_LIST);
        let slugs: Vec<&str> = m.skills.iter().map(|s| s.slug.as_str()).collect();
        assert_eq!(slugs, vec!["bonjour", "bye", "coucou", "test-moi"]);

        let bonjour = m.get("bonjour").unwrap();
        assert_eq!(bonjour.version, "1.0.0");
        assert_eq!(bonjour.description.as_deref(), Some("répond bonjour"));
        assert_eq!(bonjour.target, "global");

        // `coucou` has no description line.
        assert_eq!(m.get("coucou").unwrap().description, None);

        // Slug comes from the field, not the header "## test moi".
        assert!(m.get("test-moi").is_some());
        assert_eq!(m.get("test-moi").unwrap().display_description(), "test-moi");
    }

    #[test]
    fn empty_input_yields_no_skills() {
        assert!(parse_skill_list("").skills.is_empty());
        assert!(parse_skill_list("# just a heading\n").skills.is_empty());
    }

    #[test]
    fn block_without_slug_is_dropped() {
        let md = "## broken\n- **version** : 1.0.0\n";
        assert!(parse_skill_list(md).skills.is_empty());
    }

    #[test]
    fn display_description_falls_back_to_slug() {
        let e = SkillEntry {
            slug: "x".into(),
            description: None,
            version: "1".into(),
            target: "global".into(),
        };
        assert_eq!(e.display_description(), "x");
    }
}
