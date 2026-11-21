use net::{CloseCause, CloseLabel, Connection, ConnectionHandler, Frame, FrameCodec, Outbox};
use protocol::{ClientPacket, HandshakeClose, MessageId, ServerPacket, wire};
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, Copy)]
pub(super) enum Reason {
    AuthTimeout,
    AuthFailed,
    ResumeUnknown,
    UpdateRequired,
}
impl CloseLabel for Reason {
    const ALL: &'static [Self] = &[
        Self::AuthTimeout,
        Self::AuthFailed,
        Self::ResumeUnknown,
        Self::UpdateRequired,
    ];
    fn label(self) -> &'static str {
        match self {
            Self::AuthTimeout => HandshakeClose::AuthTimeout.label(),
            Self::AuthFailed => HandshakeClose::AuthFailed.label(),
            Self::ResumeUnknown => "resume_unknown",
            Self::UpdateRequired => HandshakeClose::UpdateRequired.label(),
        }
    }
}
type Cause = CloseCause<Reason>;
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

pub(super) struct ClientHandshake;
impl ConnectionHandler for ClientHandshake {
    type Codec = FrameCodec;
    type Out = Frame;
    type Reason = Reason;
    fn codec(&self) -> FrameCodec {
        FrameCodec::Gateway
    }
    async fn run(
        &self,
        conn: &mut Connection<FrameCodec, Frame>,
        shutdown: CancellationToken,
    ) -> Cause {
        tokio::select! {
            biased;
            ()=shutdown.cancelled()=>{send_close(conn.outbox(), wire::CloseReason::ServerShutdown,true).await; Cause::Shutdown}
            ()=tokio::time::sleep_until(conn.deadline())=>{send_close(conn.outbox(),wire::CloseReason::IdleTimeout,false).await; Cause::Handler(Reason::AuthTimeout)}
            result=conn.read()=>match result {Ok(Some(frame))=>respond(frame,conn.outbox()).await,Ok(None)=>Cause::PeerClosed,Err(error)=>error.into()}
        }
    }
}
async fn send(
    outbox: &Outbox<Frame>,
    packet: ServerPacket,
) -> Result<(), tokio::sync::mpsc::error::SendError<Frame>> {
    outbox
        .send(Frame::unsequenced(packet.id(), packet.encode().into()))
        .await
}
async fn respond(frame: Frame, sender: &Outbox<Frame>) -> Cause {
    if !matches!(frame.message_id, MessageId::AUTH | MessageId::RESUME)
        || frame.flags.is_reliable()
        || frame.seq.value() != 0
        || frame.ack.value() != 0
    {
        return Cause::ProtocolViolation;
    }
    match ClientPacket::decode(frame.message_id, &frame.payload) {
        Ok(ClientPacket::Auth(auth)) => {
            metrics::counter!("gateway_auth_attempts_total").increment(1);
            if auth.protocol_version != protocol::PROTOCOL_VERSION {
                send_close(sender, wire::CloseReason::UpdateRequired, false).await;
                metrics::counter!("gateway_auth_failed_total").increment(1);
                return Cause::Handler(Reason::UpdateRequired);
            }
            send_close(
                sender,
                TokenVerifier::NoIssuer.refusal(&auth.login_token),
                false,
            )
            .await;
            metrics::counter!("gateway_auth_failed_total").increment(1);
            Cause::Handler(Reason::AuthFailed)
        }
        Ok(ClientPacket::Resume(resume)) => {
            let (reason, cause) = if resume.protocol_version == protocol::PROTOCOL_VERSION {
                (
                    wire::ResumeFailReason::Unknown,
                    Cause::Handler(Reason::ResumeUnknown),
                )
            } else {
                (
                    wire::ResumeFailReason::VersionMismatch,
                    Cause::Handler(Reason::UpdateRequired),
                )
            };
            if send(
                sender,
                ServerPacket::ResumeFail(wire::ResumeFail {
                    reason: i32::from(reason),
                }),
            )
            .await
            .is_err()
            {
                return Cause::IoError;
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
        | Err(_) => Cause::ProtocolViolation,
    }
}

async fn send_close(sender: &Outbox<Frame>, reason: wire::CloseReason, reconnect_allowed: bool) {
    let packet = ServerPacket::Close(wire::Close {
        reason: i32::from(reason),
        reconnect_allowed,
        message: None,
    });
    // A gone receiver already reports its failure through the writer result.
    let _ = send(sender, packet).await;
}
