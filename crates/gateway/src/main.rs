//! Gateway binary (design 02).

mod accept;
mod config;
mod connection;
mod writer;

use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use infra::process::Process;
use tokio::net::TcpListener;

use config::Config;

#[derive(Debug, Parser)]
struct Cli {
    /// Path to the config file (TOML)
    #[arg(long)]
    config: PathBuf,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config: Config = infra::config::load(&cli.config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(run(&config))
}

async fn run(config: &Config) -> anyhow::Result<()> {
    anyhow::ensure!(
        config.client.write_queue_capacity.get() <= tokio::sync::Semaphore::MAX_PERMITS,
        "client write_queue_capacity exceeds Tokio's channel capacity limit"
    );
    let listener = TcpListener::bind(config.client.listen_addr)
        .await
        .with_context(|| format!("binding client listener {}", config.client.listen_addr))?;
    let client = listener
        .local_addr()
        .context("reading client listener address")?;
    let process = Process::start(&config.process).await?;
    accept::initialize_metrics();
    if let Err(error) = process.mark_ready_with(&[("client", client)]) {
        process.shutdown().await?;
        return Err(error.into());
    }
    tracing::info!(%client, "gateway ready");
    let shutdown = process.shutdown_signal();
    let (process, ()) = tokio::join!(
        process.run_until_shutdown(),
        accept::run(listener, config.client, shutdown),
    );
    process?;
    Ok(())
}
