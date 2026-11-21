//! Incremental frame reads with consistent treatment of a partial frame at EOF.

use std::io::ErrorKind;

use bytes::BytesMut;
use tokio::io::{AsyncRead, AsyncReadExt};
use tokio_util::codec::Decoder;

use crate::CodecError;

/// A transport reader, its decoder, and any bytes left for the following frame.
#[derive(Debug)]
pub struct FrameReader<R, C> {
    reader: R,
    codec: C,
    buffer: BytesMut,
    bytes: u64,
}

impl<R: AsyncRead + Unpin, C: Decoder<Error = CodecError>> FrameReader<R, C> {
    /// Takes ownership of the read half and the codec for this direction.
    pub fn new(reader: R, codec: C) -> Self {
        Self {
            reader,
            codec,
            buffer: BytesMut::new(),
            bytes: 0,
        }
    }

    /// Returns the bytes consumed from the transport, including incomplete frames.
    pub fn bytes_received(&self) -> u64 {
        self.bytes
    }

    /// Returns a decoded frame or clean EOF. Incomplete input at EOF is an error.
    pub async fn read(&mut self) -> Result<Option<C::Item>, CodecError> {
        loop {
            if let Some(item) = self.codec.decode(&mut self.buffer)? {
                return Ok(Some(item));
            }
            self.buffer.reserve(4096);
            let bytes = self.reader.read_buf(&mut self.buffer).await?;
            if bytes == 0 {
                return self
                    .codec
                    .decode_eof(&mut self.buffer)
                    .map_err(|error| match error {
                        CodecError::Io(_) => std::io::Error::from(ErrorKind::UnexpectedEof).into(),
                        CodecError::SizeLimit | CodecError::ProtocolViolation(_) => error,
                    });
            }
            self.bytes += bytes as u64;
        }
    }
}
