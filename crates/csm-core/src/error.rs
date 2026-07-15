use thiserror::Error;

/// Errors produced by the core layer.
#[derive(Debug, Error)]
pub enum CoreError {
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),

    #[error("json error: {0}")]
    Json(#[from] serde_json::Error),

    #[error("toml deserialize error: {0}")]
    TomlDe(#[from] toml::de::Error),

    #[error("toml serialize error: {0}")]
    TomlSer(#[from] toml::ser::Error),

    #[error("hash mismatch for skill '{id}': expected {expected}, got {actual}")]
    HashMismatch {
        id: String,
        expected: String,
        actual: String,
    },

    #[error("unknown target '{0}' (not declared in config.targets)")]
    UnknownTarget(String),

    #[error("could not resolve the global skills directory (no home dir)")]
    NoGlobalDir,

    #[error("config error: {0}")]
    Config(String),

    #[error("http error: {0}")]
    Http(String),
}

pub type Result<T> = std::result::Result<T, CoreError>;
