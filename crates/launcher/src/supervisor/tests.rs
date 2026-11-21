//! Real gateway processes exercise the supervisor's ownership and exit reporting.

use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    time::Duration,
};

use testkit::{GatewayFixture, TestDir, WAIT};
use tokio::{net::TcpListener, time::timeout};

use super::*;
use crate::service::State;

pub(crate) fn built_binary(name: &str) -> PathBuf {
    let test_exe = std::env::current_exe().unwrap();
    // Cargo 1.100 places the harness below `build/<package>/<hash>`;
    // older toolchains place it under `deps`. Built binaries are in the profile root.
    let file = format!("{name}{}", std::env::consts::EXE_SUFFIX);
    test_exe
        .ancestors()
        .skip(1)
        .map(|dir| dir.join(&file))
        .find(|candidate| candidate.is_file())
        .unwrap_or_else(|| {
            panic!(
                "build --bin {name} before tests; running {}",
                test_exe.display()
            )
        })
}

pub(crate) async fn fixture(gateway: &Path, broken: bool) -> PathBuf {
    let dir = TestDir::new("../../target/tmp", "launcher").await.unwrap();
    let instance = GatewayFixture::new("127.0.0.1:0".parse().unwrap(), 32, 5000);
    let command = instance.command(gateway, &dir).await.unwrap();
    let config = command
        .as_std()
        .get_args()
        .nth(1)
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    let bin = gateway
        .parent()
        .unwrap()
        .to_string_lossy()
        .replace('\\', "/");
    let second = if broken { "missing.toml" } else { &config };
    dir.write(
        "topology.toml",
        &format!(
            r#"
bin_dir={bin:?}
env_file="missing-test.env"
ready_timeout_seconds=10
stop_timeout_seconds=5
[process]
name="launcher"
environment="development"
log_level="info"
log_root="../../target/tmp/launcher-logs"
worker_threads=1
[[services]]
name="first"
binary="gateway"
config={config:?}
[[services]]
name="second"
binary="gateway"
config={second:?}
depends_on=["first"]
"#
        ),
    )
    .await
    .unwrap()
}

async fn command(send: &mpsc::Sender<Command>, operation: Operation) {
    let (command, done) = Command::with_reply(operation);
    send.send(command).await.unwrap();
    timeout(WAIT, done).await.unwrap().unwrap().unwrap();
}

pub(crate) fn ready_address(snapshot: &Snapshot) -> SocketAddr {
    let State::Ready(line) = snapshot.state() else {
        panic!("expected ready: {snapshot:?}");
    };
    line.addr("admin").unwrap()
}

#[tokio::test]
async fn killing_one_service_leaves_the_other_ready() {
    let path = fixture(&built_binary("gateway"), false).await;
    let (supervisor, snapshots) = Supervisor::new(Topology::load(&path).unwrap()).unwrap();
    let shutdown = Shutdown::default();
    let (send, receive) = mpsc::channel(1);
    let task = tokio::spawn(supervisor.run(receive, shutdown.clone()));
    command(&send, Operation::StartAll(None)).await;
    command(&send, Operation::Kill("first".to_owned())).await;
    let current = snapshots.borrow().clone();
    assert!(matches!(current[0].state(), State::Exited(status) if !status.success()));
    let address = ready_address(&current[1]);
    let status = reqwest::get(format!("http://{address}/health/ready"))
        .await
        .unwrap()
        .status();
    assert_eq!(status, reqwest::StatusCode::OK);
    shutdown.cancel();
    task.await.unwrap().unwrap();
}

pub(crate) async fn port_released(address: SocketAddr) {
    timeout(Duration::from_secs(5), async {
        loop {
            match TcpListener::bind(address).await {
                Ok(_) => return,
                Err(error) if error.kind() == std::io::ErrorKind::AddrInUse => {
                    tokio::task::yield_now().await;
                }
                Err(error) => panic!("checking released port {address}: {error}"),
            }
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn dropping_the_supervisor_releases_all_child_ports() {
    let path = fixture(&built_binary("gateway"), false).await;
    let (supervisor, snapshots) = Supervisor::new(Topology::load(&path).unwrap()).unwrap();
    let (send, receive) = mpsc::channel(1);
    let task = tokio::spawn(supervisor.run(receive, Shutdown::default()));
    command(&send, Operation::StartAll(None)).await;
    let addresses = snapshots
        .borrow()
        .iter()
        .map(ready_address)
        .collect::<Vec<_>>();

    task.abort();
    assert!(task.await.unwrap_err().is_cancelled());
    for address in addresses {
        port_released(address).await;
    }
}

#[tokio::test]
async fn headless_check_reaps_started_children_on_success_and_startup_failure() {
    let launcher = built_binary("launcher");
    let gateway = built_binary("gateway");
    for broken in [false, true] {
        let path = fixture(&gateway, broken).await;
        let mut cmd = tokio::process::Command::new(&launcher);
        cmd.args(["--headless", "check", "--topology"]).arg(path);

        let output = testkit::ServerProcess::finished(&mut cmd).await.unwrap();

        let stdout = String::from_utf8(output.stdout).unwrap();
        let rows = stdout.lines().collect::<Vec<_>>();
        let (result, ready) = rows.split_last().unwrap();
        assert_eq!(output.status.success(), !broken, "{stdout}");
        if broken {
            assert!(
                result.starts_with("RESULT launcher check FAIL: starting second:"),
                "{stdout}"
            );
            assert_eq!(ready.len(), 1, "{stdout}");
        } else {
            assert_eq!(*result, "RESULT launcher check PASS", "{stdout}");
            assert_eq!(ready.len(), 2, "{stdout}");
        }
        for (row, expected) in ready.iter().zip(["first", "second"]) {
            let (name, text) = row.split_once(' ').unwrap();
            assert_eq!(name, expected);
            let line: infra::ready::ReadyLine = text.parse().unwrap();
            port_released(line.addr("admin").unwrap()).await;
        }
    }
}
