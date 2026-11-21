//! Gateway binary (design 02).

mod config;
mod connection;

use std::path::PathBuf;

use anyhow::Context;
use clap::Parser;

use config::Config;

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

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    let config: Config = infra::config::load(&cli.config.context("--config is required")?)?;
    infra::server::run(&config.process, async |server| {
        let listener = net::Listener::bind("client", config.client)
            .await
            .context("binding client listener")?;
        metrics::counter!("gateway_auth_attempts_total").absolute(0);
        metrics::counter!("gateway_auth_failed_total").absolute(0);
        server.spawn(
            "client",
            listener.local_addr(),
            listener.serve(connection::ClientHandshake, server.shutdown().token()),
        );
        Ok(std::future::ready(()))
    })
}
