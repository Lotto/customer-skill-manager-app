use std::path::PathBuf;

/// Well-known on-disk locations for the app's own data (config, state, logs).
///
/// These live under the OS local-data directory, e.g. on Windows
/// `%LOCALAPPDATA%\CustomerSkillManager\`. Skills themselves are written
/// elsewhere (the resolved target directories, e.g. `~/.claude/skills`).
#[derive(Debug, Clone)]
pub struct AppPaths {
    pub base: PathBuf,
    pub config: PathBuf,
    pub state: PathBuf,
    pub log_dir: PathBuf,
}

impl AppPaths {
    pub fn resolve() -> Self {
        let base = dirs::data_local_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("CustomerSkillManager");
        Self {
            config: base.join("config.toml"),
            state: base.join("state.json"),
            log_dir: base.join("logs"),
            base,
        }
    }
}
