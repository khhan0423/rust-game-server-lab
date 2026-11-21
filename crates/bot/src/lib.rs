//! Bounded headless Gateway scenarios.

mod session;
mod summary;

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use clap::{Args, ValueEnum};
use tokio::task::JoinSet;
use tokio::time::{Instant, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

pub use summary::{Latency, Summary};

const MAX_ACTIVE: usize = 16_384;

/// Gateway traffic to generate after opening each connection.
#[derive(Debug, Clone, Copy, ValueEnum)]
pub enum Scenario {
    /// Wait for the server to close or the local hold time to run out.
    ConnectHold,
    /// Send one opaque Auth token and wait for the response.
    ConnectAuth,
}

/// Settings for a single load run.
#[derive(Debug, Args)]
pub struct RunConfig {
    /// Gateway client listener address.
    #[arg(long, default_value = "127.0.0.1:18400")]
    pub gateway: SocketAddr,
    /// Number of connection attempts.
    #[arg(long, default_value_t = 1, value_parser = clap::value_parser!(u32).range(1..))]
    pub connections: u32,
    /// Maximum connection attempts started per second.
    #[arg(long, default_value_t = 100, value_parser = clap::value_parser!(u32).range(1..=1_000_000))]
    pub ramp_per_sec: u32,
    /// Maximum time to wait after TCP establishment.
    #[arg(long, default_value_t = 15, value_parser = clap::value_parser!(u32).range(1..))]
    pub hold_secs: u32,
    /// Require this many established TCP connections.
    #[arg(long)]
    pub expect_established: Option<u32>,
    /// Require every established connection to close with this reason.
    #[arg(long)]
    pub expect_close_reason: Option<String>,
    /// Connection scenario.
    pub scenario: Scenario,
}

/// Drives the selected scenario until all attempts finish or `stop` is cancelled.
pub async fn run(config: &RunConfig, stop: &CancellationToken) -> anyhow::Result<Summary> {
    anyhow::ensure!(
        config.ramp_per_sec > 0 && config.ramp_per_sec <= 1_000_000,
        "invalid connection ramp"
    );
    let start = Instant::now();
    let mut summary = summary::Collector::new()?;
    let mut jobs = JoinSet::new();
    let established = Arc::new(AtomicU32::new(0));
    // Batch launches against an absolute rate; Windows timers can round short sleeps to 15.6 ms.
    let tick = Duration::from_millis(16);
    let mut launch = tokio::time::interval_at(start + tick, tick);
    launch.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut last_launch = start;
    let mut credit = 0_u128;
    let mut progress =
        tokio::time::interval_at(start + Duration::from_secs(1), Duration::from_secs(1));
    progress.set_missed_tick_behavior(MissedTickBehavior::Skip);
    let mut interrupted = false;
    while !jobs.is_empty() || (!interrupted && summary.attempted < config.connections) {
        tokio::select! {
            biased;
            () = stop.cancelled(), if !interrupted => interrupted = true,
            _ = progress.tick() => tracing::info!(
                opened = summary.attempted, established = established.load(Ordering::Relaxed),
                failed = summary.failed_to_connect, closed = summary.closed(), "bot progress"
            ),
            Some(result) = jobs.join_next(), if !jobs.is_empty() => {
                match result {
                    Ok(outcome) => {
                        if let Err(error) = summary.record(outcome) {
                            jobs.shutdown().await;
                            return Err(error);
                        }
                    }
                    Err(error) => {
                        jobs.shutdown().await;
                        return Err(error.into());
                    }
                }
            }
            _ = launch.tick(), if !interrupted && summary.attempted < config.connections => {
                let now = Instant::now();
                let elapsed = now.duration_since(last_launch).min(Duration::from_millis(100));
                last_launch = now;
                credit += elapsed.as_nanos() * u128::from(config.ramp_per_sec);
                let due = credit / 1_000_000_000;
                credit %= 1_000_000_000;
                let remaining = u128::from(config.connections - summary.attempted);
                let available = (MAX_ACTIVE - jobs.len()) as u128;
                for _ in 0..due.min(remaining).min(available) {
                    summary.attempted += 1;
                    let job = session::Job { gateway: config.gateway, scenario: config.scenario,
                        hold: Duration::from_secs(u64::from(config.hold_secs)), established: Arc::clone(&established) };
                    jobs.spawn(session::connect(job, stop.clone()));
                }
            }
        }
    }
    Ok(summary.finish(start.elapsed(), interrupted))
}
