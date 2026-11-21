//! Loading the config file (TOML) and secrets (environment variables) (design 08 section 8).

use std::env::VarError;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

/// Why the config could not be read.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// An API key cannot be used as a nonempty HTTP header.
    #[error("{name} must contain a nonempty HTTP header value")]
    InvalidHeader {
        /// Environment variable name, never its contents.
        name: String,
    },
    /// The file could not be opened or read.
    #[error("cannot read config file {}", .path.display())]
    Read {
        /// The file that was being read.
        path: PathBuf,
        /// The cause reported by the operating system.
        #[source]
        source: std::io::Error,
    },
    /// The TOML syntax is wrong, a required key is missing, or there is an unknown key.
    #[error("invalid config file {}", .path.display())]
    Parse {
        /// The file that was being read.
        path: PathBuf,
        /// Which key is wrong and how.
        #[source]
        source: toml::de::Error,
    },
    /// The environment variable that should hold the secret is missing.
    #[error("environment variable {name} is not set")]
    MissingEnv {
        /// Environment variable name.
        name: String,
    },
    /// The environment variable value is not a UTF-8 string.
    #[error("environment variable {name} is not valid UTF-8")]
    InvalidEnv {
        /// Environment variable name.
        name: String,
    },
}

/// Reads a TOML config file and converts it to `T`.
///
/// Call only during process startup, before the runtime is created (synchronous file read).
pub fn load<T: DeserializeOwned>(path: &Path) -> Result<T, ConfigError> {
    let text = std::fs::read_to_string(path).map_err(|source| ConfigError::Read {
        path: path.to_owned(),
        source,
    })?;
    toml::from_str(&text).map_err(|source| ConfigError::Parse {
        path: path.to_owned(),
        source,
    })
}

/// A secret string whose value does not appear in logs or `Debug` output.
///
/// Secrets are not kept in config files; they are read only from environment variables.
pub struct Secret(String);

impl Secret {
    /// Reads a secret from the environment variable `name`.
    pub fn from_env(name: &str) -> Result<Self, ConfigError> {
        match std::env::var(name) {
            Ok(value) => Ok(Self(value)),
            Err(VarError::NotPresent) => Err(ConfigError::MissingEnv {
                name: name.to_owned(),
            }),
            Err(VarError::NotUnicode(_)) => Err(ConfigError::InvalidEnv {
                name: name.to_owned(),
            }),
        }
    }

    /// Returns the actual value. Use only where it is strictly needed, such as when connecting.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("[redacted]")
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;

    use serde::Deserialize;

    use super::*;

    #[derive(Debug, Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Sample {
        #[expect(dead_code, reason = "the test only checks that the key is required")]
        max_connections: u32,
    }

    fn write_temp(name: &str, text: &str) -> PathBuf {
        let dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/tmp");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join(format!("infra-config-{}-{name}.toml", std::process::id()));
        std::fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn load_names_the_missing_key_when_required_key_is_absent() {
        let path = write_temp("missing", "");

        let error = load::<Sample>(&path).unwrap_err();

        assert!(matches!(error, ConfigError::Parse { .. }));
        assert!(
            error
                .source()
                .unwrap()
                .to_string()
                .contains("max_connections")
        );
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn load_rejects_unknown_key() {
        let path = write_temp("unknown", "max_connections = 4\ntypo_key = 1\n");

        let error = load::<Sample>(&path).unwrap_err();

        assert!(matches!(error, ConfigError::Parse { .. }));
        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn secret_debug_output_hides_the_value() {
        let secret = Secret("postgres://user:hunter2@db/game".to_owned());

        let printed = format!("{secret:?}");

        assert!(!printed.contains("hunter2"));
    }
}
