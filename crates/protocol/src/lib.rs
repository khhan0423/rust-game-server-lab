//! Pilot packet schemas, routing classification, and payload dispatch. This crate does no I/O.

mod id;
mod packets;

pub use id::{
    ControlId, Delivery, Direction, MessageId, MessageKind, MessageSpec, SequenceNumber, SessionId,
};
pub use packets::{ClientPacket, ControlPacket, MESSAGE_SPECS, ServerPacket};

/// Version advertised and accepted by the current session handshake.
pub const PROTOCOL_VERSION: u32 = 1;

/// Diagnostic close labels shared by the gateway handshake and load bot.
#[derive(Debug, Clone, Copy)]
pub enum HandshakeClose {
    /// No handshake arrived before the deadline.
    AuthTimeout,
    /// The token was refused.
    AuthFailed,
    /// The peer uses an incompatible protocol version.
    UpdateRequired,
}

impl HandshakeClose {
    /// Label used in diagnostics for this handshake outcome.
    pub const fn label(self) -> &'static str {
        match self {
            Self::AuthTimeout => "auth_timeout",
            Self::AuthFailed => "auth_failed",
            Self::UpdateRequired => "update_required",
        }
    }
}

/// Stable diagnostic label for a specified wire close reason.
pub const fn close_reason_label(reason: wire::CloseReason) -> Option<&'static str> {
    use wire::CloseReason as R;
    Some(match reason {
        R::Unspecified => return None,
        R::Logout => "logout",
        R::DuplicateLogin => "duplicate_login",
        R::Maintenance => "maintenance",
        R::UpdateRequired => HandshakeClose::UpdateRequired.label(),
        R::Banned => "banned",
        R::ServerShutdown => "shutdown",
        R::IdleTimeout => "idle_timeout",
        R::AuthenticationFailed => HandshakeClose::AuthFailed.label(),
    })
}

/// Protobuf payloads. Session and game rules are applied after wire decoding.
#[expect(
    clippy::doc_markdown,
    reason = "prost generates enum docs with unquoted ProtoBuf"
)]
pub mod wire {
    include!(concat!(env!("OUT_DIR"), "/pilot.rs"));
}

/// Why a payload could not be decoded.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolError {
    /// The ID does not name a client or server packet in the declared direction.
    #[error("unknown or wrong-direction message ID: {0:#06x}")]
    MessageId(u16),
    /// The ID does not name a control packet.
    #[error("unknown control ID: {0:#06x}")]
    ControlId(u16),
    /// The payload is invalid protobuf for the selected type.
    #[error("invalid protobuf payload")]
    Decode(#[from] prost::DecodeError),
}
