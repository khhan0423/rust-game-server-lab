//! Client frame framing, independent of the protobuf payload.

use bytes::{Buf, BufMut, Bytes, BytesMut};
use protocol::{MessageId, SequenceNumber};
use tokio_util::codec::{Decoder, Encoder};

use crate::{CodecError, ProtocolViolation, decode_length, encode_length};

/// Size of the common header, even for server fragments.
pub const FRAME_HEADER_SIZE: usize = 16;

/// Recognized frame flags. Reserved bits cannot be set through the public API.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameFlags(u16);

impl FrameFlags {
    /// No fragment and no replay.
    pub const NONE: Self = Self(0);
    /// Payload starts with a fragment header.
    pub const FRAGMENT: Self = Self(1);
    /// Frame participates in reliable delivery.
    pub const RELIABLE: Self = Self(2);
    /// A fragment that participates in reliable delivery.
    pub const RELIABLE_FRAGMENT: Self = Self(Self::FRAGMENT.0 | Self::RELIABLE.0);

    /// Returns the wire bits.
    pub const fn bits(self) -> u16 {
        self.0
    }

    /// Returns whether the payload starts with a fragment header.
    pub const fn is_fragment(self) -> bool {
        self.0 & Self::FRAGMENT.0 != 0
    }

    /// Returns whether the frame is sequenced for replay.
    pub const fn is_reliable(self) -> bool {
        self.0 & Self::RELIABLE.0 != 0
    }
}

/// One client protocol frame. Total length is derived from the payload size.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Frame {
    /// Routing identifier, even if the codec does not know the payload type.
    pub message_id: MessageId,
    /// Wire flags.
    pub flags: FrameFlags,
    /// Reliable frame sequence, or zero for an unsequenced frame.
    pub seq: SequenceNumber,
    /// Last processed peer sequence.
    pub ack: SequenceNumber,
    /// Opaque payload (including the fragment header when present).
    pub payload: Bytes,
}

/// Client protocol codec at either endpoint of the connection.
#[derive(Debug, Clone, Copy)]
pub enum FrameCodec {
    /// Decodes client frames and encodes server frames.
    Gateway,
    /// Decodes server frames and encodes client frames.
    Client,
}

impl Decoder for FrameCodec {
    type Item = Frame;
    type Error = CodecError;

    fn decode(&mut self, src: &mut BytesMut) -> Result<Option<Frame>, CodecError> {
        let Some(size) = decode_length(src, FRAME_HEADER_SIZE)? else {
            return Ok(None);
        };
        let Some(mut header) = src.get(4..FRAME_HEADER_SIZE) else {
            return Ok(None);
        };
        let message_id = MessageId::new(header.get_u16_le());
        let flags = FrameFlags(header.get_u16_le());
        if flags.bits() & !FrameFlags::RELIABLE_FRAGMENT.bits() != 0 {
            return Err(ProtocolViolation::ReservedFlags(flags.bits()).into());
        }
        match self {
            Self::Gateway if flags.is_fragment() => {
                return Err(ProtocolViolation::ClientFragment.into());
            }
            Self::Gateway | Self::Client => {}
        }
        if src.len() < size {
            return Ok(None);
        }
        let seq = SequenceNumber::new(header.get_u32_le());
        let ack = SequenceNumber::new(header.get_u32_le());
        let mut payload = src.split_to(size).freeze();
        payload.advance(FRAME_HEADER_SIZE);
        Ok(Some(Frame {
            message_id,
            flags,
            seq,
            ack,
            payload,
        }))
    }
}

impl Encoder<Frame> for FrameCodec {
    type Error = CodecError;

