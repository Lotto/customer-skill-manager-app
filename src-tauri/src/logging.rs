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

    // Accept either a bare level ("info") or a full RUST_LOG-style directive.
    let filter = EnvFilter::try_new(level)
        .or_else(|_| EnvFilter::try_from_default_env())
        .unwrap_or_else(|_| EnvFilter::new("info"));

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
