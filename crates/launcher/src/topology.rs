//! Validates local service definitions and computes a stable start order.

use std::{
    collections::{HashMap, HashSet},
    num::NonZeroU64,
    path::{Path, PathBuf},
    time::Duration,
};

use anyhow::{Context, ensure};
use infra::runtime::ToolRuntime;
use serde::Deserialize;

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ServiceConfig {
    name: String,
    binary: String,
    config: PathBuf,
    #[serde(default)]
    args: Vec<String>,
    #[serde(default)]
    depends_on: Vec<String>,
}

impl ServiceConfig {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    pub(crate) fn binary(&self) -> &str {
        &self.binary
    }

    pub(crate) fn config(&self) -> &Path {
        &self.config
    }

    pub(crate) fn args(&self) -> &[String] {
        &self.args
    }

    pub(crate) fn dependencies(&self) -> &[String] {
        &self.depends_on
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Settings {
    #[serde(default = "default_bin_dir")]
    bin_dir: PathBuf,
    #[serde(default = "default_env_file")]
    env_file: PathBuf,
    ready_timeout_seconds: NonZeroU64,
    stop_timeout_seconds: NonZeroU64,
    process: ToolRuntime,
    services: Vec<ServiceConfig>,
}

fn default_bin_dir() -> PathBuf {
    PathBuf::from("target/debug")
}

fn default_env_file() -> PathBuf {
    PathBuf::from(".env")
}

#[derive(Debug)]
pub(crate) struct Topology(Settings);

impl Topology {
    pub(crate) fn load(path: &Path) -> anyhow::Result<Self> {
        Self::new(infra::config::load(path)?)
            .with_context(|| format!("invalid topology {}", path.display()))
    }

    fn new(mut file: Settings) -> anyhow::Result<Self> {
        let mut pending = HashMap::new();
        ensure!(!file.services.is_empty(), "topology contains no services");
        for (index, service) in file.services.iter().enumerate() {
            for label in [service.name(), service.binary()] {
                ensure!(
                    !label.is_empty()
                        && label
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_')),
                    "invalid service name or binary: {label:?}"
                );
            }
            ensure!(
                pending.insert(service.name(), index).is_none(),
                "duplicate service {}",
                service.name()
            );
        }
        for service in &file.services {
            for dep in service.dependencies() {
                ensure!(
                    pending.contains_key(dep.as_str()),
                    "service {} has unknown dependency {dep}",
                    service.name()
                );
            }
        }
        let mut started = HashSet::new();
        let mut ordered = Vec::with_capacity(file.services.len());
        let mut services = std::mem::take(&mut file.services);
        while !services.is_empty() {
            let index = services
                .iter()
                .position(|service| {
                    service
                        .dependencies()
                        .iter()
                        .all(|dep| started.contains(dep))
                })
                .with_context(|| {
                    format!(
                        "dependency cycle involves {}",
                        services
                            .iter()
                            .map(ServiceConfig::name)
                            .collect::<Vec<_>>()
                            .join(", ")
                    )
                })?;
            let service = services.remove(index);
            started.insert(service.name.clone());
            ordered.push(service);
        }
        file.services = ordered;
        Ok(Self(file))
    }

    pub(crate) fn process(&self) -> &ToolRuntime {
        &self.0.process
    }

    pub(crate) fn bin_dir(&self) -> &Path {
        &self.0.bin_dir
    }

    pub(crate) fn env_file(&self) -> &Path {
        &self.0.env_file
    }

    pub(crate) fn ready_timeout(&self) -> Duration {
        Duration::from_secs(self.0.ready_timeout_seconds.get())
    }

    pub(crate) fn stop_timeout(&self) -> Duration {
        Duration::from_secs(self.0.stop_timeout_seconds.get())
    }

    pub(crate) fn into_services(self) -> Vec<ServiceConfig> {
        self.0.services
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_dependencies_and_preserves_order_among_available_services() {
        let fixture = "ready_timeout_seconds=10\nstop_timeout_seconds=10\n[process]\nname='launcher'\nenvironment='development'\nlog_level='info'\n";
        let cases = [
            (
                vec![("alpha", vec![]), ("alpha", vec![])],
                Err("duplicate service alpha"),
            ),
            (
                vec![("alpha", vec!["missing"])],
                Err("unknown dependency missing"),
            ),
            (
                vec![("alpha", vec!["beta"]), ("beta", vec!["alpha"])],
                Err("dependency cycle"),
            ),
            (
                vec![
                    ("delta", vec!["alpha", "beta"]),
                    ("beta", vec![]),
                    ("alpha", vec![]),
                    ("gamma", vec![]),
                ],
                Ok(vec!["beta", "alpha", "delta", "gamma"]),
            ),
        ];
        for (services, expected) in cases {
            use std::fmt::Write;
            let mut input = fixture.to_owned();
            for (name, deps) in services {
                writeln!(input, "[[services]]\nname={name:?}\nbinary='gateway'\nconfig='gateway.toml'\ndepends_on={deps:?}").unwrap();
            }
            let result = Topology::new(toml::from_str(&input).unwrap());
            match expected {
                Ok(order) => assert_eq!(
                    result
                        .unwrap()
                        .0
                        .services
                        .iter()
                        .map(ServiceConfig::name)
                        .collect::<Vec<_>>(),
                    order
                ),
                Err(reason) => assert!(result.unwrap_err().to_string().contains(reason)),
            }
        }
    }
}
