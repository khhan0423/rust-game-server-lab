//! Named TCP listeners with independent limits, queues and shutdown.
mod connection;
#[cfg(test)]
mod tests;
pub use connection::{CloseCause, CloseLabel, ConnId, Connection, ConnectionHandler, Outbox};
use serde::Deserialize;
use std::{
    collections::HashMap,
    io,
    net::{IpAddr, SocketAddr},
    num::{NonZeroU32, NonZeroUsize},
    sync::Arc,
    time::Duration,
};
use tokio::{
    net::TcpListener,
    task::{JoinError, JoinSet},
    time::{Instant, MissedTickBehavior},
};
use tokio_util::sync::CancellationToken;

/// Settings for a single TCP port.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ListenerConfig {
    listen_addr: SocketAddr,
    max_connections: NonZeroUsize,
    auth_timeout_ms: NonZeroU32,
    #[serde(deserialize_with = "queue_capacity")]
    write_queue_capacity: NonZeroUsize,
    #[serde(default = "default_rate")]
    connections_per_ip_per_sec: NonZeroU32,
    log_connections: bool,
}
fn default_rate() -> NonZeroU32 {
    NonZeroU32::new(1000).unwrap_or(NonZeroU32::MIN)
}
fn queue_capacity<'de, D: serde::Deserializer<'de>>(de: D) -> Result<NonZeroUsize, D::Error> {
    let n = NonZeroUsize::deserialize(de)?;
    if n.get() > tokio::sync::Semaphore::MAX_PERMITS {
        return Err(serde::de::Error::custom(
            "write_queue_capacity exceeds channel capacity limit",
        ));
    }
    Ok(n)
}

#[derive(Debug)]
struct ListenerMetrics {
    name: &'static str,
    current: metrics::Gauge,
    accepted: metrics::Counter,
    frames_in: metrics::Counter,
    frames_out: metrics::Counter,
    bytes_in: metrics::Counter,
    bytes_out: metrics::Counter,
}
impl ListenerMetrics {
    fn new(name: &'static str) -> Self {
        let m = Self {
            name,
            current: metrics::gauge!("net_connections_current", "listener"=>name),
            accepted: metrics::counter!("net_connections_accepted_total", "listener"=>name),
            frames_in: metrics::counter!("net_frames_received_total", "listener"=>name),
            frames_out: metrics::counter!("net_frames_sent_total", "listener"=>name),
            bytes_in: metrics::counter!("net_bytes_received_total", "listener"=>name),
            bytes_out: metrics::counter!("net_bytes_sent_total", "listener"=>name),
        };
        m.current.set(0);
        for counter in [
            &m.accepted,
            &m.frames_in,
            &m.frames_out,
            &m.bytes_in,
            &m.bytes_out,
        ] {
            counter.absolute(0);
        }
        for reason in ["limit", "ip_rate_limit", "ip_tracking_limit"] {
            m.rejected(reason).absolute(0);
        }
        m
    }
    fn closed(&self, reason: &'static str) -> metrics::Counter {
        metrics::counter!("net_connections_closed_total", "listener"=>self.name, "reason"=>reason)
    }
    fn rejected(&self, reason: &'static str) -> metrics::Counter {
        metrics::counter!("net_connections_rejected_total", "listener"=>self.name, "reason"=>reason)
    }
}

/// Owns one listening socket and all connections accepted from it.
#[derive(Debug)]
pub struct Listener {
    config: ListenerConfig,
    socket: TcpListener,
    addr: SocketAddr,
    metrics: Arc<ListenerMetrics>,
}
impl Listener {
    /// Binds this instance's socket and initializes transport metrics.
    pub async fn bind(name: &'static str, config: ListenerConfig) -> io::Result<Self> {
        let socket = TcpListener::bind(config.listen_addr).await?;
        Ok(Self {
            addr: socket.local_addr()?,
            socket,
            config,
            metrics: Arc::new(ListenerMetrics::new(name)),
        })
    }
    /// Returns the address assigned at bind time.
    pub const fn local_addr(&self) -> SocketAddr {
        self.addr
    }
    /// Accepts connections until shutdown and drains their writers.
    pub async fn serve<H: ConnectionHandler>(
        self,
        handler: H,
        shutdown: CancellationToken,
    ) -> io::Result<()> {
        let handler = Arc::new(handler);
        let config = self.config;
        let listener = self.socket;
        for cause in CloseCause::<H::Reason>::TRANSPORT {
            self.metrics.closed(cause.label()).absolute(0);
        }
        for &reason in H::Reason::ALL {
            self.metrics.closed(reason.label()).absolute(0);
        }
        let mut connections = JoinSet::new();
        let closing = CancellationToken::new();
        let mut last_id = 0_u64;
        let mut per_ip = HashMap::<IpAddr, u32>::new();
        let mut window = tokio::time::interval(Duration::from_secs(1));
        window.set_missed_tick_behavior(MissedTickBehavior::Skip);
        let shutdown = shutdown.cancelled();
        tokio::pin!(shutdown);
        loop {
            tokio::select! {
                biased;
                () = &mut shutdown => break,
                _ = window.tick() => per_ip.clear(),
                Some(joined) = connections.join_next(), if !connections.is_empty() => report_task_error(joined),
                accepted = listener.accept() => {
                    let (socket, peer) = match accepted {
                        Ok(accepted) => accepted,
                        Err(error) => {
                            tracing::warn!(%error, "client accept failed");
                            tokio::select! {
                                () = &mut shutdown => break,
                                () = tokio::time::sleep(Duration::from_millis(100)) => continue,
                            }
                        }
                    };
                    let limit = config.max_connections.get();
                    let reason = if connections.len() >= limit {
                        Some("limit")
                    } else if per_ip.len() >= limit && !per_ip.contains_key(&peer.ip()) {
                        Some("ip_tracking_limit")
                    } else {
                        let attempts = per_ip.entry(peer.ip()).or_default();
                        *attempts = attempts.saturating_add(1);
                        (*attempts > config.connections_per_ip_per_sec.get()).then_some("ip_rate_limit")
                    };
                    if let Some(reason) = reason {
                        self.metrics.rejected(reason).increment(1);
                        continue;
                    }
                    let Some(id) = last_id.checked_add(1) else {
                        tracing::error!("connection IDs exhausted");
                        break;
                    };
                    last_id = id;
                    let accepted_at = Instant::now();
                    connections.spawn(connection::serve(socket, connection::Peer { id: ConnId::new(id), addr: peer, accepted: accepted_at }, config, Arc::clone(&self.metrics), Arc::clone(&handler), closing.clone()));
                }
            }
        }
        drop(listener);
        closing.cancel();
        if tokio::time::timeout(connection::FLUSH_TIMEOUT + Duration::from_secs(1), async {
            while let Some(joined) = connections.join_next().await {
                report_task_error(joined);
            }
        })
        .await
        .is_err()
        {
            tracing::warn!("connection shutdown deadline elapsed");
            connections.shutdown().await;
        }
        Ok(())
    }
}

fn report_task_error(joined: Result<(), JoinError>) {
    if let Err(error) = joined {
        tracing::error!(%error, "connection task stopped unexpectedly");
    }
}
