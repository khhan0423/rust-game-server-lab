//! Gateway 바이너리 (설계 02).
//!
//! 지금은 프로세스 공통 부분만 있다: 설정, 로그, 관리 포트, 준비 통지.

use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;
use infra::process::{Process, ProcessConfig};
use serde::Deserialize;

#[derive(Debug, Parser)]
struct Cli {
    /// 설정 파일(TOML) 경로
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
