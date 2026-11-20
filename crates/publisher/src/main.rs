//! Publisher guest login and game entry ticket service.

mod api;
mod config;
mod credentials;

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Context;
use clap::{Parser, Subcommand};
use db::login::LOGIN_MIGRATOR;
use infra::config::Secret;
use infra::process::Process;
use tokio::net::TcpListener;

use crate::config::{Config, DbConfig};

const LOGIN_DB_URL_ENV: &str = "LOGIN_DB_URL";
const GAME_API_KEY_ENV: &str = "PUBLISHER_GAME_API_KEY";

#[derive(Debug, Parser)]
struct Cli {
    /// Path to the config file (TOML)
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Serve the guest login and ticket API
    Run,
    /// Apply migrations to the login database and exit
    Migrate,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let path = cli.config.context("--config is required")?;
    let config: Config = infra::config::load(&path)?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    match cli.command {
        Command::Run => runtime.block_on(run(config)),
        Command::Migrate => runtime.block_on(migrate(&config.db)),
    }
}

async fn connect_login_db(config: &DbConfig) -> anyhow::Result<db::Pool> {
    let url = Secret::from_env(LOGIN_DB_URL_ENV)?;
    db::connect(url.expose(), config.settings())
        .await
        .context("connecting to LoginDB")
}

async fn startup_barrier(config: &Config) -> anyhow::Result<(db::Pool, TcpListener, Secret)> {
    let key = Secret::from_env(GAME_API_KEY_ENV)?;
    anyhow::ensure!(!key.expose().is_empty(), "{GAME_API_KEY_ENV} is empty");
    let pool = connect_login_db(&config.db).await?;
    db::check_schema(&pool, &LOGIN_MIGRATOR)
        .await
        .context("checking LoginDB schema")?;
    let listener = TcpListener::bind(config.http.listen_addr)
        .await
        .context("binding publisher API listener")?;
    Ok((pool, listener, key))
}

async fn run(config: Config) -> anyhow::Result<()> {
    let process = Process::start(&config.process).await?;
    let (pool, listener, key) = match startup_barrier(&config).await {
        Ok(ready) => ready,
        Err(error) => {
            tracing::error!(%error, "startup failed");
            return Err(error);
        }
    };
    let addr = listener
        .local_addr()
        .context("reading publisher API address")?;
    let router = api::router(Arc::new(api::AppState {
        pool: pool.clone(),
        games: config
            .games
            .into_iter()
            .map(|(id, game)| (db::login::GameId::new(id), game))
            .collect(),
        ttl_secs: config.tickets.ttl_secs,
        key,
    }));
    let shutdown = process.shutdown_signal();
    process.mark_ready_with(&[("api", addr)])?;
    tracing::info!(api_addr = %addr, "publisher ready");
    tokio::try_join!(
        async {
            process
                .run_until_shutdown()
                .await
                .context("stopping publisher admin port")
        },
        async {
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown)
                .await
                .context("serving publisher API")
        },
    )?;
    pool.close().await;
    Ok(())
}

async fn migrate(config: &DbConfig) -> anyhow::Result<()> {
    let pool = connect_login_db(config).await?;
    db::migrate(&pool, &LOGIN_MIGRATOR)
        .await
        .context("applying LoginDB migrations")?;
    pool.close().await;
    Ok(())
}
