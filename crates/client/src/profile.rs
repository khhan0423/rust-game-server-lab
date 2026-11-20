//! Stores one publisher guest credential per local profile.

use std::fmt;
use std::io;
use std::path::{Path, PathBuf};
use std::str::FromStr;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// A profile name that can safely be used as a Windows file name.
#[derive(Clone, Debug)]
pub(super) struct ProfileName(String);

impl FromStr for ProfileName {
    type Err = ProfileError;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        let upper = value.to_ascii_uppercase();
        if value.is_empty()
            || value.len() > 64
            || !value
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
            || matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
            || (upper.len() == 4
                && (upper.starts_with("COM") || upper.starts_with("LPT"))
                && upper.ends_with(|c: char| c.is_ascii_digit()))
        {
            return Err(ProfileError::InvalidName);
        }
        Ok(Self(value.to_owned()))
    }
}

impl fmt::Display for ProfileName {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.0)
    }
}

impl ProfileName {
    /// Resolves the credential file in the selected data directory.
    pub(super) fn path(&self, dir: &Path) -> PathBuf {
        dir.join(format!("{}.toml", self.0))
    }
}

/// The publisher's account identifier.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(super) struct AccountId(Uuid);

impl fmt::Display for AccountId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// The publisher's guest login identifier.
#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(super) struct GuestId(Uuid);

/// An on-wire guest secret that is always redacted in debug output.
#[derive(Clone, Deserialize, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub(super) struct Secret(String);

impl fmt::Debug for Secret {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("[redacted]")
    }
}

/// The stored identity issued by the publisher.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(super) struct Guest {
    #[serde(rename = "guest_id")]
    pub(super) id: GuestId,
    pub(super) secret: Secret,
    pub(super) account_id: AccountId,
}

/// A guest identity and the publisher that issued it.
#[derive(Clone, Debug, Deserialize, PartialEq, Eq, Serialize)]
pub(super) struct Credentials {
    pub(super) publisher_url: String,
    #[serde(flatten)]
    pub(super) guest: Guest,
}

/// Problems opening or encoding a credential profile.
#[derive(Debug, thiserror::Error)]
pub(super) enum ProfileError {
    #[error(
        "profile must be a non-reserved name of up to 64 letters, numbers, dashes, or underscores"
    )]
    InvalidName,
    #[error("profile file I/O failed")]
    Io(#[from] io::Error),
    // TOML parser diagnostics can quote the line containing the secret.
    #[error("profile does not contain valid guest credentials")]
    Decode,
    #[error("could not encode guest credentials")]
    Encode,
}

/// Loads a stored credential; an absent file is a new profile.
pub(super) fn load(path: &Path) -> Result<Option<Credentials>, ProfileError> {
    match std::fs::read_to_string(path) {
        Ok(raw) => toml::from_str(&raw)
            .map(Some)
            .map_err(|_| ProfileError::Decode),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error.into()),
    }
}

/// Saves the guest credentials on the worker after account creation.
pub(super) async fn save(path: &Path, credentials: &Credentials) -> Result<(), ProfileError> {
    let serialized = toml::to_string(credentials).map_err(|_| ProfileError::Encode)?;
    tokio::fs::write(path, serialized).await?;
    Ok(())
}

#[cfg(test)]
pub(super) mod tests {
    use super::*;

    pub(crate) fn credentials() -> Credentials {
        Credentials {
            publisher_url: "http://127.0.0.1:7101".into(),
            guest: Guest {
                id: GuestId(Uuid::from_u128(0x018f_92fc_0000_7000_8000_0000_0000_0002)),
                account_id: AccountId(Uuid::from_u128(0x018f_92fc_0000_7000_8000_0000_0000_0001)),
                secret: Secret("ab".repeat(32)),
            },
        }
    }

    struct TempProfile(PathBuf);

    impl TempProfile {
        fn new() -> Self {
            let dir = std::env::temp_dir().join(format!("client-profile-{}", Uuid::now_v7()));
            std::fs::create_dir(&dir).expect("create isolated profile directory");
            Self(dir)
        }
    }

    impl Drop for TempProfile {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.0).expect("remove isolated profile directory");
        }
    }

    #[test]
    fn saved_profile_round_trips_through_named_toml_file() {
        let dir = TempProfile::new();
        let name: ProfileName = "guest-2".parse().expect("valid profile name");
        let expected = credentials();
        tokio::runtime::Builder::new_current_thread()
            .build()
            .expect("build profile test runtime")
            .block_on(save(&name.path(&dir.0), &expected))
            .expect("save guest profile");

        assert_eq!(
            load(&dir.0.join("guest-2.toml")).expect("load guest profile"),
            Some(expected)
        );
    }

    #[test]
    fn missing_profile_loads_without_credentials() {
        let dir = TempProfile::new();

        assert_eq!(
            load(&dir.0.join("missing.toml")).expect("load missing profile"),
            None
        );
    }
}
