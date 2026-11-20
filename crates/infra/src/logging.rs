//! Operations log: written asynchronously to a JSON lines file (design 08 section 4.1).

use std::path::{Path, PathBuf};

use serde::Deserialize;
use tracing::Level;
use tracing_appender::rolling::{RollingFileAppender, Rotation};

pub use tracing_appender::non_blocking::WorkerGuard;

/// Log level written in the config file.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// The most detailed development log.
    Trace,
    /// Development log.
    Debug,
    /// Connections, shutdowns, major state changes.
    Info,
    /// Abnormal events that were handled.
    Warn,
    /// Events an operator must check.
    Error,
}

impl From<LogLevel> for Level {
    fn from(level: LogLevel) -> Self {
        match level {
            LogLevel::Trace => Level::TRACE,
            LogLevel::Debug => Level::DEBUG,
            LogLevel::Info => Level::INFO,
            LogLevel::Warn => Level::WARN,
            LogLevel::Error => Level::ERROR,
        }
    }
}

/// Why logging could not be started.
#[derive(Debug, thiserror::Error)]
pub enum LogError {
    /// The log directory or file could not be created.
    #[error("cannot open log file in {}", .dir.display())]
    Open {
        /// Log directory.
        dir: PathBuf,
        /// Cause.
        #[source]
        source: tracing_appender::rolling::InitError,
    },
    /// A global logger is already installed in this process.
    #[error("a global logger is already installed")]
    AlreadyInstalled(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// Installs a global logger that writes JSON logs to `dir/ops.<date>.jsonl`.
///
/// File writes are done by a background thread. Dropping the returned [`WorkerGuard`] flushes the remaining logs and
/// stops writing, so it must be held until the process ends.
pub fn init(dir: &Path, level: LogLevel) -> Result<WorkerGuard, LogError> {
    let appender = RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("ops")
        .filename_suffix("jsonl")
        .build(dir)
        .map_err(|source| LogError::Open {
            dir: dir.to_owned(),
            source,
        })?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    tracing_subscriber::fmt()
        .json()
        .with_max_level(Level::from(level))
        .with_writer(writer)
        .try_init()
        .map_err(LogError::AlreadyInstalled)?;
    Ok(guard)
}
