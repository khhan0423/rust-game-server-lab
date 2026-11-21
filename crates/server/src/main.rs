//! Game server binary.
//!
//! For now it only goes as far as the startup barrier: it signals ready only after config, logging, the admin port,
//! the Game DB connection, and the schema check have all finished (design 01 section 10.0).

use std::path::PathBuf;

use anyhow::Context;
use clap::{Parser, Subcommand};
use infra::config::Secret;
use infra::process::ProcessConfig;
use serde::Deserialize;

/// Environment variable holding the Game DB connection string. It is a secret, so it is not kept in the config file (design 08 section 8).
const GAME_DB_URL_ENV: &str = "GAME_DB_URL";

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
    /// Start the server
    Run,
    /// Apply migrations to the Game DB and exit
    Migrate,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    process: ProcessConfig,
    db: db::PoolConfig,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config: Config = infra::config::load(&cli.config.context("--config is required")?)?;
    match cli.command.unwrap_or(Command::Run) {
        Command::Run => infra::server::run(&config.process, async |_| {
            let url = Secret::from_env(GAME_DB_URL_ENV)?;
            let pool = db::open(url.expose(), &config.db, &db::GAME_MIGRATOR).await?;
            Ok(async move { pool.close().await })
        }),
        Command::Migrate => {
            let url = Secret::from_env(GAME_DB_URL_ENV)?;
            config.process.runtime().build()?.block_on(db::migrate_url(
                url.expose(),
                &config.db,
                &db::GAME_MIGRATOR,
            ))?;
            Ok(())
        }
    }
}
