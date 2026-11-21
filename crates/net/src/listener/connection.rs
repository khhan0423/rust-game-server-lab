//! Per-connection transport and bounded output queue.

use super::{ListenerConfig, ListenerMetrics};
use crate::{CodecError, FrameReader};
use bytes::{Buf, BytesMut};
use std::{io, net::SocketAddr, sync::Arc, time::Duration};
use tokio::{
    io::AsyncWriteExt,
    net::{
        TcpStream,
        tcp::{OwnedReadHalf, OwnedWriteHalf},
    },
    sync::mpsc,
    time::Instant,
};
use tokio_util::{
    codec::{Decoder, Encoder},
    sync::CancellationToken,
};
use tracing::Instrument;

/// Monotonic identifier assigned only after admission to one listener.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ConnId(u64);
impl ConnId {
    pub(super) const fn new(value: u64) -> Self {
        Self(value)
    }
    /// Returns the listener-local number.
    pub const fn value(self) -> u64 {
        self.0
    }
}

/// Handler-defined close reasons used to initialize and increment metrics.
pub trait CloseLabel: Copy + Send + Sync + 'static {
    /// All possible application reasons.
    const ALL: &'static [Self];
    /// Stable metric label.
    fn label(self) -> &'static str;
}

/// Transport or application reason that ended this connection.
#[derive(Clone, Copy, Debug)]
pub enum CloseCause<R> {
    PeerClosed,
    ProtocolViolation,
    OversizedFrame,
    IoError,
    WriteTimeout,
    Shutdown,
    Handler(R),
}

impl<R: CloseLabel> CloseCause<R> {
    pub(super) const TRANSPORT: [Self; 6] = [
        Self::PeerClosed,
        Self::ProtocolViolation,
        Self::OversizedFrame,
        Self::IoError,
        Self::WriteTimeout,
        Self::Shutdown,
    ];
    pub(super) fn label(self) -> &'static str {
        match self {
            Self::PeerClosed => "peer_closed",
            Self::ProtocolViolation => "protocol_violation",
            Self::OversizedFrame => "oversized_frame",
            Self::IoError => "io_error",
            Self::WriteTimeout => "write_timeout",
            Self::Shutdown => "shutdown",
            Self::Handler(reason) => reason.label(),
        }
    }
}

impl<R> From<CodecError> for CloseCause<R> {
    fn from(error: CodecError) -> Self {
        match error {
            CodecError::SizeLimit => Self::OversizedFrame,
            CodecError::ProtocolViolation(_) => Self::ProtocolViolation,
            CodecError::Io(error) if error.kind() == io::ErrorKind::UnexpectedEof => {
                Self::ProtocolViolation
            }
            CodecError::Io(_) => Self::IoError,
        }
    }
}

/// Bounded queue drained by the connection's sole socket writer.
#[derive(Debug)]
pub struct Outbox<T>(mpsc::Sender<T>);
impl<T> Outbox<T> {
    /// Waits for space or returns the undelivered item when the connection closes.
    pub async fn send(&self, item: T) -> Result<(), mpsc::error::SendError<T>> {
        self.0.send(item).await
    }
    /// Queues a frame without waiting for space.
    pub fn try_send(&self, item: T) -> Result<(), mpsc::error::TrySendError<T>> {
        self.0.try_send(item)
    }
}

/// Framed input and bounded output for one admitted peer.
#[derive(Debug)]
pub struct Connection<C, T> {
    id: ConnId,
    peer: SocketAddr,
    deadline: Instant,
    reader: FrameReader<OwnedReadHalf, C>,
    outbox: Outbox<T>,
    metrics: Arc<ListenerMetrics>,
    frames_in: u64,
}
impl<C: Decoder<Error = CodecError>, T> Connection<C, T> {
    /// Unique number on this listener.
    pub const fn id(&self) -> ConnId {
        self.id
    }
    /// Remote address of the socket.
    pub const fn peer(&self) -> SocketAddr {
        self.peer
    }
    /// Absolute handshake deadline measured from acceptance.
    pub const fn deadline(&self) -> Instant {
        self.deadline
    }
    /// Bounded sender for this connection.
    pub const fn outbox(&self) -> &Outbox<T> {
        &self.outbox
    }
    /// Reads the next frame and records successful decodes.
    pub async fn read(&mut self) -> Result<Option<C::Item>, CodecError> {
        let item = self.reader.read().await?;
        if item.is_some() {
            self.metrics.frames_in.increment(1);
            self.frames_in += 1;
        }
        Ok(item)
    }
}

