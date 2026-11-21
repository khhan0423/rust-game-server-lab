use super::*;
use crate::{Frame, FrameCodec};
use protocol::MessageId;
use tokio::{io::AsyncWriteExt, net::TcpStream};
use tokio_util::codec::Encoder;

#[derive(Clone, Copy)]
enum EchoEnd {
    Replied,
}
impl CloseLabel for EchoEnd {
    const ALL: &'static [Self] = &[Self::Replied];
    fn label(self) -> &'static str {
        match self {
            Self::Replied => "replied",
        }
    }
}

struct Echo(u8);
impl ConnectionHandler for Echo {
    type Codec = FrameCodec;
    type Out = Frame;
    type Reason = EchoEnd;
    fn codec(&self) -> FrameCodec {
        FrameCodec::Gateway
    }
    async fn run(
        &self,
        conn: &mut Connection<FrameCodec, Frame>,
        _: CancellationToken,
    ) -> CloseCause<EchoEnd> {
        assert_eq!(conn.id().value(), 1);
        assert!(conn.peer().ip().is_loopback());
        let mut frame = conn.read().await.unwrap().unwrap();
        assert_eq!(frame.payload.as_ref(), &[42]);
        frame.payload = vec![self.0].into();
        conn.outbox().send(frame).await.unwrap();
        CloseCause::Handler(EchoEnd::Replied)
    }
}

#[tokio::test]
async fn two_listeners_serve_independent_handlers_and_identifiers() {
    let config = toml::from_str("listen_addr='127.0.0.1:0'\nmax_connections=2\nauth_timeout_ms=5000\nwrite_queue_capacity=1\nlog_connections=false").unwrap();
    let first = Listener::bind("first", config).await.unwrap();
    let second = Listener::bind("second", config).await.unwrap();
    let addresses = [first.local_addr(), second.local_addr()];
    let stop = CancellationToken::new();
    let tasks = [
        tokio::spawn(first.serve(Echo(1), stop.clone())),
        tokio::spawn(second.serve(Echo(2), stop.clone())),
    ];
    let query = |addr| async move {
        let mut socket = TcpStream::connect(addr).await.unwrap();
        let mut bytes = bytes::BytesMut::new();
        FrameCodec::Client
            .encode(
                Frame::unsequenced(MessageId::new(1), vec![42].into()),
                &mut bytes,
            )
            .unwrap();
        socket.write_all(&bytes).await.unwrap();
        crate::FrameReader::new(socket, FrameCodec::Client)
            .read()
            .await
            .unwrap()
            .unwrap()
            .payload
    };
    let (a, b) = tokio::join!(query(addresses[0]), query(addresses[1]));
    assert_eq!((a.as_ref(), b.as_ref()), (&[1][..], &[2][..]));
    stop.cancel();
    for task in tasks {
        task.await.unwrap().unwrap();
    }
}
