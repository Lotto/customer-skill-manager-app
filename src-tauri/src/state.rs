use crate::app_paths::AppPaths;
use crate::status::AppStatus;
use crate::tray::TrayHandles;
use csm_core::config::AppConfig;
use serde::Serialize;
use std::sync::Mutex;
use tokio::sync::mpsc::Sender;
use tokio::sync::Mutex as AsyncMutex;

/// One entitled skill, as shown in the UI's skills list (name + description
/// only — never the prompt body).
#[derive(Debug, Clone, Serialize)]
pub struct SkillListItem {
    pub slug: String,
    pub description: String,
}

/// A downloaded, verified update waiting for the user to restart. The bytes are
/// fetched in the background; installation happens only on the user's action.
pub struct PendingUpdate {
    pub version: String,
    pub update: tauri_plugin_updater::Update,
    pub bytes: Vec<u8>,
}

/// Process-wide state managed by Tauri and shared across commands, the tray and
/// the scheduler.
pub struct AppState {
    pub paths: AppPaths,
    /// The live configuration. Cloned out before any `.await`; never held across.
    pub config: Mutex<AppConfig>,
    /// The latest status snapshot.
    pub status: Mutex<AppStatus>,
    /// Held for the duration of a sync so cycles never overlap.
    pub sync_lock: AsyncMutex<()>,
    /// Sends a manual "sync now" trigger to the scheduler loop.
    pub trigger_tx: Sender<()>,
    /// Tray menu item handles, populated once the tray is built.
    pub tray: Mutex<Option<TrayHandles>>,
    /// A staged update awaiting restart, if any.
    pub pending_update: Mutex<Option<PendingUpdate>>,
    /// The latest entitled skills (name + description), refreshed each sync.
    pub skills: Mutex<Vec<SkillListItem>>,
}

impl AppState {
    pub fn new(paths: AppPaths, config: AppConfig, trigger_tx: Sender<()>) -> Self {
        Self {
            paths,
            config: Mutex::new(config),
            status: Mutex::new(AppStatus::default()),
            sync_lock: AsyncMutex::new(()),
            trigger_tx,
            tray: Mutex::new(None),
            pending_update: Mutex::new(None),
            skills: Mutex::new(Vec::new()),
        }
    }
}
