//! Game 서버 바이너리.
//!
//! 지금은 기동 배리어까지만 한다: 설정, 로그, 관리 포트, Game DB 연결과 스키마 확인이
//! 모두 끝나야 준비됨을 알린다(설계 01 10.0).

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use infra::config::Secret;
use infra::process::{Process, ProcessConfig};
use serde::Deserialize;

/// Game DB 접속 문자열을 담는 환경 변수. 비밀값이라 설정 파일에 두지 않는다(설계 08 8장).
const GAME_DB_URL_ENV: &str = "GAME_DB_URL";

#[derive(Debug, Parser)]
struct Cli {
    /// 설정 파일(TOML) 경로
    #[arg(long)]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// 서버를 띄운다
    Run,
    /// Game DB에 마이그레이션을 적용하고 끝낸다
    Migrate,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    process: ProcessConfig,
    db: DbConfig,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct DbConfig {
    max_connections: u32,
    acquire_timeout_ms: u64,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config: Config = infra::config::load(&cli.config)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    match cli.command {
        Command::Run => runtime.block_on(run(&config)),
        Command::Migrate => runtime.block_on(migrate(&config.db)),
    }
}

async fn connect_game_db(config: &DbConfig) -> anyhow::Result<db::Pool> {
    let url = Secret::from_env(GAME_DB_URL_ENV)?;
    let settings = db::PoolSettings {
        max_connections: config.max_connections,
        acquire_timeout: Duration::from_millis(config.acquire_timeout_ms),
    };
    db::connect(url.expose(), settings)
        .await
        .context("connecting to game database")
}

/// 손님을 받기 전에 끝나야 하는 준비. 하나라도 실패하면 기동 실패다.
async fn startup_barrier(config: &Config) -> anyhow::Result<db::Pool> {
    let pool = connect_game_db(&config.db).await?;
    db::check_schema(&pool, &db::GAME_MIGRATOR)
        .await
        .context("checking game database schema")?;
    Ok(pool)
}

async fn run(config: &Config) -> anyhow::Result<()> {
    let process = Process::start(&config.process).await?;
    let pool = match startup_barrier(config).await {
        Ok(pool) => pool,
        Err(error) => {
            // 운영 로그에 실패 단계를 남긴다. 로그 쓰기는 `process`가 버려질 때 비워진다.
            tracing::error!(error = format!("{error:#}"), "startup failed");
            return Err(error);
        }
    };
    process.mark_ready()?;
    tracing::info!("game server ready");
    process.run_until_shutdown().await?;
    pool.close().await;
    Ok(())
}

async fn migrate(config: &DbConfig) -> anyhow::Result<()> {
    let pool = connect_game_db(config).await?;
    db::migrate(&pool, &db::GAME_MIGRATOR)
        .await
        .context("applying game database migrations")?;
    pool.close().await;
    Ok(())
}
