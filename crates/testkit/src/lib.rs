//! Process fixtures used only by workspace integration tests.

use std::{
    io,
    net::SocketAddr,
    path::{Path, PathBuf},
    process::{Output, Stdio},
    time::Duration,
};

use infra::ready::ReadyLine;
use tokio::{
    io::{AsyncBufReadExt, BufReader},
    process::{Child, Command},
    time::timeout,
};

/// Upper bound on an integration process or readiness event.
pub const WAIT: Duration = Duration::from_secs(60);

/// Per-run scratch directory under Cargo's test root. Logs survive test failures.
#[derive(Debug)]
pub struct TestDir(PathBuf);

impl TestDir {
    /// Creates a fresh directory even when the same named test is run concurrently.
    pub async fn new(root: &str, name: &str) -> io::Result<Self> {
        let dir = Path::new(root).join(format!("{name}-{}", uuid::Uuid::now_v7()));
        tokio::fs::create_dir_all(&dir).await?;
        Ok(Self(dir))
    }

    /// Returns the directory containing the fixture's files.
    pub fn path(&self) -> &Path {
        &self.0
    }

    /// Writes a service config and returns the path passed to its executable.
    pub async fn write(&self, file: &str, contents: &str) -> io::Result<PathBuf> {
        let path = self.0.join(file);
        tokio::fs::write(&path, contents).await?;
        Ok(path)
    }
}

/// Resolves another binary built into the same Cargo profile directory.
pub fn sibling_binary(own: &str, name: &str) -> PathBuf {
    Path::new(own).with_file_name(format!("{name}{}", std::env::consts::EXE_SUFFIX))
}

/// Gateway settings varied by the integration tests.
#[derive(Debug)]
pub struct GatewayFixture {
    addr: SocketAddr,
    max: usize,
    timeout_ms: u64,
    queue: usize,
}

impl GatewayFixture {
    /// Starts with the usual bounded write queue; tests may request a smaller queue.
    pub fn new(addr: SocketAddr, max: usize, timeout_ms: u64) -> Self {
        Self {
            addr,
            max,
            timeout_ms,
            queue: 256,
        }
    }

    /// Sets the write queue limit exercised by a test.
    #[must_use]
    pub fn with_queue(mut self, queue: usize) -> Self {
        self.queue = queue;
        self
    }

    /// Constructs the command for a real gateway using a private config.
    pub async fn command(&self, binary: &Path, dir: &TestDir) -> io::Result<Command> {
        let path = dir.write("gateway.toml", &format!(
            "[process]\nname='gateway'\nenvironment='development'\nadmin_addr='127.0.0.1:0'\nlog_root='{}'\nlog_level='info'\n[client]\nlisten_addr='{}'\nmax_connections={}\nauth_timeout_ms={}\nwrite_queue_capacity={}\nlog_connections=true\n",
            dir.path().join("logs").display(), self.addr, self.max, self.timeout_ms, self.queue)).await?;
        let mut command = Command::new(binary);
        command.arg("--config").arg(path);
        Ok(command)
    }
}

/// A test server whose startup barrier has completed. Dropping it kills the child.
#[derive(Debug)]
pub struct ServerProcess {
    child: Child,
    ready: ReadyLine,
}

impl ServerProcess {
    /// Spawns the executable and reads its announcement under a deadline.
    pub async fn spawn(command: &mut Command) -> io::Result<Self> {
        let mut child = command.kill_on_drop(true).stdout(Stdio::piped()).spawn()?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| io::Error::other("missing stdout pipe"))?;
        let text = timeout(WAIT, BufReader::new(stdout).lines().next_line())
            .await??
            .ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "server exited before readiness",
                )
            })?;
        Ok(Self {
            child,
            ready: text.parse()?,
        })
    }

    /// Returns an announced port or fails if it was not published.
    pub fn addr(&self, name: &str) -> io::Result<SocketAddr> {
        self.ready.addr(name).ok_or_else(|| {
            io::Error::new(io::ErrorKind::NotFound, format!("missing listener {name}"))
        })
    }

    /// Kills and reaps the process before a test observes its absence.
    pub async fn stop(&mut self) -> io::Result<()> {
        self.child.kill().await
    }

    /// Runs a migration or a rejected startup to completion and captures both output streams.
    pub async fn finished(command: &mut Command) -> io::Result<Output> {
        timeout(WAIT, command.kill_on_drop(true).output()).await?
    }
}

/// Samples the admin endpoint until all expected metric lines appear together.
pub async fn metrics_until(
    admin: SocketAddr,
    expected: &[&str],
) -> Result<String, Box<dyn std::error::Error + Send + Sync>> {
    let client = reqwest::Client::new();
    let result = timeout(WAIT, async {
        let mut poll = tokio::time::interval(Duration::from_millis(20));
        loop {
            poll.tick().await;
            let metrics = client
                .get(format!("http://{admin}/metrics"))
                .send()
                .await?
                .error_for_status()?
                .text()
                .await?;
            if expected
                .iter()
                .all(|line| metrics.lines().any(|actual| actual == *line))
            {
                return Ok::<_, reqwest::Error>(metrics);
            }
        }
    })
    .await??;
    Ok(result)
}
