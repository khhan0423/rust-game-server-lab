use std::net::SocketAddr;
use std::num::{NonZeroU32, NonZeroUsize};

use infra::process::ProcessConfig;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Config {
    pub(super) process: ProcessConfig,
    pub(super) client: ClientConfig,
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ClientConfig {
    pub(super) listen_addr: SocketAddr,
    pub(super) max_connections: NonZeroUsize,
    pub(super) auth_timeout_ms: NonZeroU32,
    pub(super) write_queue_capacity: NonZeroUsize,
    #[serde(default = "default_ip_rate")]
    pub(super) connections_per_ip_per_sec: NonZeroU32,
    pub(super) log_connections: bool,
}

const fn default_ip_rate() -> NonZeroU32 {
    match NonZeroU32::new(1000) {
        Some(rate) => rate,
        None => NonZeroU32::MIN,
    }
}
