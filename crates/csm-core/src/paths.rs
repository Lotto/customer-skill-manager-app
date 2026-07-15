use crate::error::{CoreError, Result};
use std::path::PathBuf;

/// The default global Claude skills directory: `~/.claude/skills`.
///
/// Returns [`CoreError::NoGlobalDir`] if the OS reports no home directory.
pub fn global_skills_dir() -> Result<PathBuf> {
    dirs::home_dir()
        .map(|h| h.join(".claude").join("skills"))
        .ok_or(CoreError::NoGlobalDir)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn global_skills_dir_ends_with_claude_skills() {
        // On any real dev/test machine there is a home dir.
        let p = global_skills_dir().unwrap();
        assert!(p.ends_with("skills"));
        assert!(p.to_string_lossy().contains(".claude"));
    }
}