/// Protocol-specific behavior within the listener's transport lifecycle.
pub trait ConnectionHandler: Send + Sync + 'static {
    /// Codec selected by this port.
    type Codec: Decoder<Error = CodecError> + Encoder<Self::Out, Error = CodecError> + Send;
    /// Outgoing frame type.
    type Out: Send;
    /// Protocol-specific close reasons.
    type Reason: CloseLabel;
    /// Creates a fresh codec for each half of the connection.
    fn codec(&self) -> Self::Codec;
    /// Processes frames and responds to shutdown.
    fn run(
        &self,
        conn: &mut Connection<Self::Codec, Self::Out>,
        closing: CancellationToken,
    ) -> impl Future<Output = CloseCause<Self::Reason>> + Send;
}

struct Active(metrics::Gauge);
impl Drop for Active {
    fn drop(&mut self) {
        self.0.decrement(1);
    }
}

pub(super) struct Peer {
    pub id: ConnId,
    pub addr: SocketAddr,
    pub accepted: Instant,
}

pub(super) async fn serve<H: ConnectionHandler>(
    socket: TcpStream,
    peer: Peer,
    config: ListenerConfig,
    metrics: Arc<ListenerMetrics>,
    handler: Arc<H>,
    closing: CancellationToken,
) {
    let span = tracing::info_span!("connection", listener=metrics.name, conn_id=peer.id.value(), peer=%peer.addr);
    async move {
        let active = Active(metrics.current.clone());
        active.0.increment(1);
        metrics.accepted.increment(1);
        if config.log_connections {
            tracing::info!("connection accepted");
        }
        let (read, write) = socket.into_split();
        let (sender, receiver) = mpsc::channel(config.write_queue_capacity.get());
        let mut conn = Connection {
            id: peer.id,
            peer: peer.addr,
            deadline: peer.accepted
                + Duration::from_millis(u64::from(config.auth_timeout_ms.get())),
            reader: FrameReader::new(read, handler.codec()),
            outbox: Outbox(sender),
            metrics: Arc::clone(&metrics),
            frames_in: 0,
        };
        let mut frames_out = 0;
        let handler_task = async {
            let cause = handler.run(&mut conn, closing).await;
            let frames_in = conn.frames_in;
            metrics.bytes_in.increment(conn.reader.bytes_received());
            drop(conn);
            (cause, frames_in)
        };
        let ((reason, frames_in), written) = tokio::join!(
            handler_task,
            write_frames(write, receiver, handler.codec(), &metrics, &mut frames_out)
        );
        let cause = written.err().unwrap_or(reason);
        metrics.closed(cause.label()).increment(1);
        if config.log_connections {
            tracing::info!(
                reason = cause.label(),
                elapsed_ms = peer.accepted.elapsed().as_millis(),
                frames_in,
                frames_out,
                "connection closed"
            );
        }
    }
    .instrument(span)
    .await;
}

pub(super) const FLUSH_TIMEOUT: Duration = Duration::from_secs(3);

async fn write_frames<C: Encoder<T, Error = CodecError>, T, R>(
    mut writer: OwnedWriteHalf,
    mut queue: mpsc::Receiver<T>,
    mut codec: C,
    metrics: &ListenerMetrics,
    frames: &mut u64,
) -> Result<(), CloseCause<R>> {
    let mut bytes = BytesMut::new();
    while let Some(frame) = queue.recv().await {
        codec.encode(frame, &mut bytes)?;
        tokio::time::timeout(FLUSH_TIMEOUT, async {
            while !bytes.is_empty() {
                let count = writer.write(&bytes).await?;
                if count == 0 {
                    return Err(io::Error::from(io::ErrorKind::WriteZero));
                }
                metrics.bytes_out.increment(count as u64);
                bytes.advance(count);
            }
            Ok(())
        })
        .await
        .map_err(|_| CloseCause::WriteTimeout)?
        .map_err(|_| CloseCause::IoError)?;
        metrics.frames_out.increment(1);
        *frames += 1;
    }
    writer.shutdown().await.map_err(|_| CloseCause::IoError)
}
