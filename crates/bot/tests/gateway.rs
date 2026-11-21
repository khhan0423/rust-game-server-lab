use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use bot::{RunConfig, Scenario};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

#[tokio::test]
async fn both_bot_scenarios_report_the_gateway_close() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("bot_gateway");
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("gateway.toml");
    std::fs::write(
        &config,
        format!(
            "[process]\nname = 'gateway'\nenvironment = 'development'\nadmin_addr = '127.0.0.1:0'\nlog_root = '{}'\nlog_level = 'info'\n\
         [client]\nlisten_addr = '127.0.0.1:0'\nmax_connections = 8\nauth_timeout_ms = 750\n\
         write_queue_capacity = 256\nlog_connections = true\n",
            dir.join("logs").display()
        ),
    )
    .unwrap();
    let gateway = Path::new(env!("CARGO_BIN_EXE_bot"))
        .with_file_name(format!("gateway{}", std::env::consts::EXE_SUFFIX));
    let mut service = Command::new(gateway)
        .arg("--config")
        .arg(config)
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .expect("build the Gateway binary before running this test (scripts/gate.sh)");
    let stdout = service.stdout.take().unwrap();
    let ready = tokio::time::timeout(
        Duration::from_secs(60),
        BufReader::new(stdout).lines().next_line(),
    )
    .await
    .unwrap()
    .unwrap()
    .unwrap();
    let gateway = ready.split_once(" client=").unwrap().1.parse().unwrap();

    for (scenario, reason, auth) in [
        (Scenario::ConnectAuth, "auth_failed", true),
        (Scenario::ConnectHold, "auth_timeout", false),
    ] {
        let config = RunConfig {
            gateway,
            connections: 3,
            ramp_per_sec: 100,
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
    service.kill().await.unwrap();
}
