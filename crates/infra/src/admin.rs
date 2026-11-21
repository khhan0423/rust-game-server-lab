//! Admin port: health checks and metrics (design 08 section 2).

use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use crate::config::ConfigError;
use crate::hex::Hex;
use crate::http::{Exposure, HttpConfig, HttpListener, api::ApiKey};
use crate::shutdown::Shutdown;
use axum::Router;
use axum::extract::State;
use axum::http::StatusCode;
use axum::routing::{get, post};
use metrics_exporter_prometheus::PrometheusHandle;

/// Liveness probe path.
pub const LIVE: &str = "/health/live";
/// Startup readiness probe path.
pub const READY: &str = "/health/ready";
/// Prometheus scrape path.
pub const METRICS: &str = "/metrics";

/// Supervisor-only graceful shutdown route.
pub const STOP: &str = "/admin/shutdown";
/// Header carrying the credential for [`STOP`].
pub const STOP_HEADER: &str = "x-admin-stop-token";
/// Process environment variable that enables [`STOP`].
pub const STOP_ENV: &str = "ADMIN_STOP_TOKEN";

/// Credential issued by the parent for one child process.
#[derive(Debug)]
pub struct StopToken {
    key: ApiKey,
}

impl StopToken {
    /// Generates a new independent credential using the operating system.
    pub fn generate() -> std::io::Result<Self> {
        let text = Hex::encode(&Hex::random::<32>().map_err(std::io::Error::other)?);
        let key = ApiKey::parse(STOP_HEADER, &text)
            .ok_or_else(|| std::io::Error::other("cannot encode shutdown credential"))?;
        Ok(Self { key })
    }

    /// Supplies the credential through the child's environment.
    pub fn env_value(&self) -> &[u8] {
        self.key.bytes()
    }

    /// Asks the child to begin graceful shutdown via its announced admin address.
    pub async fn request(
        &self,
        client: &reqwest::Client,
        addr: SocketAddr,
    ) -> Result<(), reqwest::Error> {
        let mut headers = reqwest::header::HeaderMap::new();
        self.key.insert(&mut headers);
        client
            .post(format!("http://{addr}{STOP}"))
            .headers(headers)
            .send()
            .await?
            .error_for_status()?;
        Ok(())
    }
}

/// Gauge for the readiness state published by the process.
pub const SERVER_READY: &str = "server_ready";
/// Gauge for the number of runtime worker threads.
pub const RUNTIME_WORKERS: &str = "runtime_workers";
/// Gauge for tasks alive in the runtime.
pub const RUNTIME_ALIVE_TASKS: &str = "runtime_alive_tasks";
/// Gauge for tasks waiting in the runtime's global queue.
pub const RUNTIME_GLOBAL_QUEUE_DEPTH: &str = "runtime_global_queue_depth";

/// Shared flag indicating "may this process accept work". It starts in the not-ready state.
#[derive(Debug, Clone, Default)]
pub struct Readiness(Arc<AtomicBool>);

impl Readiness {
    /// Switches to ready. After this, `/health/ready` returns 200.
    pub fn set_ready(&self) {
        self.0.store(true, Ordering::Release);
    }

    pub(super) fn set_not_ready(&self) {
        self.0.store(false, Ordering::Release);
    }

    pub(crate) fn is_ready(&self) -> bool {
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
    listener: HttpListener,
    stop_key: Option<ApiKey>,
}

impl AdminListener {
    /// Opens the admin port on `addr`. Passing port 0 lets the operating system pick a free port.
    pub async fn bind(addr: SocketAddr) -> std::io::Result<Self> {
        let listener =
            HttpListener::bind("admin", &HttpConfig::new(addr), Exposure::Private).await?;
        Ok(Self {
            listener,
            stop_key: None,
        })
    }

    pub(crate) fn enable_stop_from_env(&mut self) -> Result<(), ConfigError> {
        match ApiKey::from_env(STOP_HEADER, STOP_ENV) {
            Ok(key) => self.stop_key = Some(key),
            Err(ConfigError::MissingEnv { .. }) => (),
            Err(error) => return Err(error),
        }
        Ok(())
    }

    /// The address actually bound.
    pub fn local_addr(&self) -> SocketAddr {
        self.listener.local_addr()
    }

    /// Handles admin requests until `shutdown` is cancelled.
    pub async fn serve(
        self,
        readiness: Readiness,
        metrics: PrometheusHandle,
        shutdown: Shutdown,
    ) -> std::io::Result<()> {
        let mut app = Router::new()
            .route(LIVE, get(live))
            .route(READY, get(ready))
            .route(METRICS, get(render_metrics))
            .with_state(AdminState { readiness, metrics });
        if let Some(key) = self.stop_key {
            let stop = shutdown.clone();
            let key = Arc::new(key);
            app = app.route(
                STOP,
                post(move |headers: axum::http::HeaderMap| async move {
                    key.verify(&headers)?;
                    stop.cancel();
                    Ok::<_, crate::http::api::ApiError>(StatusCode::NO_CONTENT)
                }),
            );
        }
        self.listener.serve(app, shutdown).await
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
