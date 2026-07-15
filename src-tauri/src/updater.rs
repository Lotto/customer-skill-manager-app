use crate::scheduler::apply_status;
use crate::state::{AppState, PendingUpdate};
use tauri::{AppHandle, Manager};
use tauri_plugin_updater::UpdaterExt;

/// Check for an update and, if one exists, download it in the background and
/// stage it for the next restart. Semi-automatic by design: we never restart
/// the app on our own, since a restart mid-sync could interrupt a skill write.
pub async fn maybe_check(app: &AppHandle) {
    // Don't re-download if an update is already staged.
    if app
        .state::<AppState>()
        .pending_update
        .lock()
        .unwrap()
        .is_some()
    {
        return;
    }
    match do_check(app).await {
        Ok(Some(v)) => tracing::info!("update {v} downloaded; awaiting restart"),
        Ok(None) => tracing::debug!("no update available"),
        Err(e) => tracing::debug!("update check skipped: {e}"),
    }
}

async fn do_check(app: &AppHandle) -> tauri_plugin_updater::Result<Option<String>> {
    let Some(update) = app.updater()?.check().await? else {
        return Ok(None);
    };
    let version = update.version.clone();
    tracing::info!("update {version} available; downloading");

    let bytes = update.download(|_chunk, _total| {}, || {}).await?;

    {
        let state = app.state::<AppState>();
        *state.pending_update.lock().unwrap() = Some(PendingUpdate {
            version: version.clone(),
            update,
            bytes,
        });
    }

    // Surface it in the status/tray.
    let mut status = app.state::<AppState>().status.lock().unwrap().clone();
    status.update_available = Some(version.clone());
    apply_status(app, status);

    Ok(Some(version))
}

/// Install the staged update and restart. Invoked from the tray entry.
pub fn apply_pending_update(app: &AppHandle) {
    let pending = app
        .state::<AppState>()
        .pending_update
        .lock()
        .unwrap()
        .take();

    let Some(pending) = pending else {
        tracing::debug!("apply update requested but nothing is staged");
        return;
    };

    tracing::info!("installing update {} and restarting", pending.version);
    if let Err(e) = pending.update.install(pending.bytes) {
        // Installation consumed the staged bytes; the next scheduled check will
        // re-download and re-stage. Surface the failure and leave the app running.
        tracing::error!("failed to install update {}: {e}", pending.version);
        return;
    }
    app.restart();
}
