//! Common startup and shutdown of server processes: logging, metrics, admin port, ready notification.

use std::io::Write;
use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use metrics_exporter_prometheus::{BuildError, PrometheusBuilder};
use serde::Deserialize;
use tokio::task::{JoinError, JoinHandle};
use tokio_util::sync::CancellationToken;

use crate::admin::{AdminListener, Readiness};
use crate::logging::{self, LogError, LogLevel, WorkerGuard};

/// Settings that every server process keeps in the `[process]` table of its config file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessConfig {
    /// Admin port address. Use only private network addresses.
    pub admin_addr: SocketAddr,
    /// Operations log directory.
    pub log_dir: PathBuf,
    /// Operations log level.
    pub log_level: LogLevel,
}

/// Why the common process part failed.
#[derive(Debug, thiserror::Error)]
pub enum ProcessError {
    /// Logging could not be started.
    #[error("cannot start logging")]
    Log(#[from] LogError),
    /// The metrics recorder could not be installed.
    #[error("cannot install metrics recorder")]
    Metrics(#[from] BuildError),
    /// The admin port could not be opened.
    #[error("cannot bind admin port {addr}")]
    AdminBind {
        /// The address that was being opened.
        addr: SocketAddr,
        /// Cause.
        #[source]
        source: std::io::Error,
    },
    /// The admin port failed while handling requests.
    #[error("admin port failed")]
    AdminServe(#[source] std::io::Error),
    /// The admin port task terminated abnormally.
    #[error("admin task stopped unexpectedly")]
    AdminTask(#[from] JoinError),
    /// The runtime sampler task terminated abnormally.
    #[error("runtime sampler stopped unexpectedly")]
    RuntimeSampler(#[source] JoinError),
    /// The ready notification line could not be written to standard output.
    #[error("cannot announce readiness")]
    Announce(#[source] std::io::Error),
    /// Cannot wait for the shutdown signal.
    #[error("cannot listen for shutdown signal")]
    Signal(#[source] std::io::Error),
}

/// The common part of a running server process.
#[derive(Debug)]
pub struct Process {
    readiness: Readiness,
    admin_addr: SocketAddr,
    admin_task: JoinHandle<std::io::Result<()>>,
    runtime_sampler: JoinHandle<()>,
    shutdown: CancellationToken,
    _log: WorkerGuard,
}

impl Process {
    /// Turns on logging and metrics and opens the admin port. At this point only `/health/live` returns 200.
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
        let runtime = tokio::runtime::Handle::current().metrics();
        record_runtime_metrics(&runtime);
        let runtime_sampler = tokio::spawn(sample_runtime(runtime, shutdown.clone()));
        let admin_task = tokio::spawn(listener.serve(readiness.clone(), metrics, shutdown.clone()));
        Ok(Self {
            readiness,
            admin_addr,
            admin_task,
            runtime_sampler,
            shutdown,
            _log: log,
        })
    }

    /// Signals that the startup barrier has finished. Turns on `/health/ready` and writes one line
    /// `ready admin=<address>` to standard output for the parent process to read (the launcher and integration tests wait for this line).
    pub fn mark_ready(&self) -> Result<(), ProcessError> {
        self.mark_ready_with(&[])
    }

    /// Announces readiness with additional named listener addresses on the same line.
    pub fn mark_ready_with(&self, listeners: &[(&str, SocketAddr)]) -> Result<(), ProcessError> {
        self.readiness.set_ready();
        metrics::gauge!("server_ready").set(1.0);
        let mut stdout = std::io::stdout().lock();
        write!(stdout, "ready admin={}", self.admin_addr)
            .and_then(|()| {
                for (name, addr) in listeners {
                    write!(stdout, " {name}={addr}")?;
                }
                writeln!(stdout)
            })
            .and_then(|()| stdout.flush())
            .map_err(ProcessError::Announce)
    }

    /// Resolves when the process starts graceful shutdown.
    pub fn shutdown_signal(&self) -> impl Future<Output = ()> + use<> {
        self.shutdown.clone().cancelled_owned()
    }

    /// Waits for the shutdown signal (Ctrl+C), then closes the admin port.
    pub async fn run_until_shutdown(self) -> Result<(), ProcessError> {
        let signal = tokio::signal::ctrl_c().await.map_err(ProcessError::Signal);
        if signal.is_ok() {
            tracing::info!("shutdown signal received");
        }
        self.shutdown().await.and(signal)
    }

    /// Stops the admin port and runtime sampler and notifies service tasks.
    pub async fn shutdown(self) -> Result<(), ProcessError> {
        self.readiness.set_not_ready();
        metrics::gauge!("server_ready").set(0.0);
        self.shutdown.cancel();
        let (admin, sampler) = tokio::join!(self.admin_task, self.runtime_sampler);
        sampler.map_err(ProcessError::RuntimeSampler)?;
        admin?.map_err(ProcessError::AdminServe)
    }
}

async fn sample_runtime(runtime: tokio::runtime::RuntimeMetrics, shutdown: CancellationToken) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    loop {
        tokio::select! {
            biased;
            () = shutdown.cancelled() => return,
            _ = interval.tick() => record_runtime_metrics(&runtime),
        }
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "A runtime cannot hold 2^53 tasks or workers"
)]
fn record_runtime_metrics(runtime: &tokio::runtime::RuntimeMetrics) {
    metrics::gauge!("runtime_workers").set(runtime.num_workers() as f64);
    metrics::gauge!("runtime_alive_tasks").set(runtime.num_alive_tasks() as f64);
    metrics::gauge!("runtime_global_queue_depth").set(runtime.global_queue_depth() as f64);
}
