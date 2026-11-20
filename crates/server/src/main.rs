//! Game server binary.
//!
//! For now it only goes as far as the startup barrier: it signals ready only after config, logging, the admin port,
//! the Game DB connection, and the schema check have all finished (design 01 section 10.0).

use std::path::PathBuf;
use std::time::Duration;

use anyhow::Context;
use clap::{Parser, Subcommand};
use infra::config::Secret;
use infra::process::{Process, ProcessConfig};
use serde::Deserialize;

/// Environment variable holding the Game DB connection string. It is a secret, so it is not kept in the config file (design 08 section 8).
const GAME_DB_URL_ENV: &str = "GAME_DB_URL";

#[derive(Debug, Parser)]
struct Cli {
    /// Path to the config file (TOML)
    #[arg(long)]
    config: PathBuf,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Start the server
    Run,
    /// Apply migrations to the Game DB and exit
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

/// Preparation that must finish before accepting players. If any step fails, startup fails.
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
            // Record the failed step in the operations log. The log writer is flushed when `process` is dropped.
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
