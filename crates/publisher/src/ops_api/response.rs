//! Fields exposed by the operator JSON API. Credential data is never serialized.
use crate::wire::{membership, operator_status};
use db::login::ops;
use serde::Serialize;

#[derive(Serialize)]
pub(super) struct Account {
    #[serde(rename = "account_id")]
    id: String,
    status: &'static str,
    created_at: String,
}
impl From<ops::Account> for Account {
    fn from(row: ops::Account) -> Self {
        Self {
            id: row.account_id.to_string(),
            status: operator_status(row.status),
            created_at: row.created_at,
        }
    }
}
#[derive(Serialize)]
struct Identity {
    provider: &'static str,
    subject: String,
}
#[derive(Serialize)]
struct Membership {
    game_id: String,
    status: &'static str,
    joined_at: String,
}
#[derive(Serialize)]
struct Ticket {
    game_id: String,
    expires_at: String,
    consumed: bool,
}
#[derive(Serialize)]
pub(super) struct Detail {
    account: Account,
    identities: Vec<Identity>,
    memberships: Vec<Membership>,
    tickets: Vec<Ticket>,
}
impl From<ops::Detail> for Detail {
    fn from(row: ops::Detail) -> Self {
        Self {
            account: row.account.into(),
            identities: row
                .identities
                .into_iter()
                .map(|i| Identity {
                    subject: i.subject,
                    provider: match i.provider {
                        ops::Provider::Guest => "guest",
                        ops::Provider::Google => "google",
                        ops::Provider::Apple => "apple",
                        ops::Provider::Steam => "steam",
                    },
                })
                .collect(),
            memberships: row
                .memberships
                .into_iter()
                .map(|m| Membership {
                    game_id: m.game_id.as_str().to_owned(),
                    status: membership(m.status),
                    joined_at: m.joined_at,
                })
                .collect(),
            tickets: row
                .tickets
                .into_iter()
                .map(|t| Ticket {
                    game_id: t.game_id.as_str().to_owned(),
                    expires_at: t.expires_at,
                    consumed: t.consumed,
                })
                .collect(),
        }
    }
}
#[derive(Serialize)]
pub(super) struct Record {
    id: i64,
    occurred_at: String,
    operator: String,
    action: String,
    account_id: String,
    before: &'static str,
    after: &'static str,
    reason: String,
}
impl From<ops::AuditRecord> for Record {
    fn from(row: ops::AuditRecord) -> Self {
        Self {
            id: row.id.value(),
            occurred_at: row.occurred_at,
            operator: row.operator,
            action: row.action,
            account_id: row.account_id.to_string(),
            before: operator_status(row.before),
            after: operator_status(row.after),
            reason: row.reason,
        }
    }
}
