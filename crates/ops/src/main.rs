//! Development operations tool for server monitoring and publisher account support.

mod config;
mod connections;
mod metrics;
mod pages;
mod presentation;
mod publisher;
mod security;
mod servers;
mod service;

use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;

use config::{Config, WatchedServer};

#[derive(Debug, Parser)]
struct Cli {
    /// Path to the config file (TOML)
    #[arg(long, global = true)]
    config: Option<PathBuf>,
    #[command(subcommand)]
    command: Option<Command>,
}

#[derive(Debug, clap::Subcommand)]
enum Command {
    /// Start the server.
    Run,
}

struct AppState {
    publisher: publisher::Client,
    client: reqwest::Client,
    servers: Vec<WatchedServer>,
    security: security::Security,
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config: Config = infra::config::load(&cli.config.context("--config is required")?)?;
    config.check()?;
    infra::server::run(&config.process, async |server| {
        service::start(
            server,
            config.http,
            config.publisher,
            config.operator,
            config.servers,
        )
        .await?;
        Ok(std::future::ready(()))
    })
}
