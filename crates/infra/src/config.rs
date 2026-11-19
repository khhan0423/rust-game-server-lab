//! 설정 파일(TOML)과 비밀값(환경 변수) 로드 (설계 08 8장).

use std::env::VarError;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::de::DeserializeOwned;

/// 설정을 읽지 못한 이유.
#[derive(Debug, thiserror::Error)]
pub enum ConfigError {
    /// 파일을 열거나 읽지 못했다.
    #[error("cannot read config file {}", .path.display())]
    Read {
        /// 읽으려던 파일.
        path: PathBuf,
        /// 운영체제가 돌려준 원인.
        #[source]
        source: std::io::Error,
    },
    /// TOML 문법이 틀렸거나, 필요한 키가 없거나, 모르는 키가 있다.
    #[error("invalid config file {}", .path.display())]
    Parse {
        /// 읽으려던 파일.
        path: PathBuf,
        /// 어느 키가 어떻게 틀렸는지.
        #[source]
        source: toml::de::Error,
    },
    /// 비밀값을 담아야 할 환경 변수가 없다.
    #[error("environment variable {name} is not set")]
    MissingEnv {
        /// 환경 변수 이름.
        name: String,
    },
    /// 환경 변수 값이 UTF-8 문자열이 아니다.
    #[error("environment variable {name} is not valid UTF-8")]
    InvalidEnv {
        /// 환경 변수 이름.
        name: String,
    },
}

/// TOML 설정 파일을 읽어 `T`로 바꾼다.
///
/// 런타임을 만들기 전, 프로세스 시작 단계에서만 부른다(동기 파일 읽기).
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

/// 로그와 `Debug` 출력에 값이 나오지 않는 비밀 문자열.
///
/// 비밀값은 설정 파일에 두지 않고 환경 변수에서만 읽는다.
pub struct Secret(String);

impl Secret {
    /// 환경 변수 `name`에서 비밀값을 읽는다.
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

    /// 실제 값을 꺼낸다. 접속할 때처럼 꼭 필요한 곳에서만 쓴다.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Secret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Secret(***)")
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
        let path =
            std::env::temp_dir().join(format!("infra-config-{}-{name}.toml", std::process::id()));
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
