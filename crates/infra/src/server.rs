//! Process lifecycle shared by independent servers.

use std::{fmt::Display, io, net::SocketAddr};

use axum::Router;
use tokio::task::{JoinError, JoinSet};

use crate::{
    http::HttpListener,
    process::{Process, ProcessConfig, ProcessError},
    shutdown::Shutdown,
};

/// Service listeners owned by one process, including its admin port.
#[derive(Debug)]
pub struct Server {
    process: Process,
    tasks: JoinSet<io::Result<()>>,
    addresses: Vec<(&'static str, SocketAddr)>,
}

impl Server {
    /// Shutdown handle shared by this process's workers.
    pub fn shutdown(&self) -> Shutdown {
        self.process.shutdown_signal()
    }

    /// Registers a bound port and runs its listener as a supervised task.
    pub fn spawn(
        &mut self,
        name: &'static str,
        addr: SocketAddr,
        task: impl Future<Output = io::Result<()>> + Send + 'static,
    ) {
        self.addresses.push((name, addr));
        self.tasks.spawn(async move {
            task.await
                .map_err(|error| io::Error::new(error.kind(), format!("{name} listener: {error}")))
        });
    }

    /// Serves a bound HTTP port with the process's shutdown signal.
    pub fn spawn_http(&mut self, listener: HttpListener, router: Router) {
        self.spawn(
            listener.name(),
            listener.local_addr(),
            listener.serve(router, self.shutdown()),
        );
    }

    async fn supervise(&mut self) -> Result<(), ProcessError> {
        self.process.mark_ready_with(&self.addresses)?;
        tracing::info!(listeners = ?self.addresses, "server ready");
        tokio::select! {
            signal = Shutdown::signal() => signal.map_err(ProcessError::Signal),
            Some(task) = self.tasks.join_next() => {
                task_result(task)?;
                Err(ProcessError::Service(io::Error::other("service stopped before shutdown")))
            }
        }
    }

    async fn stop(&mut self) -> Result<(), ProcessError> {
        self.process.stop();
        let mut result = Ok(());
        while let Some(task) = self.tasks.join_next().await {
            result = result.and(task_result(task));
        }
        result
    }
}

fn task_result(task: Result<io::Result<()>, JoinError>) -> Result<(), ProcessError> {
    task.map_err(ProcessError::ServiceTask)?
        .map_err(ProcessError::Service)
}

/// Starts the admin port, runs the startup barrier, supervises listeners and drains them.
/// A successful barrier supplies a finalizer for resources that outlive all listeners.
pub fn run<E, F>(
    config: &ProcessConfig,
    start: impl AsyncFnOnce(&mut Server) -> Result<F, E>,
) -> Result<(), E>
where
    E: From<ProcessError> + From<io::Error> + Display,
    F: Future<Output = ()>,
{
    config.runtime().build()?.block_on(async {
        let mut tasks = JoinSet::new();
        let process = Process::start(config, &mut tasks).await?;
        let mut server = Server {
            process,
            tasks,
            addresses: Vec::new(),
        };
        match start(&mut server).await {
            Ok(finish) => {
                let service = server.supervise().await;
                let stopped = server.stop().await;
                finish.await;
                service.and(stopped)?;
                Ok(())
            }
            Err(error) => {
                tracing::error!(error = format!("{error:#}"), "startup failed");
                if let Err(stop) = server.stop().await {
                    tracing::error!(%stop, "startup cleanup failed");
                }
                Err(error)
            }
        }
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, thiserror::Error)]
    enum TestError {
        #[error(transparent)]
        Process(#[from] ProcessError),
        #[error(transparent)]
        Io(#[from] io::Error),
    }

    #[test]
    fn listener_failure_withdraws_readiness_and_drains_tasks() -> Result<(), toml::de::Error> {
        let config = toml::from_str(
            "admin_addr='127.0.0.1:0'\nname='game'\nenvironment='development'\nlog_level='info'\nlog_root='../../target/tmp/supervisor-test'\nworker_threads=1",
        )?;
        let result: Result<(), TestError> = run(&config, async |server| {
            let ready = server.process.readiness();
            let observed = ready.clone();
            server.spawn(
                "failing",
                SocketAddr::from(([127, 0, 0, 1], 1)),
                async move {
                    tokio::time::timeout(std::time::Duration::from_secs(3), async {
                        while !observed.is_ready() {
                            tokio::task::yield_now().await;
                        }
                    })
                    .await
                    .map_err(io::Error::other)?;
                    Err(io::Error::other("listener failed"))
                },
            );
            Ok(async move {
                assert!(!ready.is_ready());
            })
        });
        assert!(
            matches!(result, Err(TestError::Process(ProcessError::Service(error))) if error.to_string().contains("listener failed"))
        );
        Ok(())
    }
}
