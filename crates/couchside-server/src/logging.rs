//! Logging to daily rolling files (and the console when interactive).

use std::path::Path;
use tracing_appender::non_blocking::WorkerGuard;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;
use tracing_subscriber::{fmt, EnvFilter};

/// Number of daily log files kept.
const MAX_LOG_FILES: usize = 14;

/// Starts logging. Keep the returned guard alive until shutdown so logs are flushed.
pub fn init(log_dir: &Path, console: bool) -> anyhow::Result<WorkerGuard> {
    std::fs::create_dir_all(log_dir)?;
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("couchside")
        .filename_suffix("log")
        .max_log_files(MAX_LOG_FILES)
        .build(log_dir)?;
    let (file_writer, guard) = tracing_appender::non_blocking(appender);

    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("info,sqlx=warn,hyper=warn"));
    let file_layer = fmt::layer().with_writer(file_writer).with_ansi(false);
    let console_layer = console.then(|| fmt::layer().with_writer(std::io::stdout));

    tracing_subscriber::registry()
        .with(filter)
        .with(file_layer)
        .with(console_layer)
        .try_init()?;
    Ok(guard)
}
