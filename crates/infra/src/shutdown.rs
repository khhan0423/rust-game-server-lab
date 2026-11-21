//! Process shutdown signals and cancellation shared by service tasks.

use std::io;

use tokio_util::sync::CancellationToken;

/// Clonable shutdown handle for one process and its listeners.
#[derive(Clone, Debug, Default)]
pub struct Shutdown(CancellationToken);

impl Shutdown {
    /// Returns a token for workers that do not depend on `infra`.
    pub fn token(&self) -> CancellationToken {
        self.0.clone()
    }

    /// Requests shutdown of all workers sharing this handle.
    pub fn cancel(&self) {
        self.0.cancel();
    }

    /// Resolves when shutdown has been requested.
    pub async fn wait(self) {
        self.0.cancelled().await;
    }

    /// Waits for Ctrl+C or, on Unix, SIGTERM from the process supervisor.
    pub async fn signal() -> io::Result<()> {
        #[cfg(unix)]
        {
            use tokio::signal::unix::{SignalKind, signal};
            let mut terminate = signal(SignalKind::terminate())?;
            tokio::select! {
                result = tokio::signal::ctrl_c() => result,
                _ = terminate.recv() => Ok(()),
            }
        }
        #[cfg(not(unix))]
        tokio::signal::ctrl_c().await
    }
}
