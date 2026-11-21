//! Typed HTTP client for the publisher's operator API.

use std::net::IpAddr;

use anyhow::Context;
use axum::http::HeaderValue;
use infra::config::Secret;
use reqwest::Url;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub(super) struct Client {
    http: reqwest::Client,
    url: Url,
    key: HeaderValue,
    operator: HeaderValue,
}

impl Client {
    pub(super) fn new(client: reqwest::Client, url: &str, operator: &str) -> anyhow::Result<Self> {
        let url = Url::parse(url).context("invalid publisher operator API URL")?;
        let loopback = url
            .host_str()
            .and_then(|host| host.trim_matches(['[', ']']).parse::<IpAddr>().ok())
            .is_some_and(|ip| ip.is_loopback());
        anyhow::ensure!(
            loopback
                && url.scheme() == "http"
                && url.path() == "/"
                && url.query().is_none()
                && url.fragment().is_none()
                && url.username().is_empty()
                && url.password().is_none(),
            "publisher operator API must be a loopback HTTP origin"
        );
        let operator = operator.trim();
        anyhow::ensure!(
            !operator.is_empty()
                && operator.len() <= 128
                && operator.bytes().all(|byte| (b' '..=b'~').contains(&byte)),
            "invalid operator name"
        );
        let operator = HeaderValue::from_str(operator)?;
        let key = Secret::from_env("PUBLISHER_OPS_API_KEY")?;
        anyhow::ensure!(!key.expose().is_empty(), "PUBLISHER_OPS_API_KEY is empty");
        let mut key =
            HeaderValue::from_str(key.expose()).context("invalid publisher operator API key")?;
        key.set_sensitive(true);
        Ok(Self {
            http: client,
            url,
            key,
            operator,
        })
    }

    pub(super) fn operator(&self) -> &str {
        self.operator.to_str().unwrap_or_default()
    }

    pub(super) fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        let mut url = self.url.clone();
        url.set_path(path);
        self.http
            .request(method, url)
            .header("x-ops-key", self.key.clone())
            .header("x-operator", self.operator.clone())
    }
}

#[derive(Clone, Copy, Debug, Deserialize)]
#[serde(transparent)]
pub(super) struct AccountId(pub(super) Uuid);

#[derive(Debug, Deserialize)]
pub(super) struct Accounts {
    pub(super) accounts: Vec<Account>,
    pub(super) has_next: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct Account {
    #[serde(rename = "account_id")]
    pub(super) id: AccountId,
    pub(super) status: AccountStatus,
    pub(super) created_at: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum AccountStatus {
    Active,
    Sanctioned,
    Closing,
}

impl AccountStatus {
    pub(super) const fn as_str(&self) -> &'static str {
        match self {
            Self::Active => "active",
            Self::Sanctioned => "sanctioned",
            Self::Closing => "closing",
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SanctionStatus {
    Active,
    Sanctioned,
}

#[derive(Debug, Deserialize)]
pub(super) struct Detail {
    pub(super) account: Account,
    pub(super) identities: Vec<Identity>,
    pub(super) memberships: Vec<Membership>,
    pub(super) tickets: Vec<Ticket>,
}

#[derive(Debug, Deserialize)]
pub(super) struct Identity {
    pub(super) provider: String,
    pub(super) subject: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct Membership {
    pub(super) game_id: String,
    pub(super) status: String,
    pub(super) joined_at: String,
}

#[derive(Debug, Deserialize)]
pub(super) struct Ticket {
    pub(super) game_id: String,
    pub(super) expires_at: String,
    pub(super) consumed: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct Audit {
    pub(super) records: Vec<AuditRecord>,
    pub(super) has_next: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct AuditRecord {
    pub(super) occurred_at: String,
    pub(super) operator: String,
    pub(super) action: String,
    pub(super) account_id: AccountId,
    pub(super) before: AccountStatus,
    pub(super) after: AccountStatus,
    pub(super) reason: String,
}
