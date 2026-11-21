use std::io::ErrorKind;
use std::net::SocketAddr;
use std::time::Duration;

use bytes::BytesMut;
use net::{CodecError, Frame, FrameCodec};
use protocol::{ClientPacket, MessageId, ServerPacket, wire};
use tokio::io::AsyncReadExt;
use tokio::net::TcpStream;
use tokio::net::tcp::OwnedReadHalf;
use tokio::sync::mpsc;
use tokio::time::Instant;
use tokio_util::codec::Decoder;
use tokio_util::sync::CancellationToken;
use tracing::Instrument;

use crate::config::ClientConfig;
use crate::writer::write_frames;

const PILOT_PROTOCOL_VERSION: u32 = 1;

#[derive(Debug, Clone, Copy)]
pub(super) struct ConnId(pub(super) u64);

pub(super) struct Peer {
    pub(super) id: ConnId,
    pub(super) address: SocketAddr,
    pub(super) accepted_at: Instant,
}

#[derive(Debug, Clone, Copy)]
pub(super) enum CloseCause {
    AuthTimeout,
    AuthFailed,
    ResumeUnknown,
    UpdateRequired,
    ProtocolViolation,
    OversizedFrame,
    IoError,
    PeerClosed,
    Shutdown,
}

impl CloseCause {
    pub(super) const ALL: [Self; 9] = [
        Self::AuthTimeout,
        Self::AuthFailed,
        Self::ResumeUnknown,
        Self::UpdateRequired,
        Self::ProtocolViolation,
        Self::OversizedFrame,
        Self::IoError,
        Self::PeerClosed,
        Self::Shutdown,
    ];

    pub(super) const fn label(self) -> &'static str {
        match self {
            Self::AuthTimeout => "auth_timeout",
            Self::AuthFailed => "auth_failed",
            Self::ResumeUnknown => "resume_unknown",
            Self::UpdateRequired => "update_required",
            Self::ProtocolViolation => "protocol_violation",
            Self::OversizedFrame => "oversized_frame",
            Self::IoError => "io_error",
            Self::PeerClosed => "peer_closed",
            Self::Shutdown => "shutdown",
        }
    }
}

#[derive(Clone, Copy)]
enum TokenVerifier {
    NoIssuer,
}

impl TokenVerifier {
    const fn refusal(self, _token: &[u8]) -> wire::CloseReason {
        match self {
            Self::NoIssuer => wire::CloseReason::AuthenticationFailed,
        }
    }
}

struct ActiveConnection(metrics::Gauge);

impl Drop for ActiveConnection {
    fn drop(&mut self) {
        self.0.decrement(1);
    }
}

pub(super) async fn run(
    socket: TcpStream,
    peer: Peer,
    config: ClientConfig,
    shutdown: CancellationToken,
) {
    let span = tracing::info_span!("connection", conn_id = peer.id.0, peer = %peer.address);
    async move {
        let active = ActiveConnection(metrics::gauge!("gateway_connections_current"));
        active.0.increment(1);
        metrics::counter!("gateway_connections_accepted_total").increment(1);
        if config.log_connections {
            tracing::info!("connection accepted");
        }

        let (read, write) = socket.into_split();
        let (sender, receiver) = mpsc::channel(config.write_queue_capacity.get());
        let deadline =
            peer.accepted_at + Duration::from_millis(u64::from(config.auth_timeout_ms.get()));
        let ((cause, frames_in), (sent, frames_out)) = tokio::join!(
            handshake(read, sender, (deadline, shutdown)),
            write_frames(write, receiver),
        );
        let cause = if sent { cause } else { CloseCause::IoError };
        metrics::counter!("gateway_connections_closed_total", "reason" => cause.label())
            .increment(1);
        if config.log_connections {
            let elapsed_ms = peer.accepted_at.elapsed().as_millis();
            tracing::info!(
                reason = cause.label(),
                elapsed_ms,
                frames_in,
                frames_out,
                "connection closed"
            );
        }
        drop(active);
    }
    .instrument(span)
    .await;
}

