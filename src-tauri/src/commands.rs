use crate::state::AppState;
use crate::status::AppStatus;
use csm_core::config::AppConfig;
use std::path::PathBuf;
use tauri::{AppHandle, Manager};
use tauri_plugin_dialog::DialogExt;

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

/// Return the entitled skills (name + description only — no prompt body).
#[tauri::command]
pub fn list_skills(state: tauri::State<AppState>) -> Vec<crate::state::SkillListItem> {
    state.skills.lock().unwrap().clone()
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

/// The default global skills directory (`~/.claude/skills`), shown in the UI as
/// the fallback destination when no folder is configured.
#[tauri::command]
pub fn default_skill_dir() -> String {
    csm_core::paths::global_skills_dir()
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_default()
}

/// Open the skills directory in the system file browser. With `path`, opens
/// that specific folder; without it, opens every effective destination
/// directory (the configured ones, or the global default).
#[tauri::command]
pub fn open_skills(app: AppHandle, path: Option<String>) -> Result<(), String> {
    let dirs: Vec<PathBuf> = match path {
        Some(p) => vec![PathBuf::from(p)],
        None => {
            let cfg = app.state::<AppState>().config.lock().unwrap().clone();
            let global = csm_core::paths::global_skills_dir().map_err(|e| e.to_string())?;
            cfg.effective_skill_dirs(&global)
        }
    };
    let opener = tauri_plugin_opener::OpenerExt::opener(&app);
    for dir in dirs {
        let _ = std::fs::create_dir_all(&dir);
        opener
            .open_path(dir.to_string_lossy(), None::<&str>)
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Kill and relaunch Claude Desktop (so it re-reads freshly published skills).
#[tauri::command]
pub async fn reload_claude_desktop() -> Result<String, String> {
    tauri::async_runtime::spawn_blocking(crate::desktop_control::reload)
        .await
        .map_err(|e| e.to_string())?
}

/// Open a native folder picker and return the chosen path, or `None` if
/// cancelled. Used by the UI to add a skills destination directory.
#[tauri::command]
pub async fn pick_skill_dir(app: AppHandle) -> Option<String> {
    let (tx, rx) = tokio::sync::oneshot::channel();
    app.dialog().file().pick_folder(move |picked| {
        let _ = tx.send(picked);
    });
    rx.await.ok().flatten().map(|p| p.to_string())
}
