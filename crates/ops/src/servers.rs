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
            Ok(body) => samples(&body)
                .map(|(name, value)| (name.to_owned(), value.to_owned()))
                .collect(),
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

fn samples(text: &str) -> impl Iterator<Item = (&str, &str)> {
    fn sample(line: &str) -> Option<(&str, &str)> {
        let mut fields = line.split_ascii_whitespace();
        let name = fields.next()?;
        let value = fields.next()?;
        if !name.starts_with('#')
            && name
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'_' | b':'))
            && value.parse::<f64>().is_ok()
        {
            Some((name, value))
        } else {
            None
        }
    }
    text.lines()
        .filter_map(sample)
        .filter(|(name, _)| *name == "server_ready")
        .take(1)
        .chain(
            text.lines()
                .filter_map(sample)
                .filter(|(name, _)| *name != "server_ready")
                .take(4),
        )
}

#[cfg(test)]
mod tests {
    use super::samples;

    #[test]
    fn metrics_keep_unlabeled_numeric_samples_and_prioritize_readiness() {
        let cases: &[(&str, &[(&str, &str)])] = &[
            (
                "# TYPE server_ready gauge\nconnection_count 2\nserver_ready 1\n",
                &[("server_ready", "1"), ("connection_count", "2")],
            ),
            (
                "bad nope\nwith_labels{shard=\"1\"} 5\nduration 1.2e-2 123456\n",
                &[("duration", "1.2e-2")],
            ),
            ("\n# help\nmissing\n", &[]),
        ];
        for &(text, expected) in cases {
            assert_eq!(samples(text).collect::<Vec<_>>(), expected);
        }
    }
}
