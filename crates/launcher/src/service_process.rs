//! Owns one server child and the lifetime of its captured output.

use std::{
    io,
    path::Path,
    process::{ExitStatus, Stdio},
    time::{Duration, Instant},
};

use anyhow::{Context, bail};
use infra::{
    admin::{STOP_ENV, StopToken},
    ready::ReadyLine,
};
use tokio::{
    process::{Child, Command},
    sync::oneshot,
    time::timeout,
};

use crate::{environment::Environment, output::Output, topology::ServiceConfig};

#[derive(Debug, Clone, Copy)]
pub(crate) struct ProcessId(u32);

impl std::fmt::Display for ProcessId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug)]
pub(crate) struct ServiceProcess {
    child: Child,
    token: StopToken,
    started: Instant,
    stdout: Output,
    stderr: Output,
}

impl ServiceProcess {
    pub(crate) fn start(
        service: &ServiceConfig,
        bin_dir: &Path,
        env: &Environment,
    ) -> anyhow::Result<(Self, oneshot::Receiver<ReadyLine>)> {
        let executable = bin_dir.join(format!(
            "{}{}",
            service.binary(),
            std::env::consts::EXE_SUFFIX
        ));
        let mut cmd = Command::new(&executable);
        #[cfg(windows)]
        // CREATE_NO_WINDOW keeps the parent's Ctrl+C from reaching the child.
        cmd.creation_flags(0x0800_0000);
        #[cfg(unix)]
        cmd.process_group(0);
        env.apply(&mut cmd, service.binary());
        let token = StopToken::generate()?;
        let mut child = cmd
            .arg("--config")
            .arg(service.config())
            .args(service.args())
            .env(STOP_ENV, std::str::from_utf8(token.env_value())?)
            .stdin(Stdio::null())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("spawning {}", executable.display()))?;
        let (send, receive) = oneshot::channel();
        let stdout = Output::read(
            child.stdout.take().context("missing stdout pipe")?,
            Some(send),
        );
        let stderr = Output::read(child.stderr.take().context("missing stderr pipe")?, None);
        Ok((
            Self {
                child,
                token,
                started: Instant::now(),
                stdout,
                stderr,
            },
            receive,
        ))
    }

    pub(crate) fn pid(&self) -> Option<ProcessId> {
        self.child.id().map(ProcessId)
    }

    pub(crate) fn started(&self) -> Instant {
        self.started
    }

    pub(crate) fn recent(&self) -> Vec<String> {
        self.stdout
            .recent()
            .into_iter()
            .chain(self.stderr.recent())
            .collect()
    }

    pub(crate) async fn wait_ready(
        &mut self,
        ready: oneshot::Receiver<ReadyLine>,
        limit: Duration,
    ) -> anyhow::Result<ReadyLine> {
        timeout(limit, async {
            tokio::select! {
                biased;
                status = self.child.wait() => bail!("exited before readiness ({})", status?),
                line = ready => line.context("stdout closed before readiness"),
            }
        })
        .await
        .context("startup readiness timed out")?
    }

    pub(crate) fn try_exit(&mut self) -> io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    pub(crate) async fn stop(
        &mut self,
        ready: ReadyLine,
        client: &reqwest::Client,
        limit: Duration,
    ) -> anyhow::Result<ExitStatus> {
        let admin = ready
            .addr("admin")
            .context("missing announced admin port")?;
        let result = timeout(limit, async {
            let (request, status) =
                tokio::join!(self.token.request(client, admin), self.child.wait());
            let status = status?;
            request?;
            Ok::<_, anyhow::Error>(status)
        })
        .await;
        match result {
            Ok(Ok(status)) => Ok(status),
            Ok(Err(error)) => {
                self.kill().await?;
                Err(error.context("graceful shutdown failed"))
            }
            Err(error) => {
                self.kill().await?;
                Err(anyhow::Error::new(error).context("graceful shutdown timed out; child killed"))
            }
        }
    }

    pub(crate) async fn kill(&mut self) -> io::Result<ExitStatus> {
        self.child.kill().await?;
        self.child.wait().await
    }

    pub(crate) async fn drain(&mut self) -> anyhow::Result<()> {
        timeout(Duration::from_secs(2), async {
            tokio::try_join!(self.stdout.finish(), self.stderr.finish())
        })
        .await??;
        Ok(())
    }
}
