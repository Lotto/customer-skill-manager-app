use crate::state::AppState;
use crate::status::AppStatus;
use csm_core::config::AppConfig;
use tauri::{AppHandle, Manager};

/// Return the latest status snapshot.
#[tauri::command]
pub fn get_status(state: tauri::State<AppState>) -> AppStatus {
    state.status.lock().unwrap().clone()
}

/// Return the current configuration.
#[tauri::command]
pub fn get_config(state: tauri::State<AppState>) -> AppConfig {
    state.config.lock().unwrap().clone()
}

/// Persist a new configuration, apply it in memory, and kick off a sync. The
/// log level takes effect on next restart.
#[tauri::command]
pub async fn save_config(app: AppHandle, config: AppConfig) -> Result<(), String> {
    let (path, tx) = {
        let state = app.state::<AppState>();
        (state.paths.config.clone(), state.trigger_tx.clone())
    };

    config.save(&path).map_err(|e| e.to_string())?;
    {
        let state = app.state::<AppState>();
        *state.config.lock().unwrap() = config;
    }
    tracing::info!("configuration updated via UI");

    let _ = tx.send(()).await;
    Ok(())
}

/// Trigger an immediate sync.
#[tauri::command]
pub async fn sync_now(app: AppHandle) -> Result<(), String> {
    let tx = { app.state::<AppState>().trigger_tx.clone() };
    tx.send(()).await.map_err(|e| e.to_string())
}

/// Open the log directory in the system file browser.
#[tauri::command]
pub fn open_logs(app: AppHandle) -> Result<(), String> {
    let dir = { app.state::<AppState>().paths.log_dir.clone() };
    let _ = std::fs::create_dir_all(&dir);
    tauri_plugin_opener::OpenerExt::opener(&app)
        .open_path(dir.to_string_lossy(), None::<&str>)
        .map_err(|e| e.to_string())
}
