//! Development operations tool for server monitoring and publisher account support.

mod config;
mod connections;
mod pages;
mod publisher;
mod security;
mod servers;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Context;
use axum::Router;
use axum::middleware;
use axum::response::Redirect;
use axum::routing::get;
use clap::Parser;
use infra::process::Process;
use tokio::net::TcpListener;

use config::{Config, WatchedServer};

#[derive(Debug, Parser)]
struct Cli {
    /// Path to the config file (TOML)
    #[arg(long)]
    config: PathBuf,
}

struct AppState {
    publisher: publisher::Client,
    client: reqwest::Client,
    servers: Vec<WatchedServer>,
    security: security::Security,
}

fn main() -> anyhow::Result<()> {
    let config: Config = infra::config::load(&Cli::parse().config)?;
    config.check()?;
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .context("building tokio runtime")?;
    runtime.block_on(run(config))
}

async fn run(config: Config) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("building operations HTTP client")?;
    let publisher = publisher::Client::new(
        client.clone(),
        &config.publisher.ops_api_url,
        &config.operator.name,
    )?;
    let listener = TcpListener::bind(config.http.listen_addr)
        .await
        .context("binding operations HTTP listener")?;
    let addr = listener
        .local_addr()
        .context("reading operations HTTP address")?;
    let state = Arc::new(AppState {
        publisher,
        client,
        servers: config.servers,
        security: security::Security::new(addr)?,
    });
    let router = Router::new()
        .route("/", get(|| async { Redirect::to("/servers") }))
        .merge(servers::routes())
        .merge(connections::routes())
        .merge(pages::routes())
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            security::headers,
        ))
        .with_state(state);
    let process = Process::start(&config.process).await?;
    let shutdown = process.shutdown_signal();
    process.mark_ready_with(&[("http", addr)])?;
    tracing::info!(http_addr = %addr, "operations tool ready");
    tokio::try_join!(
        async {
            process
                .run_until_shutdown()
                .await
                .context("stopping operations admin port")
        },
        async {
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown)
                .await
                .context("serving operations HTTP")
        },
    )?;
    Ok(())
}
