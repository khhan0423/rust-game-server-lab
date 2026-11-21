//! TOML configuration for the publisher process and its game catalog.

use infra::http::HttpConfig;
use std::collections::HashMap;

use infra::process::ProcessConfig;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Config {
    pub(super) process: ProcessConfig,
    pub(super) db: db::PoolConfig,
    pub(super) http: HttpConfig,
    pub(super) ops_api: HttpConfig,
    #[serde(default)]
    pub(super) tickets: TicketConfig,
    pub(super) games: HashMap<String, GameConfig>,
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

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct GameConfig {
    pub(super) entry_url: String,
}
