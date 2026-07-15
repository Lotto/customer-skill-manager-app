use serde::Serialize;
use std::time::{SystemTime, UNIX_EPOCH};

/// Coarse state of the agent, surfaced to both the tray icon and the UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum SyncPhase {
    /// No license configured yet.
    NeedsActivation,
    /// A sync is currently running.
    Syncing,
    /// Last sync succeeded.
    Ok,
    /// Last sync failed transiently (network, server); will retry.
    Error,
    /// License/billing problem; syncing is paused until resolved.
    LicenseProblem,
}

impl SyncPhase {
    /// Short tag used in the tray tooltip.
    pub fn tag(self) -> &'static str {
        match self {
            SyncPhase::NeedsActivation => "activation requise",
            SyncPhase::Syncing => "synchronisation…",
            SyncPhase::Ok => "à jour",
            SyncPhase::Error => "erreur",
            SyncPhase::LicenseProblem => "licence",
        }
    }
}

/// Full status snapshot shared with the UI (serialized to the frontend).
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub phase: SyncPhase,
    pub message: String,
    /// Unix seconds of the last completed sync attempt, if any.
    pub last_sync_epoch: Option<u64>,
    /// Number of skills currently managed on disk.
    pub installed_count: usize,
    /// Version string of a downloaded update awaiting restart, if any.
    pub update_available: Option<String>,
}

impl Default for AppStatus {
    fn default() -> Self {
        Self {
            phase: SyncPhase::NeedsActivation,
            message: "En attente d'activation".to_string(),
            last_sync_epoch: None,
            installed_count: 0,
            update_available: None,
        }
    }
}

/// Current Unix time in seconds (best-effort; 0 if the clock is before epoch).
pub fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}
