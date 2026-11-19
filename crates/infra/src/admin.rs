//! 관리 포트: 헬스 체크와 지표 (설계 08 2장).

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

/// "일을 받아도 되는가"를 나타내는 공유 플래그. 처음에는 준비되지 않은 상태다.
#[derive(Debug, Clone, Default)]
pub struct Readiness(Arc<AtomicBool>);

impl Readiness {
    /// 준비됨으로 바꾼다. 이후 `/health/ready`가 200을 돌려준다.
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

/// 주소에 묶인 관리 포트. [`AdminListener::serve`]를 불러야 요청을 받는다.
#[derive(Debug)]
pub struct AdminListener {
    listener: TcpListener,
    addr: SocketAddr,
}

impl AdminListener {
    /// `addr`에 관리 포트를 연다. 포트 0을 주면 운영체제가 빈 포트를 고른다.
    pub async fn bind(addr: SocketAddr) -> std::io::Result<Self> {
        let listener = TcpListener::bind(addr).await?;
        let addr = listener.local_addr()?;
        Ok(Self { listener, addr })
    }

    /// 실제로 묶인 주소.
    pub fn local_addr(&self) -> SocketAddr {
        self.addr
    }

    /// `shutdown`이 취소될 때까지 관리 요청을 처리한다.
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
