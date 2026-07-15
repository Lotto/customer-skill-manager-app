use serde::{Deserialize, Serialize};

fn default_target() -> String {
    "global".to_string()
}

/// One skill as advertised by the backend manifest.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct SkillEntry {
    /// Stable identifier, also used as the on-disk directory name.
    pub id: String,
    /// Human-readable name (shown in logs / UI).
    pub name: String,
    /// Semantic version string.
    pub version: String,
    /// Lowercase hex SHA-256 of the skill archive; the change detector.
    pub hash: String,
    /// Named target directory this skill installs into. Defaults to `global`.
    #[serde(default = "default_target")]
    pub target: String,
}

/// The full set of skills the backend says this license is entitled to.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct SkillManifest {
    pub skills: Vec<SkillEntry>,
}

impl SkillManifest {
    pub fn get(&self, id: &str) -> Option<&SkillEntry> {
        self.skills.iter().find(|s| s.id == id)
    }
}

/// Result of `POST /api/license/verify`.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct LicenseInfo {
    pub valid: bool,
    /// RFC 3339 expiry timestamp, if the license is time-limited.
    #[serde(default)]
    pub expires_at: Option<String>,
    /// Skill ids this license covers (informational).
    #[serde(default)]
    pub skills: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn target_defaults_to_global_when_absent() {
        let json = r#"{"id":"a","name":"A","version":"1.0.0","hash":"deadbeef"}"#;
        let e: SkillEntry = serde_json::from_str(json).unwrap();
        assert_eq!(e.target, "global");
    }

    #[test]
    fn manifest_roundtrips() {
        let m = SkillManifest {
            skills: vec![
                SkillEntry {
                    id: "devis".into(),
                    name: "Devis".into(),
                    version: "1.2.0".into(),
                    hash: "abc".into(),
                    target: "global".into(),
                },
                SkillEntry {
                    id: "crm".into(),
                    name: "CRM".into(),
                    version: "0.9.0".into(),
                    hash: "def".into(),
                    target: "acme".into(),
                },
            ],
        };
        let s = serde_json::to_string(&m).unwrap();
        assert_eq!(serde_json::from_str::<SkillManifest>(&s).unwrap(), m);
        assert_eq!(m.get("crm").unwrap().target, "acme");
        assert!(m.get("nope").is_none());
    }
}
