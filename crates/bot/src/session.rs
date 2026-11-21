use std::net::SocketAddr;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use bytes::BytesMut;
use net::{Frame, FrameCodec, FrameFlags};
use protocol::{ClientPacket, SequenceNumber, ServerPacket, wire};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::time::Instant;
use tokio_util::codec::{Decoder, Encoder};
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
    let mut bytes = BytesMut::new();
    match scenario {
        Scenario::ConnectHold => {}
        Scenario::ConnectAuth => {
            let packet = ClientPacket::Auth(wire::Auth {
                login_token: rand::random::<[u8; 32]>().to_vec(),
                protocol_version: 1,
                client_version: env!("CARGO_PKG_VERSION").into(),
                platform: std::env::consts::OS.into(),
            });
            FrameCodec::Client.encode(
                Frame {
                    message_id: packet.id(),
                    flags: FrameFlags::NONE,
                    seq: SequenceNumber::new(0),
                    ack: SequenceNumber::new(0),
                    payload: packet.encode().into(),
                },
                &mut bytes,
            )?;
            socket.write_all(&bytes).await?;
            bytes.clear();
        }
    }
    loop {
        if let Some(frame) = FrameCodec::Client.decode(&mut bytes)? {
            let packet = ServerPacket::decode(frame.message_id, &frame.payload)?;
            if let ServerPacket::Close(close) = packet {
                return close_label(wire::CloseReason::try_from(close.reason)?, scenario);
            }
            anyhow::bail!("unexpected server packet during handshake");
        }
        bytes.reserve(4096);
        if socket.read_buf(&mut bytes).await? == 0 {
            anyhow::ensure!(bytes.is_empty(), "incomplete frame at end of stream");
            return Ok("peer_closed");
        }
    }
}

fn close_label(reason: wire::CloseReason, scenario: Scenario) -> anyhow::Result<&'static str> {
    Ok(match reason {
        wire::CloseReason::Unspecified => anyhow::bail!("server sent an unspecified close reason"),
        wire::CloseReason::Logout => "logout",
        wire::CloseReason::DuplicateLogin => "duplicate_login",
        wire::CloseReason::Maintenance => "maintenance",
        wire::CloseReason::UpdateRequired => "update_required",
        wire::CloseReason::Banned => "banned",
        wire::CloseReason::ServerShutdown => "shutdown",
        wire::CloseReason::IdleTimeout => match scenario {
            Scenario::ConnectHold => "auth_timeout",
            Scenario::ConnectAuth => "idle_timeout",
        },
        wire::CloseReason::AuthenticationFailed => "auth_failed",
    })
}
