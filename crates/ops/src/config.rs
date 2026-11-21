//! Development sign-on and loopback listener configuration.

use std::net::SocketAddr;

use infra::admin::METRICS;
use infra::http::HttpConfig;
use infra::logging::Environment;
use infra::process::ProcessConfig;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Config {
    pub(super) process: ProcessConfig,
    pub(super) http: HttpConfig,
    pub(super) operator: Operator,
    pub(super) publisher: Publisher,
    pub(super) servers: Vec<WatchedServer>,
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

impl WatchedServer {
    pub(super) async fn scrape(&self, client: &reqwest::Client) -> Result<String, reqwest::Error> {
        client
            .get(format!("http://{}{METRICS}", self.admin_addr))
            .send()
            .await?
            .error_for_status()?
            .text()
            .await
    }
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
        Ok(())
    }
}
