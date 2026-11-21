//! Typed HTTP client for the publisher's operator API.

use anyhow::Context;
use axum::http::HeaderValue;
use infra::http::{
    Exposure, HttpOrigin,
    api::{ApiKey, operator},
};
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub(super) struct Client {
    http: reqwest::Client,
    origin: HttpOrigin,
    headers: axum::http::HeaderMap,
    operator: String,
}

impl Client {
    pub(super) fn new(client: reqwest::Client, url: &str, operator: &str) -> anyhow::Result<Self> {
        let origin = HttpOrigin::parse(url, Exposure::Loopback)
            .context("publisher operator API must be a loopback HTTP origin")?;
        let mut headers = axum::http::HeaderMap::new();
        headers.insert(
            operator::NAME_HEADER,
            HeaderValue::from_bytes(operator.as_bytes())?,
        );
        ApiKey::from_env(operator::KEY_HEADER, operator::KEY_ENV)?.insert(&mut headers);
        Ok(Self {
            http: client,
            origin,
            headers,
            operator: operator.to_owned(),
        })
    }

    pub(super) fn operator(&self) -> &str {
        self.operator.as_str()
    }

    pub(super) fn request(&self, method: reqwest::Method, path: &str) -> reqwest::RequestBuilder {
        self.http
            .request(method, self.origin.route(path))
            .headers(self.headers.clone())
    }
}

#[derive(Debug, Deserialize)]
pub(super) struct Accounts {
    pub(super) accounts: Vec<Account>,
    pub(super) has_next: bool,
}

#[derive(Debug, Deserialize)]
pub(super) struct Account {
    #[serde(rename = "account_id")]
    pub(super) id: Uuid,
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
    pub(super) account_id: Uuid,
    pub(super) before: AccountStatus,
    pub(super) after: AccountStatus,
    pub(super) reason: String,
}
