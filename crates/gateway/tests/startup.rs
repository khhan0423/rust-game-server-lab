//! The Gateway's startup and unauthenticated TCP contract.

#![expect(
    clippy::unwrap_used,
    clippy::expect_used,
    reason = "test fixture failures fail the test"
)]

use std::net::SocketAddr;
use std::path::Path;
use std::process::Stdio;
use std::time::Duration;

use bytes::BytesMut;
use net::{Frame, FrameCodec, FrameFlags, MAX_PACKET_SIZE};
use protocol::{ClientPacket, SequenceNumber, ServerPacket, wire};
use tokio::io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::{Child, Command};
use tokio_util::codec::{Decoder, Encoder};

const WAIT: Duration = Duration::from_secs(60);

fn spawn(name: &str, addr: SocketAddr, max: usize, timeout: u64) -> Child {
    let dir = Path::new(env!("CARGO_TARGET_TMPDIR")).join(name);
    std::fs::create_dir_all(&dir).unwrap();
    let config = dir.join("gateway.toml");
    let text = format!(
        "[process]\nname = 'gateway'\nenvironment = 'development'\nadmin_addr = '127.0.0.1:0'\nlog_root = '{}'\nlog_level = 'info'\n\
         [client]\nlisten_addr = '{addr}'\nmax_connections = {max}\nauth_timeout_ms = {timeout}\n\
         write_queue_capacity = 1\nlog_connections = true\n",
        dir.join("logs").display()
    );
    std::fs::write(&config, text).unwrap();
    Command::new(env!("CARGO_BIN_EXE_gateway"))
        .arg("--config")
        .arg(&config)
        .stdout(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .unwrap()
}

async fn start(name: &str, max: usize, timeout: u64) -> (Child, SocketAddr, SocketAddr) {
    let mut child = spawn(name, "127.0.0.1:0".parse().unwrap(), max, timeout);
    let stdout = child.stdout.take().unwrap();
    let line = tokio::time::timeout(WAIT, BufReader::new(stdout).lines().next_line())
        .await
        .expect("gateway did not announce readiness in time")
        .unwrap()
        .expect("gateway exited before announcing readiness");
    let (admin, client) = line
        .strip_prefix("ready admin=")
        .unwrap()
        .split_once(" client=")
        .unwrap();
    (child, admin.parse().unwrap(), client.parse().unwrap())
}

async fn send(socket: &mut TcpStream, packet: ClientPacket) {
    let mut bytes = BytesMut::new();
    FrameCodec::Client
        .encode(
            Frame {
                message_id: packet.id(),
                flags: FrameFlags::NONE,
                seq: SequenceNumber::new(0),
                ack: SequenceNumber::new(0),
                payload: packet.encode().into(),
            },
            &mut bytes,
        )
        .unwrap();
    socket.write_all(&bytes).await.unwrap();
}

async fn read_closed(mut socket: TcpStream) -> BytesMut {
    let mut remainder = Vec::new();
    tokio::time::timeout(WAIT, socket.read_to_end(&mut remainder))
        .await
        .unwrap()
        .unwrap();
    BytesMut::from(remainder.as_slice())
}

async fn assert_close(socket: TcpStream, reason: wire::CloseReason) {
    let mut bytes = read_closed(socket).await;
    let frame = FrameCodec::Client.decode(&mut bytes).unwrap().unwrap();
    assert_eq!(
        ServerPacket::decode(frame.message_id, &frame.payload).unwrap(),
        ServerPacket::Close(wire::Close {
            reason: i32::from(reason),
            reconnect_allowed: false,
            message: None
        })
    );
    assert!(bytes.is_empty());
}

async fn metrics_when(admin: SocketAddr, expected: &[&str]) -> String {
    let client = reqwest::Client::new();
    tokio::time::timeout(WAIT, async {
        loop {
            let metrics = client
                .get(format!("http://{admin}/metrics"))
                .send()
                .await
                .unwrap()
                .error_for_status()
                .unwrap()
                .text()
                .await
                .unwrap();
            if expected
                .iter()
                .all(|line| metrics.lines().any(|actual| actual == *line))
            {
                break metrics;
            }
        }
    })
    .await
    .expect("gateway metrics did not reach expected values")
}

#[tokio::test]
async fn gateway_announces_ready_and_answers_health_checks() {
    let (mut child, admin, _) = start("gateway_ready", 1, 1000).await;
    let ready = reqwest::get(format!("http://{admin}/health/ready"))
        .await
        .unwrap();
    assert_eq!(ready.status(), 200);
    let metrics = metrics_when(admin, &["gateway_connections_current 0"]).await;
    for name in [
        "runtime_workers",
        "runtime_alive_tasks",
        "runtime_global_queue_depth",
    ] {
        assert!(
            metrics
                .lines()
                .any(|line| line.starts_with(&format!("{name} ")))
        );
    }
    child.kill().await.unwrap();
}

#[tokio::test]
async fn gateway_rejects_unauthenticated_connections_and_accounts_for_each_close() {
    let (mut child, admin, client) = start("gateway_handshake", 8, 500).await;
    assert_close(
        TcpStream::connect(client).await.unwrap(),
        wire::CloseReason::IdleTimeout,
    )
    .await;

    let mut auth = TcpStream::connect(client).await.unwrap();
    send(
        &mut auth,
        ClientPacket::Auth(wire::Auth {
            login_token: vec![1, 2, 3],
            protocol_version: 1,
            client_version: "test".into(),
            platform: "test".into(),
        }),
    )
    .await;
    assert_close(auth, wire::CloseReason::AuthenticationFailed).await;

    let mut oversized = TcpStream::connect(client).await.unwrap();
    let len = u32::try_from(MAX_PACKET_SIZE + 1).unwrap();
    oversized.write_all(&len.to_le_bytes()).await.unwrap();
    assert!(read_closed(oversized).await.is_empty());

    let mut ping = TcpStream::connect(client).await.unwrap();
    send(&mut ping, ClientPacket::Ping(wire::Ping { client_time: 1 })).await;
    assert!(read_closed(ping).await.is_empty());

    metrics_when(
        admin,
        &[
            "gateway_connections_current 0",
            "gateway_connections_accepted_total 4",
            "gateway_connections_closed_total{reason=\"auth_timeout\"} 1",
            "gateway_connections_closed_total{reason=\"auth_failed\"} 1",
            "gateway_connections_closed_total{reason=\"oversized_frame\"} 1",
            "gateway_connections_closed_total{reason=\"protocol_violation\"} 1",
            "gateway_frames_received_total 2",
            "gateway_frames_sent_total 2",
        ],
    )
    .await;
    child.kill().await.unwrap();
}

#[tokio::test]
async fn gateway_refuses_connections_over_the_cap() {
    let (mut child, admin, client) = start("gateway_cap", 1, 60_000).await;
    let held = TcpStream::connect(client).await.unwrap();
    metrics_when(admin, &["gateway_connections_current 1"]).await;
    assert!(
        read_closed(TcpStream::connect(client).await.unwrap())
            .await
            .is_empty()
    );
    drop(held);
    metrics_when(
        admin,
        &[
            "gateway_connections_current 0",
            "gateway_connections_accepted_total 1",
            "gateway_connections_rejected_total{reason=\"limit\"} 1",
            "gateway_connections_closed_total{reason=\"peer_closed\"} 1",
        ],
    )
    .await;
    child.kill().await.unwrap();
}

#[tokio::test]
async fn gateway_does_not_announce_ready_when_client_address_is_taken() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let child = spawn("gateway_bind", listener.local_addr().unwrap(), 1, 1000);
    let result = tokio::time::timeout(WAIT, child.wait_with_output())
        .await
        .unwrap()
        .unwrap();
    assert!(!result.status.success());
    assert_eq!(result.stdout, b"");
}