async fn handshake(
    mut reader: OwnedReadHalf,
    sender: mpsc::Sender<ServerPacket>,
    lifetime: (Instant, CancellationToken),
) -> (CloseCause, u64) {
    let (deadline, shutdown) = lifetime;
    let mut buffer = BytesMut::new();
    let mut frames = 0;
    let cause = tokio::select! {
        biased;
        () = shutdown.cancelled() => {
            send_close(&sender, wire::CloseReason::ServerShutdown, true).await;
            CloseCause::Shutdown
        }
        () = tokio::time::sleep_until(deadline) => {
            send_close(&sender, wire::CloseReason::IdleTimeout, false).await;
            CloseCause::AuthTimeout
        }
        received = read_frame(&mut reader, &mut buffer) => match received {
            Ok(Some(frame)) => {
                frames = 1;
                metrics::counter!("gateway_frames_received_total").increment(1);
                respond(frame, &sender).await
            }
            Ok(None) => CloseCause::PeerClosed,
            Err(CodecError::SizeLimit) => CloseCause::OversizedFrame,
            Err(CodecError::ProtocolViolation(_)) => CloseCause::ProtocolViolation,
            Err(CodecError::Io(error)) if error.kind() == ErrorKind::UnexpectedEof => CloseCause::ProtocolViolation,
            Err(CodecError::Io(_)) => CloseCause::IoError,
        }
    };
    (cause, frames)
}

async fn read_frame(
    reader: &mut OwnedReadHalf,
    buffer: &mut BytesMut,
) -> Result<Option<Frame>, CodecError> {
    loop {
        if let Some(frame) = FrameCodec::Gateway.decode(buffer)? {
            return Ok(Some(frame));
        }
        buffer.reserve(4096);
        let bytes = reader.read_buf(buffer).await?;
        if bytes == 0 {
            return FrameCodec::Gateway
                .decode_eof(buffer)
                .map_err(|error| match error {
                    CodecError::Io(_) => std::io::Error::from(ErrorKind::UnexpectedEof).into(),
                    CodecError::SizeLimit | CodecError::ProtocolViolation(_) => error,
                });
        }
        metrics::counter!("gateway_bytes_received_total").increment(bytes as u64);
    }
}

async fn respond(frame: Frame, sender: &mpsc::Sender<ServerPacket>) -> CloseCause {
    if !matches!(frame.message_id, MessageId::AUTH | MessageId::RESUME)
        || frame.flags.is_reliable()
        || frame.seq.value() != 0
        || frame.ack.value() != 0
    {
        return CloseCause::ProtocolViolation;
    }
    match ClientPacket::decode(frame.message_id, &frame.payload) {
        Ok(ClientPacket::Auth(auth)) => {
            metrics::counter!("gateway_auth_attempts_total").increment(1);
            if auth.protocol_version != PILOT_PROTOCOL_VERSION {
                send_close(sender, wire::CloseReason::UpdateRequired, false).await;
                metrics::counter!("gateway_auth_failed_total").increment(1);
                return CloseCause::UpdateRequired;
            }
            send_close(
                sender,
                TokenVerifier::NoIssuer.refusal(&auth.login_token),
                false,
            )
            .await;
            metrics::counter!("gateway_auth_failed_total").increment(1);
            CloseCause::AuthFailed
        }
        Ok(ClientPacket::Resume(resume)) => {
            let (reason, cause) = if resume.protocol_version == PILOT_PROTOCOL_VERSION {
                (wire::ResumeFailReason::Unknown, CloseCause::ResumeUnknown)
            } else {
                (
                    wire::ResumeFailReason::VersionMismatch,
                    CloseCause::UpdateRequired,
                )
            };
            if sender
                .send(ServerPacket::ResumeFail(wire::ResumeFail {
                    reason: i32::from(reason),
                }))
                .await
                .is_err()
            {
                return CloseCause::IoError;
            }
            if reason == wire::ResumeFailReason::VersionMismatch {
                send_close(sender, wire::CloseReason::UpdateRequired, false).await;
            }
            cause
        }
        Ok(
            ClientPacket::Ping(_)
            | ClientPacket::Ack(_)
            | ClientPacket::Suspend(_)
            | ClientPacket::Logout(_)
            | ClientPacket::EnterGame(_)
            | ClientPacket::BuyItem(_)
            | ClientPacket::InventoryList(_)
            | ClientPacket::SendMail(_)
            | ClientPacket::ListMail(_)
            | ClientPacket::ReceiveMail(_)
            | ClientPacket::EnterSpace(_)
            | ClientPacket::LeaveSpace(_)
            | ClientPacket::Move(_),
        )
        | Err(_) => CloseCause::ProtocolViolation,
    }
}

async fn send_close(
    sender: &mpsc::Sender<ServerPacket>,
    reason: wire::CloseReason,
    reconnect_allowed: bool,
) {
    let packet = ServerPacket::Close(wire::Close {
        reason: i32::from(reason),
        reconnect_allowed,
        message: None,
    });
    // A gone receiver already reports its failure through the writer result.
    let _ = sender.send(packet).await;
}
