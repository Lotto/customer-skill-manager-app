//! Customer Skill Manager — a silent, tray-resident desktop agent that keeps a
//! customer's licensed Claude Code skills in sync and updates itself.
//!
//! The GUI-free logic lives in the `csm-core` crate; this crate is the Tauri
//! shell: tray, scheduler, updater, configuration UI and OS integration.

mod app_paths;
mod commands;
mod logging;
mod scheduler;
mod state;
mod status;
mod tray;
mod updater;

use app_paths::AppPaths;
use csm_core::config::AppConfig;
use state::AppState;
use tauri::{AppHandle, Manager, WindowEvent};
use tauri_plugin_autostart::{MacosLauncher, ManagerExt};

/// Show and focus the configuration window (creating nothing; it always exists,
/// just hidden).
pub fn show_config_window(app: &AppHandle) {
    if let Some(window) = app.get_webview_window("main") {
        let _ = window.show();
        let _ = window.unminimize();
        let _ = window.set_focus();
    }
}

/// Application entry point (called from `main`).
pub fn run() {
    let paths = AppPaths::resolve();
    let config = AppConfig::load(&paths.config).unwrap_or_default();

    // Keep the logging worker alive for the whole process.
    let guard = logging::init(&paths, &config.log_level);
    std::mem::forget(guard);

    tracing::info!(
        version = env!("CARGO_PKG_VERSION"),
        "starting Customer Skill Manager"
    );

    let (trigger_tx, trigger_rx) = tokio::sync::mpsc::channel::<()>(8);
    let app_state = AppState::new(paths, config, trigger_tx);

    tauri::Builder::default()
        // single-instance must be registered first so a second launch just
        // surfaces the existing window instead of starting a rival agent.
        .plugin(tauri_plugin_single_instance::init(|app, _argv, _cwd| {
            tracing::info!("second instance launched; focusing config window");
            show_config_window(app);
        }))
        .plugin(tauri_plugin_autostart::init(
            MacosLauncher::LaunchAgent,
            Some(vec!["--minimized"]),
        ))
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .manage(app_state)
        .invoke_handler(tauri::generate_handler![
            commands::get_status,
            commands::get_config,
            commands::save_config,
            commands::sync_now,
            commands::open_logs,
        ])
        .setup(move |app| {
            let handle = app.handle().clone();

            // Run at login (idempotent; ignore failures on locked-down machines).
            if let Err(e) = app.autolaunch().enable() {
                tracing::warn!("could not enable autostart: {e}");
            }

            tray::build_tray(&handle)?;

            // First run without a license: reveal the activation window. This is
            // the only time the app shows itself unprompted.
            let activated = handle
                .state::<AppState>()
                .config
                .lock()
                .unwrap()
                .is_activated();
            if !activated {
                tracing::info!("no license configured; opening activation window");
                show_config_window(&handle);
            }

            // Start the background sync + update scheduler.
            let sched_handle = handle.clone();
            tauri::async_runtime::spawn(async move {
                scheduler_loop(sched_handle, trigger_rx).await;
            });

            Ok(())
        })
        .on_window_event(|window, event| {
            // Closing the window hides it; the agent keeps running in the tray.
            if let WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("failed to build the Tauri application")
        .run(|_app, _event| {});
}

/// The scheduler loop: sync, then check for app updates, then wait for the
/// interval or a manual trigger. Kept here so it owns the trigger receiver.
async fn scheduler_loop(app: AppHandle, mut rx: tokio::sync::mpsc::Receiver<()>) {
    use tokio::time::sleep;
    loop {
        scheduler::run_once(&app).await;
        updater::maybe_check(&app).await;

        let interval = {
            let state = app.state::<AppState>();
            let cfg = state.config.lock().unwrap();
            cfg.interval()
        };

        tokio::select! {
            _ = sleep(interval) => {}
            msg = rx.recv() => {
                if msg.is_none() {
                    tracing::info!("trigger channel closed; scheduler stopping");
                    break;
                }
                tracing::debug!("manual sync trigger");
            }
        }
    }
}
