//! Bot scenarios against the real gateway.

use std::time::Duration;

use bot::{RunConfig, Scenario};
use testkit::{GatewayFixture, ServerProcess, TestDir, sibling_binary};
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn both_bot_scenarios_report_the_gateway_close() {
    let dir = TestDir::new(env!("CARGO_TARGET_TMPDIR"), "bot_gateway")
        .await
        .unwrap();
    let binary = sibling_binary(env!("CARGO_BIN_EXE_bot"), "gateway");
    let mut command = GatewayFixture::new("127.0.0.1:0".parse().unwrap(), 8, 750)
        .command(&binary, &dir)
        .await
        .unwrap();
    let mut service = ServerProcess::spawn(&mut command).await.unwrap();
    let gateway = service.addr("client").unwrap();

    for (scenario, reason, auth) in [
        (Scenario::ConnectAuth, "auth_failed", true),
        (Scenario::ConnectHold, "auth_timeout", false),
    ] {
        let config = RunConfig {
            gateway,
            connections: 3,
            ramp_per_sec: "100".parse().unwrap(),
            hold_secs: 3,
            expect_established: Some(3),
            expect_close_reason: Some(reason.into()),
            scenario,
        };
        let summary = tokio::time::timeout(
            Duration::from_secs(10),
            bot::run(&config, &CancellationToken::new()),
        )
        .await
        .unwrap()
        .unwrap();
        assert!(summary.meets_expectations(&config), "{summary:?}");
        assert_eq!(summary.attempted, 3);
        assert_eq!(summary.failed_to_connect, 0);
        assert_eq!(summary.closed_by_timeout, 0);
        assert_eq!(summary.auth_to_close_ms.is_some(), auth);
    }
    service.stop().await.unwrap();
}
