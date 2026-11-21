use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use bytes::BytesMut;
use net::{Frame, FrameCodec, FrameReader};
use protocol::{ClientPacket, ServerPacket, wire};
use tokio::io::AsyncWriteExt;
use tokio::net::TcpStream;
use tokio::time::Instant;
use tokio_util::codec::Encoder;
use tokio_util::sync::CancellationToken;

use crate::Scenario;

pub(super) struct Job {
    pub(super) gateway: SocketAddr,
    pub(super) scenario: Scenario,
    pub(super) hold: Duration,
    pub(super) established: Arc<AtomicU32>,
}

pub(super) enum Outcome {
    FailedConnect,
    CancelledConnect,
    Established {
        latency: Duration,
        end: End,
        auth_close: Option<Duration>,
    },
}

pub(super) enum End {
    Server(&'static str),
    Timeout,
    Io,
    Cancelled,
}

pub(super) async fn connect(job: Job, stop: CancellationToken) -> Outcome {
    let start = Instant::now();
    let mut socket = tokio::select! {
        biased;
        () = stop.cancelled() => return Outcome::CancelledConnect,
        result = tokio::time::timeout(Duration::from_secs(5), TcpStream::connect(job.gateway)) => {
            match result {
                Ok(Ok(socket)) => socket,
                Ok(Err(_)) | Err(_) => return Outcome::FailedConnect,
            }
        }
    };
    let latency = start.elapsed();
    job.established.fetch_add(1, Ordering::Relaxed);
    let auth = match job.scenario {
        Scenario::ConnectAuth => Some(Instant::now()),
        Scenario::ConnectHold => None,
    };
    let end = tokio::select! {
        biased;
        () = stop.cancelled() => End::Cancelled,
        result = tokio::time::timeout(job.hold, exchange(&mut socket, job.scenario)) => match result {
            Ok(Ok(reason)) => End::Server(reason),
            Ok(Err(_)) => End::Io,
            Err(_) => End::Timeout,
        }
    };
    let auth_close = match &end {
        End::Server(_) => auth.map(|start| start.elapsed()),
        End::Timeout | End::Io | End::Cancelled => None,
    };
    Outcome::Established {
        latency,
        end,
        auth_close,
    }
}

async fn exchange(socket: &mut TcpStream, scenario: Scenario) -> anyhow::Result<&'static str> {
    match scenario {
        Scenario::ConnectHold => {}
        Scenario::ConnectAuth => {
            let packet = ClientPacket::Auth(wire::Auth {
                login_token: rand::random::<[u8; 32]>().to_vec(),
                protocol_version: protocol::PROTOCOL_VERSION,
                client_version: env!("CARGO_PKG_VERSION").into(),
                platform: std::env::consts::OS.into(),
            });
            let mut bytes = BytesMut::new();
            FrameCodec::Client.encode(
                Frame::unsequenced(packet.id(), packet.encode().into()),
                &mut bytes,
            )?;
            socket.write_all(&bytes).await?;
        }
    }
    let Some(frame) = FrameReader::new(socket, FrameCodec::Client).read().await? else {
        return Ok("peer_closed");
    };
    let packet = ServerPacket::decode(frame.message_id, &frame.payload)?;
    let ServerPacket::Close(close) = packet else {
        anyhow::bail!("unexpected server packet during handshake")
    };
    let reason = wire::CloseReason::try_from(close.reason)?;
    if matches!(
        (reason, scenario),
        (wire::CloseReason::IdleTimeout, Scenario::ConnectHold)
    ) {
        return Ok("auth_timeout");
    }
    protocol::close_reason_label(reason)
        .ok_or_else(|| anyhow::anyhow!("server sent an unspecified close reason"))
}
