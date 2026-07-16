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
        Ok(Ok(summary)) => {
            // Refresh the skills list exposed to the UI (name + description only).
            *state.skills.lock().unwrap() = summary
                .skills
                .iter()
                .map(|(slug, description)| crate::state::SkillListItem {
                    slug: slug.clone(),
                    description: description.clone(),
                })
                .collect();
            summary_to_status(app, summary)
        }
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
    /// (slug, description) for every entitled skill, for the UI list.
    skills: Vec<(String, String)>,
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

    if cfg.sync_to_desktop {
        mirror_to_desktop(&cfg, &global_dir, &outcome);
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

    let skills = outcome
        .manifest
        .skills
        .iter()
        .map(|e| (e.slug.clone(), e.display_description().to_string()))
        .collect();

    Ok(SyncSummary {
        installed: outcome.installed.len(),
        removed: outcome.removed.len(),
        errors: outcome.errors.len(),
        total,
        skills,
    })
}

/// Mirror the entitled skills into Claude Desktop's skill store(s), if any are
/// present on this machine. Best-effort: failures are logged, never fatal.
fn mirror_to_desktop(cfg: &AppConfig, global_dir: &std::path::Path, outcome: &csm_core::sync::SyncOutcome) {
    let Some(roaming) = dirs::config_dir() else {
        return;
    };
    let stores = csm_core::desktop::discover_rpm_stores(&roaming);
    if stores.is_empty() {
        tracing::debug!("no Claude Desktop plugin store found; skipping desktop sync");
        return;
    }
    let read_dir = cfg
        .effective_skill_dirs(global_dir)
        .into_iter()
        .next()
        .unwrap_or_else(|| global_dir.to_path_buf());

    let now = chrono::Utc::now();
    // Match JavaScript's `Date.toISOString()` exactly (millis + `Z`), which is
    // what Claude Desktop writes and validates its manifest against.
    let now_iso = now.to_rfc3339_opts(chrono::SecondsFormat::Millis, true);
    let d = csm_core::desktop::sync_desktop(
        &outcome.manifest,
        &read_dir,
        &stores,
        now.timestamp_millis(),
        &now_iso,
    );
    tracing::info!(
        stores = d.stores,
        installed = d.installed,
        removed = d.removed,
        "mirrored skills to Claude Desktop"
    );
    for (store, err) in &d.errors {
        tracing::warn!(store = store.as_str(), "desktop sync error: {err}");
    }
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
