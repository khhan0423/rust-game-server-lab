//! TOML configuration for the publisher process and its game catalog.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::time::Duration;

use infra::process::ProcessConfig;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Config {
    pub(super) process: ProcessConfig,
    pub(super) db: DbConfig,
    pub(super) http: HttpConfig,
    #[serde(default)]
    pub(super) tickets: TicketConfig,
    pub(super) games: HashMap<String, GameConfig>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DbConfig {
    max_connections: u32,
    acquire_timeout_ms: u64,
}

impl DbConfig {
    pub(super) const fn settings(&self) -> db::PoolSettings {
        db::PoolSettings {
            max_connections: self.max_connections,
            acquire_timeout: Duration::from_millis(self.acquire_timeout_ms),
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct HttpConfig {
    pub(super) listen_addr: SocketAddr,
}

#[derive(Debug, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub(super) struct TicketConfig {
    pub(super) ttl_secs: u32,
}

impl Default for TicketConfig {
    fn default() -> Self {
        Self { ttl_secs: 60 }
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GameConfig {
    pub(super) entry_url: String,
}
