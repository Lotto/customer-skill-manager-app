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

    #[error("subscription inactive (HTTP 402) — check the customer's billing")]
    SubscriptionInactive,

    #[error("license invalid (HTTP 403) — key rejected by the backend")]
    LicenseInvalid,

    #[error("resource not found (HTTP 404): {0}")]
    NotFound(String),

    #[error("rate limited (HTTP 429)")]
    RateLimited,

    #[error("server error (HTTP {0})")]
    ServerError(u16),

    #[error("http error: {0}")]
    Http(String),
}

impl CoreError {
    /// Whether retrying the same request could plausibly succeed. License and
    /// not-found failures are permanent; transport, rate-limit and 5xx are not.
    pub fn is_permanent(&self) -> bool {
        matches!(
            self,
            CoreError::SubscriptionInactive | CoreError::LicenseInvalid | CoreError::NotFound(_)
        )
    }

    /// Whether this is a licensing/billing failure, which the UI should surface
    /// as an "attention" state rather than a transient hiccup.
    pub fn is_license_error(&self) -> bool {
        matches!(
            self,
            CoreError::SubscriptionInactive | CoreError::LicenseInvalid
        )
    }
}

pub type Result<T> = std::result::Result<T, CoreError>;
