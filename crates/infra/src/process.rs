//! 서버 프로세스 공통 기동과 종료: 로그, 지표, 관리 포트, 준비 통지.

use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;

use metrics_exporter_prometheus::{BuildError, PrometheusBuilder};
use serde::Deserialize;
use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::admin::{AdminListener, Readiness};
use crate::logging::{self, LogError, LogLevel, WorkerGuard};

/// 모든 서버 프로세스가 설정 파일의 `[process]` 표에 두는 항목.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessConfig {
    /// 관리 포트 주소. 사설망 주소만 쓴다.
    pub admin_addr: SocketAddr,
    /// 운영 로그 디렉터리.
    pub log_dir: PathBuf,
    /// 운영 로그 레벨.
    pub log_level: LogLevel,
}

/// 프로세스 공통 부분이 실패한 이유.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    /// 로그를 시작하지 못했다.
    #[error("cannot start logging")]
    Log(#[from] LogError),
    /// 지표 수집기를 설치하지 못했다.
    #[error("cannot install metrics recorder")]
    Metrics(#[from] BuildError),
    /// 관리 포트를 열지 못했다.
    #[error("cannot bind admin port {addr}")]
    AdminBind {
        /// 열려던 주소.
        addr: SocketAddr,
        /// 원인.
        #[source]
        source: std::io::Error,
    },
    /// 관리 포트가 요청을 처리하다 실패했다.
    #[error("admin port failed")]
    AdminServe(#[source] std::io::Error),
    /// 관리 포트 task가 비정상 종료했다.
    #[error("admin task stopped unexpectedly")]
    AdminTask(#[from] JoinError),
    /// 준비 통지 줄을 표준 출력에 쓰지 못했다.
    #[error("cannot announce readiness")]
    Announce(#[source] std::io::Error),
    /// 종료 신호를 기다릴 수 없다.
    #[error("cannot listen for shutdown signal")]
    Signal(#[source] std::io::Error),
}

/// 떠 있는 서버 프로세스의 공통 부분.
#[derive(Debug)]
pub struct Process {
    readiness: Readiness,
    admin_addr: SocketAddr,
    admin_task: JoinHandle<std::io::Result<()>>,
    shutdown: CancellationToken,
    _log: WorkerGuard,
}

impl Process {
    /// 로그와 지표를 켜고 관리 포트를 연다. 이 시점에는 `/health/live`만 200이다.
    pub async fn start(config: &ProcessConfig) -> Result<Self, ProcessError> {
        let log = logging::init(&config.log_dir, config.log_level)?;
        let metrics = PrometheusBuilder::new().install_recorder()?;
        let listener = AdminListener::bind(config.admin_addr)
            .await
            .map_err(|source| ProcessError::AdminBind {
                addr: config.admin_addr,
                source,
            })?;
        let admin_addr = listener.local_addr();
        let readiness = Readiness::default();
        let shutdown = CancellationToken::new();
        let admin_task = tokio::spawn(listener.serve(readiness.clone(), metrics, shutdown.clone()));
        Ok(Self {
            readiness,
            admin_addr,
            admin_task,
            shutdown,
            _log: log,
        })
    }

    /// 기동 배리어가 끝났음을 알린다. `/health/ready`를 켜고, 부모 프로세스가 읽도록
    /// 표준 출력에 `ready admin=<주소>` 한 줄을 쓴다(런처와 통합 테스트가 이 줄을 기다린다).
    pub fn mark_ready(&self) -> Result<(), ProcessError> {
        self.readiness.set_ready();
        metrics::gauge!("server_ready").set(1.0);
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "ready admin={}", self.admin_addr)
            .and_then(|()| stdout.flush())
            .map_err(ProcessError::Announce)
    }

    /// 종료 신호(Ctrl+C)를 기다린 뒤 관리 포트를 닫는다.
    pub async fn run_until_shutdown(self) -> Result<(), ProcessError> {
        tokio::signal::ctrl_c()
            .await
            .map_err(ProcessError::Signal)?;
        tracing::info!("shutdown signal received");
        self.shutdown.cancel();
        self.admin_task.await?.map_err(ProcessError::AdminServe)
    }
}
