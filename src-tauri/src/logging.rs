use crate::app_paths::AppPaths;
use tracing_subscriber::prelude::*;
use tracing_subscriber::EnvFilter;

/// Initialize `tracing` to a daily-rotating file in the log directory plus
/// stdout (visible only in debug builds — the release binary is a windowless
/// tray process). The returned worker guard must outlive the process, so the
/// caller leaks it intentionally.
pub fn init(paths: &AppPaths, level: &str) -> tracing_appender::non_blocking::WorkerGuard {
    let _ = std::fs::create_dir_all(&paths.log_dir);

    let file_appender = tracing_appender::rolling::daily(&paths.log_dir, "csm.log");
    let (non_blocking, guard) = tracing_appender::non_blocking(file_appender);

    // Accept either a bare level ("info") or a full RUST_LOG-style directive,
    // and silence the updater plugin's own ERROR log for the expected
    // "no release yet / endpoint 404" case (our own updater module logs the
    // outcome at a sane level instead).
    let base = if level.trim().is_empty() {
        "info"
    } else {
        level
    };
    let directive = format!("{base},tauri_plugin_updater=off");
    let filter = EnvFilter::try_new(&directive)
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("info,tauri_plugin_updater=off"));

    tracing_subscriber::registry()
        .with(filter)
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(non_blocking),
        )
        .with(
            tracing_subscriber::fmt::layer()
                .with_writer(std::io::stdout)
                .with_filter(tracing_subscriber::filter::LevelFilter::DEBUG),
        )
        .init();

    guard
}
