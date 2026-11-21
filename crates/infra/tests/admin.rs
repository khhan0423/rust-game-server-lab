//! Health check and metrics responses of the admin port (design 08 section 2).

#![expect(
    clippy::unwrap_used,
    reason = "test helpers fail the test by panicking"
)]

use std::net::SocketAddr;

use infra::admin::{AdminListener, Readiness};
use infra::shutdown::Shutdown;
use metrics_exporter_prometheus::{PrometheusBuilder, PrometheusRecorder};

async fn start_admin(recorder: &PrometheusRecorder) -> (SocketAddr, Readiness) {
    let listener = AdminListener::bind("127.0.0.1:0".parse().unwrap())
        .await
        .unwrap();
    let addr = listener.local_addr();
    let readiness = Readiness::default();
    tokio::spawn(listener.serve(readiness.clone(), recorder.handle(), Shutdown::default()));
    (addr, readiness)
}

async fn status(addr: SocketAddr, path: &str) -> u16 {
    reqwest::get(format!("http://{addr}{path}"))
        .await
        .unwrap()
        .status()
        .as_u16()
}

#[tokio::test]
async fn live_is_ok_and_ready_is_unavailable_before_ready() {
    let recorder = PrometheusBuilder::new().build_recorder();
    let (addr, _readiness) = start_admin(&recorder).await;

    assert_eq!(status(addr, "/health/live").await, 200);
    assert_eq!(status(addr, "/health/ready").await, 503);
}

#[tokio::test]
async fn ready_turns_ok_after_set_ready() {
    let recorder = PrometheusBuilder::new().build_recorder();
    let (addr, readiness) = start_admin(&recorder).await;

    readiness.set_ready();

    assert_eq!(status(addr, "/health/ready").await, 200);
}

#[tokio::test]
async fn metrics_endpoint_renders_recorded_values() {
    let recorder = PrometheusBuilder::new().build_recorder();
    metrics::with_local_recorder(&recorder, || metrics::gauge!("admin_test_gauge").set(7.0));
    let (addr, _readiness) = start_admin(&recorder).await;

    let body = reqwest::get(format!("http://{addr}/metrics"))
        .await
        .unwrap()
        .text()
        .await
        .unwrap();

    assert!(
        body.lines().any(|line| line == "admin_test_gauge 7"),
        "unexpected body: {body}"
    );
}
