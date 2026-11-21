//! Transitions and public snapshots for one supervised service.

use std::{
    fmt,
    path::Path,
    process::ExitStatus,
    time::{Duration, Instant},
};

use anyhow::{Context, ensure};
use infra::ready::ReadyLine;
use tokio::sync::oneshot;

use crate::{
    environment::Environment,
    service_process::{ProcessId, ServiceProcess},
    topology::ServiceConfig,
};

#[derive(Clone, Debug)]
pub(crate) enum State {
    Stopped,
    Starting,
    Ready(ReadyLine),
    Stopping,
    Exited(ExitStatus),
    Failed(String),
}

impl fmt::Display for State {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Stopped => f.write_str("Stopped"),
            Self::Starting => f.write_str("Starting"),
            Self::Ready(_) => f.write_str("Ready"),
            Self::Stopping => f.write_str("Stopping"),
            Self::Exited(status) => write!(f, "Exited ({status})"),
            Self::Failed(error) => write!(f, "Failed: {error}"),
        }
    }
}

#[derive(Clone, Debug)]
pub(crate) struct Snapshot {
    name: String,
    state: State,
    pid: Option<ProcessId>,
    started: Option<Instant>,
    logs: Vec<String>,
}

impl Snapshot {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }
    pub(crate) fn state(&self) -> &State {
        &self.state
    }
    pub(crate) fn pid(&self) -> Option<ProcessId> {
        self.pid
    }
    pub(crate) fn uptime(&self) -> Option<Duration> {
        self.started.map(|start| start.elapsed())
    }
    pub(crate) fn logs(&self) -> &[String] {
        &self.logs
    }
}

#[derive(Debug)]
pub(crate) struct Service {
    config: ServiceConfig,
    process: Option<ServiceProcess>,
    state: State,
    recent: Vec<String>,
}

impl Service {
    pub(crate) fn new(config: ServiceConfig) -> Self {
        Self {
            config,
            process: None,
            state: State::Stopped,
            recent: Vec::new(),
        }
    }

    pub(crate) fn config(&self) -> &ServiceConfig {
        &self.config
    }

    fn transition(&mut self, state: State) -> State {
        tracing::info!(service = self.config.name(), from = %self.state, to = %state, "service state changed");
        std::mem::replace(&mut self.state, state)
    }
    pub(crate) fn ready(&self) -> Option<&ReadyLine> {
        match &self.state {
            State::Ready(line) => Some(line),
            State::Stopped
            | State::Starting
            | State::Stopping
            | State::Exited(_)
            | State::Failed(_) => None,
        }
    }

    pub(crate) fn snapshot(&self) -> Snapshot {
        Snapshot {
            name: self.config.name().to_owned(),
            state: self.state.clone(),
            pid: self.process.as_ref().and_then(ServiceProcess::pid),
            started: self.process.as_ref().map(ServiceProcess::started),
            logs: self
                .process
                .as_ref()
                .map_or_else(|| self.recent.clone(), ServiceProcess::recent),
        }
    }

    pub(crate) fn start(
        &mut self,
        bin_dir: &Path,
        env: &Environment,
    ) -> anyhow::Result<oneshot::Receiver<ReadyLine>> {
        ensure!(self.process.is_none(), "service already started");
        let (process, ready) = ServiceProcess::start(&self.config, bin_dir, env)?;
        self.process = Some(process);
        self.transition(State::Starting);
        Ok(ready)
    }

    pub(crate) async fn wait_ready(
        &mut self,
        ready: oneshot::Receiver<ReadyLine>,
        limit: Duration,
    ) -> anyhow::Result<()> {
        let line = self
            .process
            .as_mut()
            .context("missing starting process")?
            .wait_ready(ready, limit)
            .await?;
        self.transition(State::Ready(line));
        Ok(())
    }

    pub(crate) async fn fail(&mut self, error: &anyhow::Error) {
        if let Err(cleanup) = self.kill().await {
            tracing::error!(service = self.config.name(), error = %cleanup, "cannot clean up failed service");
        }
        self.transition(State::Failed(format!("{error:#}")));
    }

    pub(crate) fn begin_stop(&mut self) -> Option<State> {
        (!matches!(self.state, State::Stopped)).then(|| self.transition(State::Stopping))
    }

    pub(crate) async fn stop(
        &mut self,
        previous: State,
        client: &reqwest::Client,
        limit: Duration,
    ) -> anyhow::Result<()> {
        if let Some(mut process) = self.process.take() {
            let result = match previous {
                State::Ready(line) => process.stop(line, client, limit).await,
                State::Starting
                | State::Stopping
                | State::Stopped
                | State::Exited(_)
                | State::Failed(_) => Ok(process.kill().await?),
            };
            self.collect_output(&mut process).await;
            let status = result?;
            ensure!(status.success(), "service stopped with {status}");
        }
        self.transition(State::Stopped);
        Ok(())
    }

    pub(crate) async fn kill(&mut self) -> anyhow::Result<()> {
        if let Some(mut process) = self.process.take() {
            let status = process.kill().await?;
            self.collect_output(&mut process).await;
            self.transition(State::Exited(status));
        }
        Ok(())
    }

    pub(crate) async fn observe_exit(&mut self) -> anyhow::Result<()> {
        if let Some(process) = self.process.as_mut()
            && let Some(status) = process.try_exit()?
            && let Some(mut process) = self.process.take()
        {
            self.collect_output(&mut process).await;
            self.transition(State::Exited(status));
        }
        Ok(())
    }

    async fn collect_output(&mut self, process: &mut ServiceProcess) {
        if let Err(error) = process.drain().await {
            tracing::warn!(service = self.config.name(), %error, "child output did not drain");
        }
        self.recent = process.recent();
    }
}
