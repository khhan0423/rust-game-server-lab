//! Operations log: written asynchronously to a JSON lines file (design 08 section 4.1).

mod json;
#[cfg(test)]
mod tests;

use std::path::PathBuf;

use serde::Deserialize;
use tracing::Level;
use tracing_appender::rolling::{RollingFileAppender, Rotation};
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

pub use tracing_appender::non_blocking::WorkerGuard;

/// Deployment stage recorded on each log line.
#[derive(Debug, Clone, Copy, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Environment {
    /// Local development.
    Development,
    /// Private friends test.
    FriendsTest,
    /// Invite-only alpha.
    Alpha,
    /// Beta test.
    Beta,
    /// Live service.
    Live,
}

/// Kind of process writing the operations log.
#[derive(Debug, Clone, Copy, Deserialize, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ProcessName {
    /// Publisher login server.
    Publisher,
    /// Game server.
    Game,
    /// Gateway server.
    Gateway,
    /// Operations tool.
    Ops,
    /// Headless test bot.
    Bot,
    /// Desktop test client.
    Client,
}

impl ProcessName {
    const fn as_str(self) -> &'static str {
        match self {
            Self::Publisher => "publisher",
            Self::Game => "game",
            Self::Gateway => "gateway",
            Self::Ops => "ops",
            Self::Bot => "bot",
            Self::Client => "client",
        }
    }
}

/// A short label that can be used inside a log directory name.
#[derive(Debug, Clone, Deserialize)]
#[serde(try_from = "String")]
pub struct Instance(String);

impl Default for Instance {
    fn default() -> Self {
        Self("1".into())
    }
}

impl TryFrom<String> for Instance {
    type Error = InvalidInstance;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        {
            return Err(InvalidInstance);
        }
        Ok(Self(value))
    }
}

/// The instance label cannot be used in a log directory name.
#[derive(Debug, thiserror::Error)]
#[error("instance must be 1 to 64 ASCII letters, numbers, dashes, or underscores")]
pub struct InvalidInstance;

/// Identity and operations log settings kept in the `[process]` table.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LogConfig {
    /// Kind of process writing the log.
    pub name: ProcessName,
    /// Instance label, or the client profile name.
    #[serde(default)]
    pub instance: Instance,
    /// Deployment stage.
    pub environment: Environment,
    /// Root containing one directory per process instance.
    #[serde(default = "default_log_root")]
    pub log_root: PathBuf,
    /// Operations log level.
    pub log_level: LogLevel,
}

fn default_log_root() -> PathBuf {
    PathBuf::from("logs")
}

impl LogConfig {
    fn directory(&self) -> PathBuf {
        self.log_root
            .join(format!("{}-{}", self.name.as_str(), self.instance.0))
    }
}

/// Top-level config for a tool that does not open an admin port.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolConfig<T = LogConfig> {
    /// Identity and operations log settings.
    pub process: T,
}

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

/// Installs a global logger that writes JSON to `<log_root>/<name>-<instance>/ops.<date>.jsonl`.
///
/// File writes are done by a background thread. Dropping the returned [`WorkerGuard`] flushes the remaining logs and
/// stops writing, so it must be held until the process ends.
pub fn init(config: &LogConfig) -> Result<WorkerGuard, LogError> {
    let appender = appender(config)?;
    let (writer, guard) = tracing_appender::non_blocking(appender);
    subscriber(config, writer)
        .try_init()
        .map_err(|source| LogError::AlreadyInstalled(Box::new(source)))?;
    Ok(guard)
}

fn subscriber(
    config: &LogConfig,
    writer: impl for<'a> tracing_subscriber::fmt::MakeWriter<'a> + Send + Sync + 'static,
) -> impl tracing::Subscriber + Send + Sync {
    tracing_subscriber::registry()
        .with(tracing_subscriber::filter::LevelFilter::from_level(
            Level::from(config.log_level),
        ))
        .with(json::SpanFields)
        .with(
            tracing_subscriber::fmt::layer()
                .event_format(json::Line::from(config))
                .fmt_fields(json::SpanFields)
                .with_writer(writer),
        )
}

fn appender(config: &LogConfig) -> Result<RollingFileAppender, LogError> {
    let dir = config.directory();
    RollingFileAppender::builder()
        .rotation(Rotation::DAILY)
        .filename_prefix("ops")
        .filename_suffix("jsonl")
        .build(&dir)
        .map_err(|source| LogError::Open { dir, source })
}
