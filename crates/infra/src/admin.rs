//! Admin port: health checks and metrics (design 08 section 2).

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::get;
use metrics_exporter_prometheus::PrometheusHandle;
use tokio::net::TcpListener;
use tokio_util::sync::CancellationToken;

/// Shared flag indicating "may this process accept work". It starts in the not-ready state.
#[derive(Debug, Clone, Default)]
pub struct Readiness(Arc<AtomicBool>);

impl Readiness {
    /// Switches to ready. After this, `/health/ready` returns 200.
    pub fn set_ready(&self) {
        self.0.store(true, Ordering::Release);
    }

    fn is_ready(&self) -> bool {
        self.0.load(Ordering::Acquire)
    }
}

#[derive(Clone)]
struct AdminState {
    readiness: Readiness,
    metrics: PrometheusHandle,
}

/// Admin port bound to an address. It accepts requests only after [`AdminListener::serve`] is called.
#[derive(Debug)]
pub struct AdminListener {
    listener: TcpListener,
    addr: SocketAddr,
}

impl AdminListener {
    /// Opens the admin port on `addr`. Passing port 0 lets the operating system pick a free port.
    pub async fn bind(addr: SocketAddr) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let addr = listener.local_addr()?;
        Ok(Self { listener, addr })
    }

    /// The address actually bound.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// Handles admin requests until `shutdown` is cancelled.
    pub async fn serve(
        self,
        readiness: Readiness,
        metrics: PrometheusHandle,
        shutdown: CancellationToken,
    ) -> std::io::Result<()> {
        let app = Router::new()
            .route("/health/live", get(live))
            .route("/health/ready", get(ready))
            .route("/metrics", get(render_metrics))
            .with_state(AdminState { readiness, metrics });
        axum::serve(self.listener, app)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await
    }
}

async fn live() -> StatusCode {
    StatusCode::OK
}

async fn ready(State(state): State<AdminState>) -> StatusCode {
    if state.readiness.is_ready() {
        StatusCode::OK
    } else {
        StatusCode::SERVICE_UNAVAILABLE
    }
}

async fn render_metrics(State(state): State<AdminState>) -> String {
    state.metrics.render()
}
