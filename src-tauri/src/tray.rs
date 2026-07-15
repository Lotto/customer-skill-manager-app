use crate::state::AppState;
use crate::status::AppStatus;
use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    AppHandle, Manager, Wry,
};

/// Handles to the mutable tray menu items, so the scheduler can update the
/// status line and reveal the "restart to update" entry at runtime.
pub struct TrayHandles {
    pub status_item: MenuItem<Wry>,
    pub update_item: MenuItem<Wry>,
}

/// Build the tray icon and its menu, and store the mutable item handles in state.
pub fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let status_item = MenuItem::with_id(app, "status", "Démarrage…", false, None::<&str>)?;
    let sync_item = MenuItem::with_id(app, "sync", "Synchroniser maintenant", true, None::<&str>)?;
    let config_item =
        MenuItem::with_id(app, "config", "Ouvrir la configuration", true, None::<&str>)?;
    let logs_item = MenuItem::with_id(
        app,
        "logs",
        "Ouvrir le dossier des logs",
        true,
        None::<&str>,
    )?;
    let update_item = MenuItem::with_id(app, "update", "Aucune mise à jour", false, None::<&str>)?;
    let quit_item = MenuItem::with_id(app, "quit", "Quitter", true, None::<&str>)?;
    let sep1 = PredefinedMenuItem::separator(app)?;
    let sep2 = PredefinedMenuItem::separator(app)?;

    let menu = Menu::with_items(
        app,
        &[
            &status_item,
            &sep1,
            &sync_item,
            &config_item,
            &logs_item,
            &update_item,
            &sep2,
            &quit_item,
        ],
    )?;

    let icon = app
        .default_window_icon()
        .cloned()
        .expect("bundled default window icon");

    TrayIconBuilder::with_id("main")
        .icon(icon)
        .icon_as_template(true)
        .tooltip("Customer Skill Manager")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(handle_menu_event)
        .on_tray_icon_event(handle_tray_event)
        .build(app)?;

    let state = app.state::<AppState>();
    *state.tray.lock().unwrap() = Some(TrayHandles {
        status_item,
        update_item,
    });

    Ok(())
}

fn handle_menu_event(app: &AppHandle, event: tauri::menu::MenuEvent) {
    match event.id.as_ref() {
        "sync" => {
            let tx = app.state::<AppState>().trigger_tx.clone();
            // Non-blocking: if the queue is full a sync is already imminent.
            let _ = tx.try_send(());
        }
        "config" => crate::show_config_window(app),
        "logs" => {
            let dir = app.state::<AppState>().paths.log_dir.clone();
            let _ = std::fs::create_dir_all(&dir);
            if let Err(e) = tauri_plugin_opener::OpenerExt::opener(app)
                .open_path(dir.to_string_lossy(), None::<&str>)
            {
                tracing::warn!("failed to open log dir: {e}");
            }
        }
        "update" => crate::updater::apply_pending_update(app),
        "quit" => {
            tracing::info!("quit requested from tray");
            app.exit(0);
        }
        other => tracing::debug!("unhandled tray menu id: {other}"),
    }
}

fn handle_tray_event(tray: &tauri::tray::TrayIcon, event: TrayIconEvent) {
    if let TrayIconEvent::Click {
        button: MouseButton::Left,
        button_state: MouseButtonState::Up,
        ..
    } = event
    {
        crate::show_config_window(tray.app_handle());
    }
}

/// Refresh the tray status line, tooltip and update entry from a status snapshot.
pub fn refresh(app: &AppHandle, status: &AppStatus) {
    let state = app.state::<AppState>();
    let guard = state.tray.lock().unwrap();
    let Some(handles) = guard.as_ref() else {
        return;
    };

    let _ = handles
        .status_item
        .set_text(format!("État : {}", status.message));

    if let Some(version) = &status.update_available {
        let _ = handles
            .update_item
            .set_text(format!("Redémarrer pour installer {version}"));
        let _ = handles.update_item.set_enabled(true);
    } else {
        let _ = handles.update_item.set_text("Aucune mise à jour");
        let _ = handles.update_item.set_enabled(false);
    }

    if let Some(tray) = app.tray_by_id("main") {
        let _ = tray.set_tooltip(Some(format!(
            "Customer Skill Manager — {}",
            status.phase.tag()
        )));
    }
}
