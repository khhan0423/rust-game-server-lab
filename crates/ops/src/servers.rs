//! Polls the existing admin ports and renders a refreshable status table.

use std::net::SocketAddr;
use std::sync::Arc;

use askama::Template;
use axum::Router;
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;

use crate::AppState;
use crate::config::WatchedServer;
use crate::pages::render;

pub(super) const RUNTIME_METRICS: [&str; 3] = [
    "runtime_workers",
    "runtime_alive_tasks",
    "runtime_global_queue_depth",
];

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
        client.get(format!("{root}/health/live")).send(),
        client.get(format!("{root}/health/ready")).send(),
        client.get(format!("{root}/metrics")).send(),
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
                ];
                let samples = samples(&body)
                    .filter(|sample| sample.labels.is_none())
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

#[derive(Debug)]
pub(super) struct Sample<'a> {
    pub(super) name: &'a str,
    pub(super) labels: Option<&'a str>,
    pub(super) value: &'a str,
}

pub(super) fn samples(text: &str) -> impl Iterator<Item = Sample<'_>> {
    text.lines().filter_map(|line| {
        let line = line.trim_start();
        let boundary = line.find(|ch: char| ch == '{' || ch.is_ascii_whitespace())?;
        let (name, rest) = line.split_at(boundary);
        let (labels, values) = if let Some(labels) = rest.strip_prefix('{') {
            let (labels, values) = labels.rsplit_once('}')?;
            (Some(labels), values)
        } else {
            (None, rest)
        };
        let value = values.split_ascii_whitespace().next()?;
        (name
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':'))
            && value.parse::<f64>().is_ok())
        .then_some(Sample {
            name,
            labels,
            value,
        })
    })
}

#[cfg(test)]
mod tests {
    use super::samples;

    #[test]
    fn metrics_keep_numeric_samples_and_labels() {
        for (text, expected) in [
            (
                "# TYPE server_ready gauge\nconnection_count 2\nserver_ready 1\n",
                vec![("connection_count", None, "2"), ("server_ready", None, "1")],
            ),
            (
                "bad nope\nwith_labels{shard=\"first shard\"} 5\nduration 1.2e-2 123456\n",
                vec![
                    ("with_labels", Some("shard=\"first shard\""), "5"),
                    ("duration", None, "1.2e-2"),
                ],
            ),
            ("\n# help\nmissing\n", vec![]),
        ] {
            assert_eq!(
                samples(text)
                    .map(|sample| (sample.name, sample.labels, sample.value))
                    .collect::<Vec<_>>(),
                expected
            );
        }
    }
}
