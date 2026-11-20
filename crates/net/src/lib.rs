//! Incremental framing for the client connection and backend link.

mod envelope;
mod frame;

pub use envelope::{ENVELOPE_HEADER_SIZE, Envelope, EnvelopeCodec, EnvelopeMessage};
pub use frame::{FRAME_HEADER_SIZE, Frame, FrameCodec, FrameFlags};

/// Maximum total wire size, including the corresponding header.
pub const MAX_PACKET_SIZE: usize = 64 * 1024;

/// Why framing could not continue.
#[derive(Debug, thiserror::Error)]
pub enum CodecError {
    /// Close immediately without sending a notice or reading the advertised body.
    #[error("packet exceeds maximum size of {MAX_PACKET_SIZE} bytes")]
    SizeLimit,
    /// Malformed wire input. Close without sending a Close notice.
    #[error(transparent)]
    ProtocolViolation(#[from] ProtocolViolation),
    /// Underlying transport failure, including an incomplete packet at EOF.
    #[error(transparent)]
    Io(#[from] std::io::Error),
}

/// Invalid packet header values.
#[derive(Debug, thiserror::Error)]
pub enum ProtocolViolation {
    /// The advertised length cannot even hold the header.
    #[error("packet length {0} is smaller than its header")]
    ShortLength(u32),
    /// Reserved (currently undefined) client frame bits are set.
    #[error("reserved frame flags in {0:#06x}")]
    ReservedFlags(u16),
    /// Only the server may send fragments.
    #[error("client sent a fragment")]
    ClientFragment,
    /// The envelope does not define this kind byte.
    #[error("unknown envelope kind {0}")]
    EnvelopeKind(u8),
}

fn decode_length(src: &[u8], header_size: usize) -> Result<Option<usize>, CodecError> {
    let Some(length) = src.first_chunk() else {
        return Ok(None);
    };
    let length = u32::from_le_bytes(*length);
    let size = usize::try_from(length).map_err(|_| CodecError::SizeLimit)?;
    if size > MAX_PACKET_SIZE {
        return Err(CodecError::SizeLimit);
    }
    if size < header_size {
        return Err(ProtocolViolation::ShortLength(length).into());
    }
    Ok(Some(size))
}

fn encode_length(payload_size: usize, header_size: usize) -> Result<(usize, u32), CodecError> {
    let size = payload_size.saturating_add(header_size);
    if size > MAX_PACKET_SIZE {
        return Err(CodecError::SizeLimit);
    }
    Ok((
        size,
        u32::try_from(size).map_err(|_| CodecError::SizeLimit)?,
    ))
}
