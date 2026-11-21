//! Runs the same supervision commands from a terminal.

use std::io::{self, Write};

use anyhow::Context;
use infra::shutdown::Shutdown;
use tokio::sync::mpsc;

use crate::{
    service::State,
    supervisor::{Command, Operation},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, clap::ValueEnum)]
pub(crate) enum Mode {
    /// Start, verify, and stop the configured services.
    Check,
    /// Start and supervise services until interrupted.
    Up,
}

impl Mode {
    pub(crate) fn name(self) -> &'static str {
        match self {
            Self::Check => "check",
            Self::Up => "up",
        }
    }

    pub(crate) async fn run(self, commands: &mpsc::Sender<Command>) -> anyhow::Result<()> {
        let signal = Shutdown::signal();
        tokio::pin!(signal);
        let result = tokio::select! {
            result = Self::start_all(commands) => result,
            interrupted = &mut signal => interrupted.context("listening for shutdown")
                .and(Err(anyhow::anyhow!("interrupted during startup"))),
        };
        match (self, result) {
            (Self::Up, Ok(())) => match write_line(format_args!("RESULT launcher up PASS")) {
                Ok(()) => signal.await.context("listening for shutdown"),
                Err(error) => Err(error.into()),
            },
            (Self::Check, result) | (Self::Up, result @ Err(_)) => result,
        }
    }

    async fn start_all(commands: &mpsc::Sender<Command>) -> anyhow::Result<()> {
        let (progress, mut events) = mpsc::channel(4);
        let (command, done) = Command::with_reply(Operation::StartAll(Some(progress)));
        commands.send(command).await.context("supervisor stopped")?;
        while let Some(event) = events.recv().await {
            if let State::Ready(line) = event.state() {
                write_line(format_args!("{} {line}", event.name()))?;
            }
        }
        done.await.context("supervisor stopped")?
    }
}

pub(crate) fn write_line(line: std::fmt::Arguments<'_>) -> io::Result<()> {
    let mut stdout = io::stdout().lock();
    writeln!(stdout, "{line}")?;
    stdout.flush()
}
