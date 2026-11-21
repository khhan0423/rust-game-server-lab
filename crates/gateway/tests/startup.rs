//! The Gateway's startup and unauthenticated TCP contract.

#![expect(clippy::unwrap_used, reason = "test fixture failures fail the test")]

use bytes::BytesMut;
use net::{Frame, FrameCodec, MAX_PACKET_SIZE};
use protocol::{ClientPacket, ServerPacket, wire};
use std::net::SocketAddr;
use std::path::Path;
use testkit::{GatewayFixture, ServerProcess, TestDir, WAIT};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::{TcpListener, TcpStream};
use tokio::process::Command;
use tokio_util::codec::{Decoder, Encoder};

async fn command(name: &str, addr: SocketAddr, max: usize, timeout: u64) -> Command {
    let dir = TestDir::new(env!("CARGO_TARGET_TMPDIR"), name)
        .await
        .unwrap();
    GatewayFixture::new(addr, max, timeout)
        .with_queue(1)
        .command(Path::new(env!("CARGO_BIN_EXE_gateway")), &dir)
        .await
        .unwrap()
}

async fn start(name: &str, max: usize, timeout: u64) -> (ServerProcess, SocketAddr, SocketAddr) {
    let mut command = command(name, "127.0.0.1:0".parse().unwrap(), max, timeout).await;
    let child = ServerProcess::spawn(&mut command).await.unwrap();
    let admin = child.addr("admin").unwrap();
    let client = child.addr("client").unwrap();
    (child, admin, client)
}

async fn send(socket: &mut TcpStream, packet: ClientPacket) {
    let mut bytes = BytesMut::new();
    FrameCodec::Client
        .encode(
            Frame::unsequenced(packet.id(), packet.encode().into()),
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

#[tokio::test]
async fn gateway_announces_ready_and_answers_health_checks() {
    let (mut child, admin, _) = start("gateway_ready", 1, 1000).await;
    let ready = reqwest::get(format!("http://{admin}/health/ready"))
        .await
        .unwrap();
    assert_eq!(ready.status(), 200);
    let metrics =
        testkit::metrics_until(admin, &["net_connections_current{listener=\"client\"} 0"])
            .await
            .unwrap();
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
    child.stop().await.unwrap();
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
            protocol_version: protocol::PROTOCOL_VERSION,
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

    testkit::metrics_until(
        admin,
        &[
            "net_connections_current{listener=\"client\"} 0",
            "net_connections_accepted_total{listener=\"client\"} 4",
            "net_connections_closed_total{listener=\"client\",reason=\"auth_timeout\"} 1",
            "net_connections_closed_total{listener=\"client\",reason=\"auth_failed\"} 1",
            "net_connections_closed_total{listener=\"client\",reason=\"oversized_frame\"} 1",
            "net_connections_closed_total{listener=\"client\",reason=\"protocol_violation\"} 1",
            "net_frames_received_total{listener=\"client\"} 2",
            "net_frames_sent_total{listener=\"client\"} 2",
        ],
    )
    .await
    .unwrap();
    child.stop().await.unwrap();
}

#[tokio::test]
async fn gateway_refuses_connections_over_the_cap() {
    let (mut child, admin, client) = start("gateway_cap", 1, 60_000).await;
    let held = TcpStream::connect(client).await.unwrap();
    testkit::metrics_until(admin, &["net_connections_current{listener=\"client\"} 1"])
        .await
        .unwrap();
    assert!(
        read_closed(TcpStream::connect(client).await.unwrap())
            .await
            .is_empty()
    );
    drop(held);
    testkit::metrics_until(
        admin,
        &[
            "net_connections_current{listener=\"client\"} 0",
            "net_connections_accepted_total{listener=\"client\"} 1",
            "net_connections_rejected_total{listener=\"client\",reason=\"limit\"} 1",
            "net_connections_closed_total{listener=\"client\",reason=\"peer_closed\"} 1",
        ],
    )
    .await
    .unwrap();
    child.stop().await.unwrap();
}

#[tokio::test]
async fn gateway_does_not_announce_ready_when_client_address_is_taken() {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut command = command("gateway_bind", listener.local_addr().unwrap(), 1, 1000).await;
    let result = ServerProcess::finished(&mut command).await.unwrap();
    assert!(!result.status.success());
    assert_eq!(result.stdout, b"");
}
