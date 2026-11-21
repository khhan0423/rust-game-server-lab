//! Publisher guest login and game entry ticket service.

mod api;
mod config;
mod credentials;
mod ops_api;

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

struct Startup {
    pool: db::Pool,
    public: TcpListener,
    operator: TcpListener,
    game_key: Secret,
    ops_key: Secret,
}

async fn startup_barrier(config: &Config) -> anyhow::Result<Startup> {
    let key = Secret::from_env(GAME_API_KEY_ENV)?;
    anyhow::ensure!(!key.expose().is_empty(), "{GAME_API_KEY_ENV} is empty");
    let ops_key = Secret::from_env("PUBLISHER_OPS_API_KEY")?;
    anyhow::ensure!(
        !ops_key.expose().is_empty(),
        "PUBLISHER_OPS_API_KEY is empty"
    );
    anyhow::ensure!(
        config.ops_api.listen_addr.ip().is_loopback(),
        "publisher operator API must listen on loopback"
    );
    let pool = connect_login_db(&config.db).await?;
    db::check_schema(&pool, &LOGIN_MIGRATOR)
        .await
        .context("checking LoginDB schema")?;
    let listener = TcpListener::bind(config.http.listen_addr)
        .await
        .context("binding publisher API listener")?;
    let operator = TcpListener::bind(config.ops_api.listen_addr)
        .await
        .context("binding publisher operator API listener")?;
    Ok(Startup {
        pool,
        public: listener,
        operator,
        game_key: key,
        ops_key,
    })
}

async fn run(config: Config) -> anyhow::Result<()> {
    let process = Process::start(&config.process).await?;
    let Startup {
        pool,
        public,
        operator,
        game_key,
        ops_key,
    } = match startup_barrier(&config).await {
        Ok(ready) => ready,
        Err(error) => {
            tracing::error!(%error, "startup failed");
            return Err(error);
        }
    };
    let addr = public
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
        key: game_key,
    }));
    let ops_addr = operator
        .local_addr()
        .context("reading publisher operator API address")?;
    let ops_router = ops_api::router(pool.clone(), ops_key);
    let shutdown = process.shutdown_signal();
    let ops_shutdown = process.shutdown_signal();
    process.mark_ready_with(&[("api", addr), ("ops_api", ops_addr)])?;
    tracing::info!(api_addr = %addr, ops_api_addr = %ops_addr, "publisher ready");
    tokio::try_join!(
        async {
            process
                .run_until_shutdown()
                .await
                .context("stopping publisher admin port")
        },
        async {
            axum::serve(public, router)
                .with_graceful_shutdown(shutdown)
                .await
                .context("serving publisher API")
        },
        async {
            axum::serve(operator, ops_router)
                .with_graceful_shutdown(ops_shutdown)
                .await
                .context("serving publisher operator API")
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