    fn encode(&mut self, frame: Frame, dst: &mut BytesMut) -> Result<(), CodecError> {
        let (size, len) = encode_length(frame.payload.len(), FRAME_HEADER_SIZE)?;
        match self {
            Self::Client if frame.flags.is_fragment() => {
                return Err(ProtocolViolation::ClientFragment.into());
            }
            Self::Client | Self::Gateway => {}
        }
        dst.reserve(size);
        dst.put_u32_le(len);
        dst.put_u16_le(frame.message_id.value());
        dst.put_u16_le(frame.flags.bits());
        dst.put_u32_le(frame.seq.value());
        dst.put_u32_le(frame.ack.value());
        dst.extend_from_slice(&frame.payload);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::MAX_PACKET_SIZE;

    fn frame(seq: u32) -> Frame {
        Frame {
            message_id: MessageId::new(u16::MAX),
            flags: FrameFlags::RELIABLE,
            seq: SequenceNumber::new(seq),
            ack: SequenceNumber::new(0xfedc_ba98),
            payload: Bytes::from_static(b"pilot"),
        }
    }

    #[test]
    fn frame_header_matches_the_wire_at_field_boundaries() {
        let mut packet = frame(u32::MAX);
        packet.ack = SequenceNumber::new(u32::MAX);
        let mut bytes = BytesMut::new();
        FrameCodec::Client
            .encode(packet.clone(), &mut bytes)
            .unwrap();
        assert_eq!(
            bytes.as_ref(),
            b"\x15\x00\x00\x00\xff\xff\x02\x00\xff\xff\xff\xff\xff\xff\xff\xffpilot"
        );
        assert_eq!(
            FrameCodec::Gateway.decode(&mut bytes).unwrap(),
            Some(packet)
        );
    }

    #[test]
    fn oversized_frame_is_rejected_from_length_alone_without_reserving() {
        let mut bytes = BytesMut::from(&[1, 0, 1, 0][..]);
        let capacity = bytes.capacity();
        assert!(matches!(
            FrameCodec::Gateway.decode(&mut bytes),
            Err(CodecError::SizeLimit)
        ));
        assert_eq!(bytes.capacity(), capacity);
        assert_eq!(bytes.as_ref(), &[1, 0, 1, 0]);
    }

    #[test]
    fn frames_shorter_than_the_header_are_rejected() {
        for len in [0_u8, 1, 15] {
            let mut bytes = BytesMut::from(&[len, 0, 0, 0][..]);
            assert!(matches!(FrameCodec::Gateway.decode(&mut bytes),
                Err(CodecError::ProtocolViolation(ProtocolViolation::ShortLength(actual))) if actual == u32::from(len)));
        }
    }

    #[test]
    fn feeding_one_byte_at_a_time_yields_one_complete_frame() {
        let packet = frame(7);
        let mut whole = BytesMut::new();
        FrameCodec::Client
            .encode(packet.clone(), &mut whole)
            .unwrap();
        let mut input = BytesMut::new();
        for (idx, byte) in whole.iter().copied().enumerate() {
            input.put_u8(byte);
            let expected = if idx + 1 == whole.len() {
                Some(packet.clone())
            } else {
                None
            };
            assert_eq!(FrameCodec::Gateway.decode(&mut input).unwrap(), expected);
        }
        assert_eq!(FrameCodec::Gateway.decode(&mut input).unwrap(), None);
    }

    #[test]
    fn coalesced_frames_are_returned_in_order() {
        let packets = [frame(7), frame(8), frame(9)];
        let mut bytes = BytesMut::new();
        for packet in &packets {
            FrameCodec::Client
                .encode(packet.clone(), &mut bytes)
                .unwrap();
        }
        for packet in packets {
            assert_eq!(
                FrameCodec::Gateway.decode(&mut bytes).unwrap(),
                Some(packet)
            );
        }
        assert_eq!(FrameCodec::Gateway.decode(&mut bytes).unwrap(), None);
    }

    #[test]
    fn fragments_are_rejected_from_clients_and_allowed_from_servers() {
        let mut packet = frame(7);
        packet.flags = FrameFlags::RELIABLE_FRAGMENT;
        let mut bytes = BytesMut::new();
        FrameCodec::Gateway
            .encode(packet.clone(), &mut bytes)
            .unwrap();
        assert!(matches!(
            FrameCodec::Gateway.decode(&mut bytes),
            Err(CodecError::ProtocolViolation(
                ProtocolViolation::ClientFragment
            ))
        ));
        assert_eq!(
            FrameCodec::Client.decode(&mut bytes).unwrap(),
            Some(packet.clone())
        );
        assert!(matches!(
            FrameCodec::Client.encode(packet, &mut bytes),
            Err(CodecError::ProtocolViolation(
                ProtocolViolation::ClientFragment
            ))
        ));
    }

    #[test]
    fn reserved_flags_are_rejected_before_the_body_arrives() {
        let mut bytes = BytesMut::from(
            &b"\x15\x00\x00\x00\xff\xff\x04\x00\x00\x00\x00\x00\x00\x00\x00\x00"[..],
        );
        assert!(matches!(
            FrameCodec::Gateway.decode(&mut bytes),
            Err(CodecError::ProtocolViolation(
                ProtocolViolation::ReservedFlags(4)
            ))
        ));
    }

    #[test]
    fn encoding_checks_the_size_before_appending_bytes() {
        let mut bytes = BytesMut::new();
        let mut packet = frame(7);
        packet.payload = Bytes::from(vec![0; MAX_PACKET_SIZE - FRAME_HEADER_SIZE + 1]);
        assert!(matches!(
            FrameCodec::Client.encode(packet, &mut bytes),
            Err(CodecError::SizeLimit)
        ));
        assert!(bytes.is_empty());
    }
}
