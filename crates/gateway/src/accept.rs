use std::collections::HashMap;
use std::net::IpAddr;
use std::time::Duration;

use tokio::net::TcpListener;
use tokio::task::{JoinError, JoinSet};
use tokio::time::{Instant, MissedTickBehavior};
use tokio_util::sync::CancellationToken;

use crate::config::ClientConfig;
use crate::connection::{self, CloseCause, ConnId};
use crate::writer::FLUSH_TIMEOUT;

pub(super) fn initialize_metrics() {
    metrics::gauge!("gateway_connections_current").set(0);
    metrics::counter!("gateway_connections_accepted_total").absolute(0);
    for reason in ["limit", "ip_rate_limit"] {
        metrics::counter!("gateway_connections_rejected_total", "reason" => reason).absolute(0);
    }
    for reason in CloseCause::ALL {
        metrics::counter!("gateway_connections_closed_total", "reason" => reason.label())
            .absolute(0);
    }
    metrics::counter!("gateway_frames_received_total").absolute(0);
    metrics::counter!("gateway_frames_sent_total").absolute(0);
    metrics::counter!("gateway_bytes_received_total").absolute(0);
    metrics::counter!("gateway_bytes_sent_total").absolute(0);
    metrics::counter!("gateway_auth_attempts_total").absolute(0);
    metrics::counter!("gateway_auth_failed_total").absolute(0);
}

pub(super) async fn run(
    listener: TcpListener,
    config: ClientConfig,
    shutdown: impl Future<Output = ()>,
) {
    let mut connections = JoinSet::new();
    let closing = CancellationToken::new();
    let mut last_id = 0_u64;
    let mut per_ip = HashMap::<IpAddr, u32>::new();
    let mut window = tokio::time::interval(Duration::from_secs(1));
    window.set_missed_tick_behavior(MissedTickBehavior::Skip);
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
                let Some(id) = last_id.checked_add(1) else {
                    tracing::error!("connection IDs exhausted");
                    break;
                };
                last_id = id;
                let limit = config.max_connections.get();
                let reason = if connections.len() >= limit {
                    Some("limit")
                } else if per_ip.len() >= limit && !per_ip.contains_key(&peer.ip()) {
                    Some("ip_rate_limit")
                } else {
                    let attempts = per_ip.entry(peer.ip()).or_default();
                    *attempts = attempts.saturating_add(1);
                    (*attempts > config.connections_per_ip_per_sec.get()).then_some("ip_rate_limit")
                };
                if let Some(reason) = reason {
                    metrics::counter!("gateway_connections_rejected_total", "reason" => reason).increment(1);
                    continue;
                }
                let accepted_at = Instant::now();
                connections.spawn(connection::run(socket, connection::Peer {
                    id: ConnId(id), address: peer, accepted_at,
                }, config, closing.clone()));
            }
        }
    }
    drop(listener);
    closing.cancel();
    if tokio::time::timeout(FLUSH_TIMEOUT + Duration::from_secs(1), async {
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
}

fn report_task_error(joined: Result<(), JoinError>) {
    if let Err(error) = joined {
        tracing::error!(%error, "connection task stopped unexpectedly");
    }
}
