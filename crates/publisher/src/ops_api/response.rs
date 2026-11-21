//! Explicit field selections for the operator JSON API.

use db::login::{AccountStatus, MembershipStatus, ops};
use serde_json::{Value, json};

const fn status(value: AccountStatus) -> &'static str {
    match value {
        AccountStatus::Active => "active",
        AccountStatus::Sanctioned => "sanctioned",
        AccountStatus::Closing => "closing",
    }
}

pub(super) fn account(row: &ops::Account) -> Value {
    json!({"account_id": row.account_id.as_str(), "status": status(row.status), "created_at": row.created_at})
}

pub(super) fn detail(row: ops::Detail) -> Value {
    let identities = row
        .identities
        .into_iter()
        .map(|identity| {
            let provider = match identity.provider {
                ops::Provider::Guest => "guest",
                ops::Provider::Google => "google",
                ops::Provider::Apple => "apple",
                ops::Provider::Steam => "steam",
            };
            json!({"provider": provider, "subject": identity.subject})
        })
        .collect::<Vec<_>>();
    let memberships = row.memberships.into_iter().map(|membership| {
        let state = match membership.status {
            MembershipStatus::Member => "member",
            MembershipStatus::Canceled => "canceled",
            MembershipStatus::Sanctioned => "sanctioned",
        };
        json!({"game_id": membership.game_id.as_str(), "status": state, "joined_at": membership.joined_at})
    }).collect::<Vec<_>>();
    let tickets = row.tickets.into_iter().map(|ticket| {
        json!({"game_id": ticket.game_id.as_str(), "expires_at": ticket.expires_at, "consumed": ticket.consumed})
    }).collect::<Vec<_>>();
    json!({"account": account(&row.account), "identities": identities, "memberships": memberships, "tickets": tickets})
}

pub(super) fn audit_record(row: &ops::AuditRecord) -> Value {
    json!({
        "id": row.id.0, "occurred_at": row.occurred_at, "operator": row.operator, "action": row.action,
        "account_id": row.account_id.as_str(), "before": status(row.before), "after": status(row.after), "reason": row.reason,
    })
}
