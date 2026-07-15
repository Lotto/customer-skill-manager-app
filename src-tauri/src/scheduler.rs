use crate::state::AppState;
use crate::status::{now_epoch, AppStatus, SyncPhase};
use csm_core::config::AppConfig;
use csm_core::http::HttpSkillSource;
use csm_core::state::InstalledState;
use csm_core::sync::run_sync;
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// Per-request timeout for backend calls (mirrors the loader's `CSM_TIMEOUT`).
const REQUEST_TIMEOUT: Duration = Duration::from_secs(15);

/// Run exactly one sync cycle, updating status and the tray. Never panics: all
/// failures are folded into an error status.
pub async fn run_once(app: &AppHandle) {
    let state = app.state::<AppState>();
    // Serialize cycles so a slow sync and a manual trigger can't overlap.
    let _guard = state.sync_lock.lock().await;

    let cfg = state.config.lock().unwrap().clone();
    if !cfg.is_activated() {
        apply_status(
            app,
            AppStatus {
                phase: SyncPhase::NeedsActivation,
                message: "En attente d'activation".into(),
                ..current(app)
            },
        );
        return;
    }

    apply_status(
        app,
        AppStatus {
            phase: SyncPhase::Syncing,
            message: "Synchronisation…".into(),
            ..current(app)
        },
    );

    let state_path = state.paths.state.clone();
    let result = tauri::async_runtime::spawn_blocking(move || run_blocking(cfg, state_path)).await;

    let new_status = match result {
        Ok(Ok(summary)) => summary_to_status(app, summary),
        Ok(Err(e)) => {
            let license = e.is_license_error();
            tracing::warn!("sync failed: {e}");
            AppStatus {
                phase: if license {
                    SyncPhase::LicenseProblem
                } else {
                    SyncPhase::Error
                },
                message: e.to_string(),
                last_sync_epoch: Some(now_epoch()),
                ..current(app)
            }
        }
        Err(join_err) => {
            tracing::error!("sync task panicked: {join_err}");
            AppStatus {
                phase: SyncPhase::Error,
                message: "Erreur interne pendant la synchronisation".into(),
                last_sync_epoch: Some(now_epoch()),
                ..current(app)
            }
        }
    };

    apply_status(app, new_status);
}

/// The blocking part of a sync (network + disk), run off the async runtime.
struct SyncSummary {
    installed: usize,
    removed: usize,
    errors: usize,
    total: usize,
}

fn run_blocking(cfg: AppConfig, state_path: std::path::PathBuf) -> csm_core::Result<SyncSummary> {
    let source = HttpSkillSource::new(
        cfg.backend_url.clone(),
        cfg.license_key.clone(),
        REQUEST_TIMEOUT,
    )?;
    let global_dir = csm_core::paths::global_skills_dir()?;
    let outcome = run_sync(&source, &cfg, &global_dir, &state_path)?;
    let total = InstalledState::load(&state_path)?.skills.len();

    for (slug, err) in &outcome.errors {
        tracing::warn!(skill = slug.as_str(), "skill sync error: {err}");
    }
    if outcome.changed() {
        tracing::info!(
            installed = outcome.installed.len(),
            removed = outcome.removed.len(),
            "sync applied changes"
        );
    } else {
        tracing::debug!("sync: already up to date");
    }

    Ok(SyncSummary {
        installed: outcome.installed.len(),
        removed: outcome.removed.len(),
        errors: outcome.errors.len(),
        total,
    })
}

fn summary_to_status(app: &AppHandle, s: SyncSummary) -> AppStatus {
    let message = if s.errors > 0 {
        format!(
            "{} skill(s) en échec — nouvelle tentative au prochain cycle",
            s.errors
        )
    } else if s.installed > 0 || s.removed > 0 {
        format!("{} installé(s), {} retiré(s)", s.installed, s.removed)
    } else {
        "À jour".into()
    };
    AppStatus {
        phase: if s.errors > 0 {
            SyncPhase::Error
        } else {
            SyncPhase::Ok
        },
        message,
        last_sync_epoch: Some(now_epoch()),
        installed_count: s.total,
        ..current(app)
    }
}

/// Snapshot the current status (used to preserve fields like `update_available`
/// when only some fields change).
fn current(app: &AppHandle) -> AppStatus {
    app.state::<AppState>().status.lock().unwrap().clone()
}

/// Store a new status, notify the UI, and refresh the tray.
pub fn apply_status(app: &AppHandle, status: AppStatus) {
    {
        let state = app.state::<AppState>();
        *state.status.lock().unwrap() = status.clone();
    }
    let _ = app.emit("status", &status);
    crate::tray::refresh(app, &status);
}
