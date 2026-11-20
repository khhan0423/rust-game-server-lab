//! Wire envelope for packets on a backend link.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use protocol::{ControlId, MessageId, SessionId};
use tokio_util::codec::{Decoder, Encoder};

use crate::{CodecError, ProtocolViolation, decode_length, encode_length};

/// Size of the envelope header (the fields have no padding).
pub const ENVELOPE_HEADER_SIZE: usize = 15;

/// The kind byte and its matching message ID namespace.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EnvelopeMessage {
    /// Kind 0: relay a client protocol message.
    Relay(MessageId),
    /// Kind 1: carry a server-only control.
    Control(ControlId),
}

/// One server-to-server envelope with an opaque payload.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Envelope {
    /// Client session, or zero for a link-level message.
    pub session: SessionId,
    /// The kind and corresponding wire message ID.
    pub message: EnvelopeMessage,
    /// Uninterpreted message bytes.
    pub payload: Bytes,
}

/// Incremental codec for the Gateway/backend link.
#[derive(Debug, Clone, Copy, Default)]
pub struct EnvelopeCodec;

impl Decoder for EnvelopeCodec {
    type Item = Envelope;
    type Error = CodecError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Envelope>, CodecError> {
        let Some(size) = decode_length(src, ENVELOPE_HEADER_SIZE)? else {
            return Ok(None);
        };
        let Some(mut header) = src.get(4..ENVELOPE_HEADER_SIZE) else {
            return Ok(None);
        };
        let kind = header.get_u8();
        let session = SessionId::new(header.get_u64_le());
        let id = header.get_u16_le();
        let message = match kind {
            0 => EnvelopeMessage::Relay(MessageId::new(id)),
            1 => EnvelopeMessage::Control(ControlId::new(id)),
            _ => return Err(ProtocolViolation::EnvelopeKind(kind).into()),
        };
        if src.len() < size {
            return Ok(None);
        }
        let mut payload = src.split_to(size).freeze();
        payload.advance(ENVELOPE_HEADER_SIZE);
        Ok(Some(Envelope {
            session,
            message,
            payload,
        }))
    }
}

impl Encoder<Envelope> for EnvelopeCodec {
    type Error = CodecError;

    fn encode(&mut self, envelope: Envelope, dst: &mut BytesMut) -> Result<(), CodecError> {
        let (size, len) = encode_length(envelope.payload.len(), ENVELOPE_HEADER_SIZE)?;
        let (kind, msg) = match envelope.message {
            EnvelopeMessage::Relay(id) => (0, id.value()),
            EnvelopeMessage::Control(id) => (1, id.value()),
        };
        dst.reserve(size);
        dst.put_u32_le(len);
        dst.put_u8(kind);
        dst.put_u64_le(envelope.session.value());
        dst.put_u16_le(msg);
        dst.extend_from_slice(&envelope.payload);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn both_envelope_kinds_round_trip_in_the_fixed_wire_order() {
        for (message, kind) in [
            (EnvelopeMessage::Relay(MessageId::new(0x8123)), 0),
            (EnvelopeMessage::Control(ControlId::new(0x8123)), 1),
        ] {
            let packet = Envelope {
                session: SessionId::new(0x0102_0304_0506_0708),
                message,
                payload: Bytes::from_static(b"ab"),
            };
            let mut bytes = BytesMut::new();
            EnvelopeCodec.encode(packet.clone(), &mut bytes).unwrap();
            assert_eq!(
                bytes.as_ref(),
                &[
                    17, 0, 0, 0, kind, 8, 7, 6, 5, 4, 3, 2, 1, 0x23, 0x81, b'a', b'b'
                ]
            );
            assert_eq!(EnvelopeCodec.decode(&mut bytes).unwrap(), Some(packet));
        }
    }

    #[test]
    fn undefined_envelope_kind_is_rejected() {
        let mut bytes = BytesMut::from(&[15, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0][..]);
        assert!(matches!(
            EnvelopeCodec.decode(&mut bytes),
            Err(CodecError::ProtocolViolation(
                ProtocolViolation::EnvelopeKind(2)
            ))
        ));
    }
}
