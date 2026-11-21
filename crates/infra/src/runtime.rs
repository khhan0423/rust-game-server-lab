//! Multi-thread runtime settings for server processes and the load bot.

use std::{io, num::NonZeroUsize};

use serde::Deserialize;
use tokio::runtime::{Builder, Runtime};

use crate::logging::LogConfig;

/// Process settings for tools that host an async runtime without an admin port.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolRuntime {
    /// Operations log configuration.
    #[serde(flatten)]
    pub log: LogConfig,
    /// Runtime worker configuration.
    #[serde(flatten)]
    pub runtime: RuntimeConfig,
}

/// Settings shared by processes that own a multi-thread runtime.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeConfig {
    worker_threads: Option<NonZeroUsize>,
}

impl RuntimeConfig {
    /// Starts a runtime with the configured worker count, or Tokio's default if omitted.
    pub fn build(&self) -> io::Result<Runtime> {
        let mut builder = Builder::new_multi_thread();
        if let Some(count) = self.worker_threads {
            builder.worker_threads(count.get());
        }
        builder.enable_all().build()
    }
}

#[cfg(test)]
mod tests {

    #[test]
    fn process_worker_count_configures_the_actual_runtime() {
        let settings = "[process]\nadmin_addr='127.0.0.1:0'\nname='game'\nenvironment='development'\nlog_level='info'\nworker_threads=2";
        let config: crate::logging::ToolConfig<crate::process::ProcessConfig> =
            toml::from_str(settings).unwrap();
        assert_eq!(
            config
                .process
                .runtime()
                .build()
                .unwrap()
                .metrics()
                .num_workers(),
            2
        );
    }
}
