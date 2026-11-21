//! Publisher guest login and game entry ticket service.

mod api;
mod config;
mod credentials;
mod ops_api;
mod service;
mod wire;

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use db::login::LOGIN_MIGRATOR;
use infra::config::Secret;

use crate::config::Config;

const LOGIN_DB_URL_ENV: &str = "LOGIN_DB_URL";

#[derive(Debug, Parser)]
struct Cli {
    /// Path to the config file (TOML)
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
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
    match cli.command.unwrap_or(Command::Run) {
        Command::Run => infra::server::run(&config.process, async |server| {
            let pool = service::start(server, &config).await?;
            Ok(async move { pool.close().await })
        }),
        Command::Migrate => {
            let url = Secret::from_env(LOGIN_DB_URL_ENV)?;
            config.process.runtime().build()?.block_on(db::migrate_url(
                url.expose(),
                &config.db,
                &LOGIN_MIGRATOR,
            ))?;
            Ok(())
        }
    }
}
