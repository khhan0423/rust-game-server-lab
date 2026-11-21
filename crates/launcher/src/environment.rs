//! Loads the same assignment format and precedence as the service run scripts.

use std::{ffi::OsString, path::Path};

use anyhow::Context;
use tokio::process::Command;

pub(crate) struct Environment(Vec<(String, OsString)>);

impl Environment {
    pub(crate) fn load(path: &Path) -> anyhow::Result<Self> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => String::new(),
            Err(error) => return Err(error).with_context(|| format!("reading {}", path.display())),
        };
        let mut pairs = Vec::new();
        for (name, value) in text.lines().filter_map(|line| line.split_once('=')) {
            let mut bytes = name.bytes();
            if !bytes
                .next()
                .is_some_and(|b| b.is_ascii_alphabetic() || b == b'_')
                || !bytes.all(|b| b.is_ascii_alphanumeric() || b == b'_')
                || std::env::var_os(name).is_some()
                || pairs.iter().any(|(key, _)| key == name)
            {
                continue;
            }
            let value = ['"', '\'']
                .into_iter()
                .find_map(|quote| value.strip_prefix(quote)?.strip_suffix(quote))
                .unwrap_or(value);
            anyhow::ensure!(
                !value.contains('\0'),
                "invalid environment assignment for {name}"
            );
            pairs.push((name.to_owned(), OsString::from(value)));
        }
        Ok(Self(pairs))
    }

    pub(crate) fn apply(&self, cmd: &mut Command, binary: &str) {
        cmd.envs(self.0.iter().map(|(key, value)| (key, value)));
        if binary.eq_ignore_ascii_case("ops") {
            for name in [
                "DATABASE_URL",
                "LOGIN_DB_URL",
                "GAME_DB_URL",
                "POSTGRES_PASSWORD",
                "PUBLISHER_GAME_API_KEY",
            ] {
                cmd.env_remove(name);
            }
        }
    }
}
