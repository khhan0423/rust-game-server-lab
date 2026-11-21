//! Development sign-on and loopback listener configuration.

use std::net::SocketAddr;

use infra::logging::Environment;
use infra::process::ProcessConfig;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Config {
    pub(super) process: ProcessConfig,
    pub(super) http: Http,
    pub(super) operator: Operator,
    pub(super) publisher: Publisher,
    pub(super) servers: Vec<WatchedServer>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Http {
    pub(super) listen_addr: SocketAddr,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Operator {
    pub(super) name: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Publisher {
    pub(super) ops_api_url: String,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct WatchedServer {
    pub(super) name: String,
    pub(super) admin_addr: SocketAddr,
    pub(super) role: Option<ServerRole>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ServerRole {
    Gateway,
}

impl Config {
    pub(super) fn check(&self) -> anyhow::Result<()> {
        match self.process.log.environment {
            Environment::Development => {}
            Environment::FriendsTest
            | Environment::Alpha
            | Environment::Beta
            | Environment::Live => {
                anyhow::bail!("real operator sign-on is not implemented yet");
            }
        }
        anyhow::ensure!(
            self.http.listen_addr.ip().is_loopback(),
            "operations HTTP must listen on loopback"
        );
        Ok(())
    }
}
