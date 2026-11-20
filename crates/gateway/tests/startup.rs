//! Whether the Gateway starts, signals ready, and responds on the admin port.

use std::net::SocketAddr;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Upper bound for waiting on an event. Not a synchronization mechanism, but a breaker that cuts off a stuck test.
const WAIT: Duration = Duration::from_secs(60);

#[tokio::test]
async fn gateway_announces_ready_and_answers_health_checks() {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join("gateway_ready");
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("gateway.toml");
    let text = format!(
        "[process]\nadmin_addr = \"127.0.0.1:0\"\nlog_dir = '{}'\nlog_level = \"info\"\n",
        dir.join("logs").display()
    );
    std::fs::write(&config, text).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_gateway"))
        .arg("--config")
        .arg(&config)
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()
        .unwrap();

    let stdout = child.stdout.take().unwrap();
    let line = tokio::time::timeout(WAIT, BufReader::new(stdout).lines().next_line())
        .await
        .expect("gateway did not announce readiness in time")
        .unwrap()
        .expect("gateway exited before announcing readiness");
    let admin: SocketAddr = line
        .strip_prefix("ready admin=")
        .expect("unexpected ready line")
        .parse()
        .unwrap();

    let ready = reqwest::get(format!("http://{admin}/health/ready"))
        .await
        .unwrap();
    assert_eq!(ready.status(), 200);
}
