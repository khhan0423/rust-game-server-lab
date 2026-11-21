//! Polls the existing admin ports and renders a refreshable status table.

use std::net::SocketAddr;
use std::sync::Arc;

use askama::Template;
use axum::Router;
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;
use infra::admin::{LIVE, READY, SERVER_READY};

use crate::AppState;
use crate::config::WatchedServer;
use crate::presentation::render;

use crate::metrics::{RUNTIME_METRICS, samples};

#[derive(Template)]
#[template(path = "servers.html")]
struct ServersPage<'a> {
    operator: &'a str,
    rows: Vec<ServerRow>,
}

#[derive(Template)]
#[template(path = "server_table.html")]
struct ServerTable {
    rows: Vec<ServerRow>,
}

struct ServerRow {
    name: String,
    address: SocketAddr,
    liveness: &'static str,
    readiness: &'static str,
    metrics: Vec<(String, String)>,
}

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/servers", get(servers))
        .route("/servers/table", get(table))
}

async fn servers(State(state): State<Arc<AppState>>) -> Response {
    let rows = load(&state).await;
    render(&ServersPage {
        operator: state.publisher.operator(),
        rows,
    })
}

async fn table(State(state): State<Arc<AppState>>) -> Response {
    render(&ServerTable {
        rows: load(&state).await,
    })
}

async fn load(state: &AppState) -> Vec<ServerRow> {
    crate::metrics::poll_in_order(
        state
            .servers
            .iter()
            .map(|server| poll(&state.client, server))
            .collect(),
    )
    .await
}

async fn poll(client: &reqwest::Client, server: &WatchedServer) -> ServerRow {
    let address = server.admin_addr;
    let root = format!("http://{address}");
    let (live, ready, metrics) = tokio::join!(
        client.get(format!("{root}{LIVE}")).send(),
        client.get(format!("{root}{READY}")).send(),
        server.scrape(client),
    );
    let liveness = if live.is_ok_and(|response| response.status().is_success()) {
        "up"
    } else {
        "down"
    };
    let readiness = if ready.is_ok_and(|response| response.status().is_success()) {
        "ready"
    } else {
        "not ready"
    };
    let metrics = match metrics {
        Ok(body) => {
            let priority = [
                SERVER_READY,
                RUNTIME_METRICS[0],
                RUNTIME_METRICS[1],
                RUNTIME_METRICS[2],
                "net_connections_current",
            ];
            let samples = samples(&body)
                .filter(|sample| {
                    sample.labels.is_none()
                        || (sample.name == "net_connections_current"
                            && sample.label("listener") == Some("client"))
                })
                .collect::<Vec<_>>();
            priority
                .into_iter()
                .filter_map(|name| samples.iter().find(|sample| sample.name == name))
                .chain(
                    samples
                        .iter()
                        .filter(|sample| !priority.contains(&sample.name))
                        .take(4),
                )
                .map(|sample| (sample.name.to_owned(), sample.value.to_owned()))
                .collect()
        }
        Err(_) => Vec::new(),
    };
    ServerRow {
        name: server.name.clone(),
        address,
        liveness,
        readiness,
        metrics,
    }
}
