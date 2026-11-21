//! Runs the supervisor independently of the caller's event loop.

use std::thread::{self, JoinHandle};

use anyhow::Context;
use infra::shutdown::Shutdown;
use tokio::{
    runtime::Handle,
    sync::{mpsc, watch},
};

use crate::{
    service::Snapshot,
    supervisor::{Command, Supervisor},
    topology::Topology,
};

pub(crate) struct Session {
    shutdown: Shutdown,
    runtime: Handle,
    worker: Option<JoinHandle<anyhow::Result<()>>>,
}

impl Session {
    pub(crate) fn start(
        topology: Topology,
    ) -> anyhow::Result<(Self, mpsc::Sender<Command>, watch::Receiver<Vec<Snapshot>>)> {
        let runtime = topology.process().runtime.build()?;
        let handle = runtime.handle().clone();
        let (supervisor, snapshots) = Supervisor::new(topology)?;
        let (send, receive) = mpsc::channel(16);
        let shutdown = Shutdown::default();
        let stop = shutdown.clone();
        let worker = thread::Builder::new()
            .name("launcher-supervisor".into())
            .spawn(move || runtime.block_on(supervisor.run(receive, stop)))
            .context("starting supervisor thread")?;
        Ok((
            Self {
                shutdown,
                runtime: handle,
                worker: Some(worker),
            },
            send,
            snapshots,
        ))
    }

    pub(crate) fn wait_for<F: Future>(&self, future: F) -> F::Output {
        self.runtime.block_on(future)
    }

    pub(crate) fn finish(&mut self) -> anyhow::Result<()> {
        self.shutdown.cancel();
        if let Some(worker) = self.worker.take() {
            worker
                .join()
                .map_err(|_| anyhow::anyhow!("supervisor thread panicked"))??;
        }
        Ok(())
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        if let Err(error) = self.finish() {
            tracing::error!(%error, "supervisor cleanup failed");
        }
    }
}
