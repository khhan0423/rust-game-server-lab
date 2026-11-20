//! Gateway binary (design 02).
//!
//! For now it has only the common process part: config, logging, admin port, ready notification.

use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use infra::process::{Process, ProcessConfig};
use serde::Deserialize;

#[derive(Debug, Parser)]
struct Cli {
    /// Path to the config file (TOML)
    #[arg(long)]
    config: PathBuf,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    process: ProcessConfig,
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
    let process = Process::start(&config.process).await?;
    process.mark_ready()?;
    tracing::info!("gateway ready");
    process.run_until_shutdown().await?;
    Ok(())
}
