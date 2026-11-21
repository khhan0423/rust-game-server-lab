use std::sync::Arc;

use askama::Template;
use axum::Router;
use axum::extract::State;
use axum::response::Response;
use axum::routing::get;

use crate::AppState;
use crate::config::{ServerRole, WatchedServer};
use crate::metrics::{RUNTIME_METRICS, poll_in_order, samples};
use crate::presentation::render;

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
    poll_in_order(
        state
            .servers
            .iter()
            .filter(|server: &&WatchedServer| server.role == Some(ServerRole::Gateway))
            .map(|server: &WatchedServer| async move {
                let mut row = Gateway {
                    name: server.name.clone(),
                    address: server.admin_addr,
                    status: "down",
                    totals: Vec::new(),
                    rejected: Vec::new(),
                    closed: Vec::new(),
                };
                if let Ok(body) = server.scrape(&state.client).await {
                    row.status = "up";
                    row.collect(&body);
                }
                row
            })
            .collect(),
    )
    .await
}

impl Gateway {
    fn collect(&mut self, text: &str) {
        let samples = samples(text).collect::<Vec<_>>();
        for name in [
            "net_connections_current",
            "net_connections_accepted_total",
            "net_frames_received_total",
            "net_frames_sent_total",
            "net_bytes_received_total",
            "net_bytes_sent_total",
        ]
        .into_iter()
        .chain(RUNTIME_METRICS)
        {
            if let Some(sample) = samples.iter().find(|sample| {
                sample.name == name
                    && if name.starts_with("net_") {
                        sample.label("listener") == Some("client")
                    } else {
                        sample.labels.is_none()
                    }
            }) {
                self.totals.push((name, sample.value.to_owned()));
            }
        }
        for sample in samples {
            if sample.label("listener") != Some("client") {
                continue;
            }
            let Some(reason) = sample.label("reason") else {
                continue;
            };
            let series = match sample.name {
                "net_connections_rejected_total" => &mut self.rejected,
                "net_connections_closed_total" => &mut self.closed,
                _ => continue,
            };
            series.push((reason.to_owned(), sample.value.to_owned()));
        }
        self.rejected.sort_unstable();
        self.closed.sort_unstable();
    }
}
