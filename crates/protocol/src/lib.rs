//! Pilot packet schemas, routing classification, and payload dispatch. This crate does no I/O.

mod id;
mod packets;

pub use id::{
    ControlId, Delivery, Direction, MessageId, MessageKind, MessageSpec, SequenceNumber, SessionId,
};
pub use packets::{ClientPacket, ControlPacket, MESSAGE_SPECS, ServerPacket};

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
