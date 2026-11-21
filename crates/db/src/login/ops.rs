//! Credential-free `LoginDB` queries and audited operator mutations.

mod status;

pub use status::{
    AttributionError, AuditReason, OperatorName, SanctionStatus, StatusChange, StatusOutcome,
    set_status,
};

use super::{AccountId, AccountStatus, GUEST_PROVIDER, GameId, MembershipStatus};
use crate::{DbError, Pool};

/// An account's basic information.
#[derive(Debug)]
pub struct Account {
    /// Internal publisher ID.
    pub account_id: AccountId,
    /// Current platform state.
    pub status: AccountStatus,
    /// UTC creation timestamp.
    pub created_at: String,
}

/// Login provider discriminator stored in `LoginDB`.
#[derive(Debug, Clone, Copy, sqlx::Type)]
#[repr(i16)]
pub enum Provider {
    /// Guest login.
    Guest = 0,
    /// Google login.
    Google = 1,
    /// Apple login.
    Apple = 2,
    /// Steam login.
    Steam = 3,
}

/// Public metadata about a linked identity.
#[derive(Debug)]
pub struct Identity {
    /// Login provider.
    pub provider: Provider,
    /// Identifier with that provider.
    pub subject: String,
}

/// An account's membership in one game.
#[derive(Debug)]
pub struct Membership {
    /// Publisher catalog entry.
    pub game_id: GameId,
    /// Current membership state.
    pub status: MembershipStatus,
    /// UTC joining timestamp.
    pub joined_at: String,
}

/// Metadata about an issued entry ticket, without its credential or hash.
#[derive(Debug)]
pub struct TicketInfo {
    /// Game the ticket was issued for.
    pub game_id: GameId,
    /// UTC expiry timestamp.
    pub expires_at: String,
    /// Whether the ticket was redeemed.
    pub consumed: bool,
}

/// Account and its credential-free related records.
#[derive(Debug)]
pub struct Detail {
    /// Basic account record.
    pub account: Account,
    /// Linked login methods.
    pub identities: Vec<Identity>,
    /// Game memberships.
    pub memberships: Vec<Membership>,
    /// Up to 20 recent tickets.
    pub tickets: Vec<TicketInfo>,
}

/// Audit record identifier.
#[derive(Debug, Clone, Copy)]
pub struct AuditId(i64);
impl AuditId {
    /// Returns the database identifier.
    pub const fn value(self) -> i64 {
        self.0
    }
}

/// Persisted operator action.
#[derive(Debug)]
pub struct AuditRecord {
    /// Unique record identifier.
    pub id: AuditId,
    /// UTC action timestamp.
    pub occurred_at: String,
    /// Operator name.
    pub operator: String,
    /// Action name.
    pub action: String,
    /// Target account.
    pub account_id: AccountId,
    /// Previous state.
    pub before: AccountStatus,
    /// Committed state.
    pub after: AccountStatus,
    /// Operator's explanation.
    pub reason: String,
}

/// One window into a list.
#[derive(Debug, Clone, Copy)]
pub struct Page {
    limit: i64,
    offset: i64,
}
impl Page {
    /// Reads one extra row so callers can detect a following page.
    pub fn new(number: u32, size: usize) -> Option<Self> {
        let size = i64::try_from(size).ok().filter(|&n| n > 0)?;
        Some(Self {
            limit: size.checked_add(1)?,
            offset: i64::from(number.checked_sub(1)?).checked_mul(size)?,
        })
    }
}

/// Lists accounts, optionally matching an exact account ID or guest subject.
pub async fn accounts(
    pool: &Pool,
    search: Option<&str>,
    page: Page,
) -> Result<Vec<Account>, DbError> {
    let rows = sqlx::query!(
        "SELECT a.account_id, a.status AS \"status: AccountStatus\", to_char(a.created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS \"created_at!\"
         FROM accounts a
         WHERE $1::text IS NULL OR a.account_id = $1 OR EXISTS (
             SELECT 1 FROM account_identities i WHERE i.account_id = a.account_id AND i.provider = $2 AND i.subject = $1
         ) ORDER BY a.created_at DESC, a.account_id DESC LIMIT $3 OFFSET $4",
        search, GUEST_PROVIDER, page.limit, page.offset,
    ).fetch_all(pool).await?;
    rows.into_iter()
        .map(|row| {
            Ok(Account {
                account_id: AccountId::parse(&row.account_id)?,
                status: row.status,
                created_at: row.created_at,
            })
        })
        .collect()
}

/// Reads account information without selecting either secret hashes or ticket hashes.
pub async fn detail(pool: &Pool, id: &AccountId) -> Result<Option<Detail>, DbError> {
    let Some(row) = sqlx::query!(
        "SELECT account_id, status AS \"status: AccountStatus\", to_char(created_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS \"created_at!\"
         FROM accounts WHERE account_id = $1", id.to_string(),
    ).fetch_optional(pool).await? else { return Ok(None); };
    let identities = sqlx::query_as!(Identity,
        "SELECT provider AS \"provider: Provider\", subject FROM account_identities WHERE account_id = $1 ORDER BY provider", id.to_string(),
    ).fetch_all(pool).await?;
    let memberships = sqlx::query!(
        "SELECT game_id, status AS \"status: MembershipStatus\", to_char(joined_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS \"joined_at!\"
         FROM game_memberships WHERE account_id = $1 ORDER BY game_id", id.to_string(),
    ).fetch_all(pool).await?.into_iter().map(|row| Membership {
        game_id: GameId::new(row.game_id), status: row.status, joined_at: row.joined_at,
    }).collect();
    let tickets = sqlx::query!(
        "SELECT game_id, to_char(expires_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS \"expires_at!\", (consumed_at IS NOT NULL) AS \"consumed!\"
         FROM entry_tickets WHERE account_id = $1 ORDER BY COALESCE(issued_at, expires_at) DESC, expires_at DESC, game_id LIMIT 20", id.to_string(),
    ).fetch_all(pool).await?.into_iter().map(|row| TicketInfo {
        game_id: GameId::new(row.game_id), expires_at: row.expires_at, consumed: row.consumed,
    }).collect();
    Ok(Some(Detail {
        account: Account {
            account_id: AccountId::parse(&row.account_id)?,
            status: row.status,
            created_at: row.created_at,
        },
        identities,
        memberships,
        tickets,
    }))
}

/// Lists audit records with the newest actions first.
pub async fn audit(pool: &Pool, page: Page) -> Result<Vec<AuditRecord>, DbError> {
    let rows = sqlx::query!(
        "SELECT id, to_char(occurred_at AT TIME ZONE 'UTC', 'YYYY-MM-DD\"T\"HH24:MI:SS.US\"Z\"') AS \"occurred_at!\", operator, action, account_id,
            before_status AS \"before: AccountStatus\", after_status AS \"after: AccountStatus\", reason
         FROM ops_audit ORDER BY id DESC LIMIT $1 OFFSET $2", page.limit, page.offset,
    ).fetch_all(pool).await?;
    rows.into_iter()
        .map(|row| {
            Ok(AuditRecord {
                id: AuditId(row.id),
                occurred_at: row.occurred_at,
                operator: row.operator,
                action: row.action,
                account_id: AccountId::parse(&row.account_id)?,
                before: row.before,
                after: row.after,
                reason: row.reason,
            })
        })
        .collect()
}
