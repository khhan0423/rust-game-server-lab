//! Polls the existing admin ports and renders a refreshable status table.

use std::net::SocketAddr;
use std::sync::Arc;

use askama::Template;
use axum::Router;
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;
use infra::admin::{LIVE, METRICS, READY};

use crate::AppState;
use crate::config::WatchedServer;
use crate::pages::render;

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
    let mut rows = Vec::with_capacity(state.servers.len());
    for server in &state.servers {
        rows.push(poll(&state.client, server).await);
    }
    rows
}

async fn poll(client: &reqwest::Client, server: &WatchedServer) -> ServerRow {
    let address = server.admin_addr;
    let root = format!("http://{address}");
    let (live, ready, metrics) = tokio::join!(
        client.get(format!("{root}{LIVE}")).send(),
        client.get(format!("{root}{READY}")).send(),
        client.get(format!("{root}{METRICS}")).send(),
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
    let metrics = match metrics.and_then(reqwest::Response::error_for_status) {
        Ok(response) => match response.text().await {
            Ok(body) => {
                let priority = [
                    "server_ready",
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
        },
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
