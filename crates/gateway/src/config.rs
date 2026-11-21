use infra::process::ProcessConfig;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Config {
    pub(super) process: ProcessConfig,
    pub(super) client: net::ListenerConfig,
}
