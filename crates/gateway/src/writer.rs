use std::io::ErrorKind;
use std::time::Duration;

use bytes::{Buf, BytesMut};
use net::{CodecError, Frame, FrameCodec, FrameFlags};
use protocol::{SequenceNumber, ServerPacket};
use tokio::io::AsyncWriteExt;
use tokio::net::tcp::OwnedWriteHalf;
use tokio::sync::mpsc;
use tokio_util::codec::Encoder;

pub(super) const FLUSH_TIMEOUT: Duration = Duration::from_secs(3);

pub(super) async fn write_frames(
    mut writer: OwnedWriteHalf,
    mut receiver: mpsc::Receiver<ServerPacket>,
) -> (bool, u64) {
    let Some(first) = receiver.recv().await else {
        return (true, 0);
    };
    let mut frames = 0;
    let sent = tokio::time::timeout(FLUSH_TIMEOUT, async {
        let mut next = Some(first);
        let mut bytes = BytesMut::new();
        while let Some(packet) = next {
            FrameCodec::Gateway.encode(
                Frame {
                    message_id: packet.id(),
                    flags: FrameFlags::NONE,
                    seq: SequenceNumber::new(0),
                    ack: SequenceNumber::new(0),
                    payload: packet.encode().into(),
                },
                &mut bytes,
            )?;
            while !bytes.is_empty() {
                let written = writer.write(&bytes).await?;
                if written == 0 {
                    return Err(std::io::Error::from(ErrorKind::WriteZero).into());
                }
                metrics::counter!("gateway_bytes_sent_total").increment(written as u64);
                bytes.advance(written);
            }
            metrics::counter!("gateway_frames_sent_total").increment(1);
            frames += 1;
            next = receiver.recv().await;
        }
        writer.shutdown().await.map_err(CodecError::Io)
    })
    .await
    .is_ok_and(|result| result.is_ok());
    (sent, frames)
}
