use std::sync::Arc;

use askama::Template;
use axum::Router;
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;

use crate::AppState;
use crate::config::ServerRole;
use crate::pages::render;
use crate::servers::{RUNTIME_METRICS, samples};

#[derive(Template)]
#[template(path = "connections.html")]
struct Page<'a> {
    operator: &'a str,
    rows: Vec<Gateway>,
}

#[derive(Template)]
#[template(path = "connections_table.html")]
struct Table {
    rows: Vec<Gateway>,
}

struct Gateway {
    name: String,
    address: std::net::SocketAddr,
    status: &'static str,
    totals: Vec<(&'static str, String)>,
    rejected: Vec<(String, String)>,
    closed: Vec<(String, String)>,
}

pub(super) fn routes() -> Router<Arc<AppState>> {
    Router::new()
        .route("/connections", get(page))
        .route("/connections/table", get(table))
}

async fn page(State(state): State<Arc<AppState>>) -> Response {
    render(&Page {
        operator: state.publisher.operator(),
        rows: load(&state).await,
    })
}

async fn table(State(state): State<Arc<AppState>>) -> Response {
    render(&Table {
        rows: load(&state).await,
    })
}

async fn load(state: &AppState) -> Vec<Gateway> {
    let mut rows = Vec::new();
    for server in state
        .servers
        .iter()
        .filter(|server| server.role == Some(ServerRole::Gateway))
    {
        let mut row = Gateway {
            name: server.name.clone(),
            address: server.admin_addr,
            status: "down",
            totals: Vec::new(),
            rejected: Vec::new(),
            closed: Vec::new(),
        };
        let response = state
            .client
            .get(format!("http://{}/metrics", server.admin_addr))
            .send()
            .await;
        if let Ok(response) = response.and_then(reqwest::Response::error_for_status)
            && let Ok(body) = response.text().await
        {
            row.status = "up";
            row.collect(&body);
        }
        rows.push(row);
    }
    rows
}

impl Gateway {
    fn collect(&mut self, text: &str) {
        let samples = samples(text).collect::<Vec<_>>();
        for name in [
            "gateway_connections_current",
            "gateway_connections_accepted_total",
            "gateway_frames_received_total",
            "gateway_frames_sent_total",
            "gateway_bytes_received_total",
            "gateway_bytes_sent_total",
        ]
        .into_iter()
        .chain(RUNTIME_METRICS)
        {
            if let Some(sample) = samples
                .iter()
                .find(|sample| sample.name == name && sample.labels.is_none())
            {
                self.totals.push((name, sample.value.to_owned()));
            }
        }
        for sample in samples {
            let Some(reason) = sample
                .labels
                .and_then(|labels| labels.strip_prefix("reason=\""))
                .and_then(|label| label.strip_suffix('"'))
            else {
                continue;
            };
            let series = match sample.name {
                "gateway_connections_rejected_total" => &mut self.rejected,
                "gateway_connections_closed_total" => &mut self.closed,
                _ => continue,
            };
            series.push((reason.to_owned(), sample.value.to_owned()));
        }
        self.rejected.sort_unstable();
        self.closed.sort_unstable();
    }
}
