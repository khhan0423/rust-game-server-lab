//! 운영 로그: JSON 줄 파일에 비동기로 쓴다 (설계 08 4.1).

use std::path::{Path, PathBuf};

use serde::Deserialize;
use tracing::Level;
use tracing_appender::rolling::{RollingFileAppender, Rotation};

pub use tracing_appender::non_blocking::WorkerGuard;

/// 설정 파일에 적는 로그 레벨.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    /// 가장 자세한 개발용 로그.
    Trace,
    /// 개발용 로그.
    Debug,
    /// 접속, 종료, 주요 상태 변화.
    Info,
    /// 비정상이지만 처리한 일.
    Warn,
    /// 운영자가 확인해야 하는 일.
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

/// 로그를 시작하지 못한 이유.
#[derive(Debug, thiserror::Error)]
pub enum LogError {
    /// 로그 디렉터리나 파일을 만들지 못했다.
    #[error("cannot open log file in {}", .dir.display())]
    Open {
        /// 로그 디렉터리.
        dir: PathBuf,
        /// 원인.
        #[source]
        source: tracing_appender::rolling::InitError,
    },
    /// 이 프로세스에 이미 전역 로거가 설치돼 있다.
    #[error("a global logger is already installed")]
    AlreadyInstalled(#[source] Box<dyn std::error::Error + Send + Sync>),
}

/// `dir/ops.<날짜>.jsonl`에 JSON 로그를 쓰도록 전역 로거를 설치한다.
///
/// 파일 쓰기는 백그라운드 스레드가 한다. 돌려받은 [`WorkerGuard`]를 버리면 남은 로그를 비우고
/// 쓰기를 멈추므로, 프로세스가 끝날 때까지 들고 있어야 한다.
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
