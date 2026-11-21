//! Operations HTTP service composition.
use crate::{
    AppState, config, config::WatchedServer, connections, pages, presentation, publisher, security,
    servers,
};
use anyhow::Context;
use axum::{Router, middleware, response::Redirect, routing::get};
use infra::{
    http::{Exposure, HttpListener},
    server::Server,
};
use std::{sync::Arc, time::Duration};

pub(super) async fn start(
    server: &mut Server,
    http: infra::http::HttpConfig,
    publisher_config: config::Publisher,
    operator: config::Operator,
    servers: Vec<WatchedServer>,
) -> anyhow::Result<()> {
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(2))
        .no_proxy()
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .context("building operations HTTP client")?;
    let publisher = publisher::Client::new(
        client.clone(),
        &publisher_config.ops_api_url,
        &operator.name,
    )?;
    let listener = HttpListener::bind("http", &http, Exposure::Loopback)
        .await
        .context("binding operations HTTP listener")?;
    let addr = listener.local_addr();
    let state = Arc::new(AppState {
        publisher,
        client,
        servers,
        security: security::Security::new(addr)?,
    });
    let router = Router::new()
        .route("/", get(|| async { Redirect::to("/servers") }))
        .merge(servers::routes())
        .merge(connections::routes())
        .merge(pages::routes())
        .merge(presentation::assets())
        .layer(middleware::from_fn_with_state(
            Arc::clone(&state),
            security::headers,
        ))
        .with_state(state);
    server.spawn_http(listener, router);
    Ok(())
}
