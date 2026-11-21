//! Common startup and shutdown of server processes: logging, metrics, admin port, ready notification.

use std::io::Write;
use std::net::SocketAddr;
use std::time::Duration;

use crate::shutdown::Shutdown;
use metrics_exporter_prometheus::{BuildError, PrometheusBuilder};
use serde::Deserialize;
use tokio::task::{JoinError, JoinSet};

use crate::admin::{
    AdminListener, RUNTIME_ALIVE_TASKS, RUNTIME_GLOBAL_QUEUE_DEPTH, RUNTIME_WORKERS, Readiness,
    SERVER_READY,
};
use crate::logging::{self, LogConfig, LogError, WorkerGuard};
use crate::runtime::RuntimeConfig;

/// Settings that every server process keeps in the `[process]` table of its config file.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ProcessConfig {
    /// Admin port address. Use only private network addresses.
    admin_addr: SocketAddr,
    /// Process identity and operations log settings.
    #[serde(flatten)]
    pub log: LogConfig,
    /// Runtime worker configuration.
    #[serde(flatten)]
    runtime: RuntimeConfig,
}

impl ProcessConfig {
    /// Returns the settings for the runtime that hosts this server.
    pub fn runtime(&self) -> &RuntimeConfig {
        &self.runtime
    }
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
    /// A supervised service failed.
    #[error("{name} service failed")]
    Service {
        /// Name of the supervised service.
        name: &'static str,
        /// Error returned by the service.
        #[source]
        source: std::io::Error,
    },
    /// A supervised task terminated abnormally.
    #[error("service task stopped unexpectedly")]
    ServiceTask(#[source] JoinError),
    /// The ready notification line could not be written to standard output.
    #[error("cannot announce readiness")]
    Announce(#[source] std::io::Error),
    /// Cannot wait for the shutdown signal.
    #[error("cannot listen for shutdown signal")]
    Signal(#[source] std::io::Error),
}

/// The common part of a running server process.
#[derive(Debug)]
pub(crate) struct Process {
    readiness: Readiness,
    admin_addr: SocketAddr,
    shutdown: Shutdown,
}

impl Process {
    /// Turns on metrics and opens the admin port. Readiness starts false.
    pub(crate) async fn start(
        config: &ProcessConfig,
        tasks: &mut JoinSet<Result<(), ProcessError>>,
    ) -> Result<Self, ProcessError> {
        let metrics = PrometheusBuilder::new().install_recorder()?;
        let listener = AdminListener::bind(config.admin_addr)
            .await
            .map_err(|source| ProcessError::AdminBind {
                addr: config.admin_addr,
                source,
            })?;
        let admin_addr = listener.local_addr();
        let readiness = Readiness::default();
        let shutdown = Shutdown::default();
        let runtime = tokio::runtime::Handle::current().metrics();
        record_runtime_metrics(&runtime);
        let stop = shutdown.clone();
        tasks.spawn(async move {
            sample_runtime(runtime, stop).await;
            Ok(())
        });
        let admin = listener.serve(readiness.clone(), metrics, shutdown.clone());
        tasks.spawn(async move {
            admin.await.map_err(|source| ProcessError::Service {
                name: "admin",
                source,
            })
        });
        Ok(Self {
            readiness,
            admin_addr,
            shutdown,
        })
    }

    pub(crate) fn init_log(config: &ProcessConfig) -> Result<WorkerGuard, ProcessError> {
        Ok(logging::init(&config.log)?)
    }

    /// Announces readiness with additional named listener addresses on the same line.
    pub(crate) fn mark_ready_with(
        &self,
        listeners: &[(&str, SocketAddr)],
    ) -> Result<(), ProcessError> {
        let line = crate::ready::ReadyLine::new(self.admin_addr, listeners)
            .map_err(ProcessError::Announce)?;
        self.readiness.set_ready();
        metrics::gauge!(SERVER_READY).set(1.0);
        let mut stdout = std::io::stdout().lock();
        writeln!(stdout, "{line}")
            .and_then(|()| stdout.flush())
            .map_err(ProcessError::Announce)
    }

    /// Resolves when the process starts graceful shutdown.
    pub(crate) fn shutdown_signal(&self) -> Shutdown {
        self.shutdown.clone()
    }

    /// Clears readiness before asking all supervised workers to drain.
    pub(crate) fn stop(&self) {
        self.readiness.set_not_ready();
        metrics::gauge!(SERVER_READY).set(0.0);
        self.shutdown.cancel();
    }

    #[cfg(test)]
    pub(crate) fn readiness(&self) -> Readiness {
        self.readiness.clone()
    }
}

async fn sample_runtime(runtime: tokio::runtime::RuntimeMetrics, shutdown: Shutdown) {
    let mut interval = tokio::time::interval(Duration::from_secs(1));
    interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
    let cancelled = shutdown.wait();
    tokio::pin!(cancelled);
    loop {
        tokio::select! {
            biased;
            () = &mut cancelled => return,
            _ = interval.tick() => record_runtime_metrics(&runtime),
        }
    }
}

#[expect(
    clippy::cast_precision_loss,
    reason = "A runtime cannot hold 2^53 tasks or workers"
)]
fn record_runtime_metrics(runtime: &tokio::runtime::RuntimeMetrics) {
    metrics::gauge!(RUNTIME_WORKERS).set(runtime.num_workers() as f64);
    metrics::gauge!(RUNTIME_ALIVE_TASKS).set(runtime.num_alive_tasks() as f64);
    metrics::gauge!(RUNTIME_GLOBAL_QUEUE_DEPTH).set(runtime.global_queue_depth() as f64);
}
