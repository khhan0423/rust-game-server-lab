//! The owner of the local service cluster. Commands are serialized by its task.

#[cfg(test)]
pub(crate) mod tests;

use std::{path::PathBuf, time::Duration};

use anyhow::{Context, ensure};
use infra::shutdown::Shutdown;
use tokio::sync::{mpsc, oneshot, watch};

use crate::{
    environment::Environment,
    service::{Service, Snapshot},
    topology::Topology,
};

pub(crate) enum Operation {
    StartAll(Option<mpsc::Sender<Snapshot>>),
    StopAll,
    Start(String),
    Stop(String),
    Restart(String),
    Kill(String),
}

impl Operation {
    pub(crate) fn submit(self, commands: &mpsc::Sender<Command>) -> Result<(), &'static str> {
        use mpsc::error::TrySendError;

        let (operation, service) = match &self {
            Self::StartAll(_) => ("start_all", "all"),
            Self::StopAll => ("stop_all", "all"),
            Self::Start(name) => ("start", name.as_str()),
            Self::Stop(name) => ("stop", name.as_str()),
            Self::Restart(name) => ("restart", name.as_str()),
            Self::Kill(name) => ("kill", name.as_str()),
        };
        tracing::info!(operation, service, "launcher command requested");
        // Reserve first so the log fields can still borrow the operation on failure.
        let error = match commands.try_reserve() {
            Ok(permit) => {
                permit.send(Command {
                    operation: self,
                    reply: None,
                });
                return Ok(());
            }
            Err(TrySendError::Full(())) => "Supervisor command queue is full. Try again shortly.",
            Err(TrySendError::Closed(())) => "Supervisor is offline. Relaunch to send commands.",
        };
        tracing::warn!(
            operation,
            service,
            error,
            "launcher command could not be sent"
        );
        Err(error)
    }
}

pub(crate) struct Command {
    operation: Operation,
    reply: Option<oneshot::Sender<anyhow::Result<()>>>,
}

impl Command {
    pub(crate) fn with_reply(
        operation: Operation,
    ) -> (Self, oneshot::Receiver<anyhow::Result<()>>) {
        let (reply, receive) = oneshot::channel();
        (
            Self {
                operation,
                reply: Some(reply),
            },
            receive,
        )
    }
}

pub(crate) struct Supervisor {
    series: Vec<Service>,
    bin_dir: PathBuf,
    env: Environment,
    ready_limit: Duration,
    stop_limit: Duration,
    client: reqwest::Client,
    snapshots: watch::Sender<Vec<Snapshot>>,
}

impl Supervisor {
    pub(crate) fn new(
        topology: Topology,
    ) -> anyhow::Result<(Self, watch::Receiver<Vec<Snapshot>>)> {
        let env = Environment::load(topology.env_file())?;
        let bin_dir = topology.bin_dir().to_owned();
        let ready_limit = topology.ready_timeout();
        let stop_limit = topology.stop_timeout();
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .build()?;
        let series = topology
            .into_services()
            .into_iter()
            .map(Service::new)
            .collect::<Vec<_>>();
        let (snapshots, receive) = watch::channel(series.iter().map(Service::snapshot).collect());
        Ok((
            Self {
                series,
                bin_dir,
                env,
                ready_limit,
                stop_limit,
                client,
                snapshots,
            },
            receive,
        ))
    }

    pub(crate) async fn run(
        mut self,
        mut commands: mpsc::Receiver<Command>,
        shutdown: Shutdown,
    ) -> anyhow::Result<()> {
        let mut tick = tokio::time::interval(Duration::from_millis(200));
        tick.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                biased;
                () = shutdown.clone().wait() => break,
                command = commands.recv() => {
                    let Some(command) = command else { break };
                    let result = tokio::select! {
                        () = shutdown.clone().wait() => break,
                        result = self.apply(command.operation) => result,
                    };
                    if let Some(reply) = command.reply {
                        // A closed requester no longer needs the result.
                        let _ = reply.send(result);
                    } else if let Err(error) = result {
                        tracing::warn!(%error, "service command failed");
                    }
                }
                _ = tick.tick() => {
                    for service in &mut self.series {
                        if let Err(error) = service.observe_exit().await { service.fail(&error).await; }
                    }
                    self.publish();
                }
            }
        }
        self.stop_all().await
    }

    fn publish(&self) {
        self.snapshots
            .send_replace(self.series.iter().map(Service::snapshot).collect());
    }

    fn index(&self, name: &str) -> anyhow::Result<usize> {
        self.series
            .iter()
            .position(|service| service.config().name() == name)
            .with_context(|| format!("unknown service {name}"))
    }

    async fn apply(&mut self, operation: Operation) -> anyhow::Result<()> {
        match operation {
            Operation::StartAll(progress) => {
                for index in 0..self.series.len() {
                    self.start(index).await?;
                    if let Some(progress) = &progress {
                        progress
                            .send(self.series[index].snapshot())
                            .await
                            .context("startup reporter closed")?;
                    }
                }
                Ok(())
            }
            Operation::StopAll => self.stop_all().await,
            Operation::Start(name) => self.start(self.index(&name)?).await,
            Operation::Stop(name) => self.stop(self.index(&name)?).await,
            Operation::Restart(name) => {
                let index = self.index(&name)?;
                self.stop(index).await?;
                self.start(index).await
            }
            Operation::Kill(name) => {
                let index = self.index(&name)?;
                let result = self.series[index]
                    .kill()
                    .await
                    .with_context(|| format!("killing {name}"));
                if let Err(error) = &result {
                    self.series[index].fail(error).await;
                }
                self.publish();
                result
            }
        }
    }

    async fn start(&mut self, index: usize) -> anyhow::Result<()> {
        if self.series[index].ready().is_some() {
            return Ok(());
        }
        let result = self.start_and_wait(index).await;
        let name = self.series[index].config().name();
        let result = result.with_context(|| format!("starting {name}"));
        if let Err(error) = &result {
            self.series[index].fail(error).await;
        }
        self.publish();
        result
    }

    async fn start_and_wait(&mut self, index: usize) -> anyhow::Result<()> {
        for dep in self.series[index].config().dependencies() {
            ensure!(
                self.series[self.index(dep)?].ready().is_some(),
                "dependency {dep} is not ready"
            );
        }
        let started = self.series[index].start(&self.bin_dir, &self.env)?;
        self.publish();
        self.series[index]
            .wait_ready(started, self.ready_limit)
            .await?;
        Ok(())
    }

    async fn stop(&mut self, index: usize) -> anyhow::Result<()> {
        let Some(previous) = self.series[index].begin_stop() else {
            return Ok(());
        };
        self.publish();
        let result = self.series[index]
            .stop(previous, &self.client, self.stop_limit)
            .await
            .with_context(|| format!("stopping {}", self.series[index].config().name()));
        if let Err(error) = &result {
            self.series[index].fail(error).await;
        }
        self.publish();
        result
    }

    async fn stop_all(&mut self) -> anyhow::Result<()> {
        let mut result = Ok(());
        for index in (0..self.series.len()).rev() {
            result = result.and(self.stop(index).await);
        }
        result
    }
}
