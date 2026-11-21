//! Login database persistence for publisher accounts, guest identities, and entry tickets.

pub mod ops;
mod types;

use sqlx::migrate::Migrator;

pub use self::types::{
    AccountId, AccountStatus, EntryGrant, GameId, GuestCredentials, GuestId, GuestSecret,
    LoginOutcome, MembershipStatus, Ticket, VerifiedTicket,
};
use crate::{DbError, Pool};

/// Migrations that apply only to the login database.
pub static LOGIN_MIGRATOR: Migrator = sqlx::migrate!("../../migrations/login");

/// Discriminator for the guest authentication provider in the login database.
const GUEST_PROVIDER: i16 = 0;

/// Inserts the new account and its guest identity in one transaction. Only the hash is persisted.
pub async fn create_guest(
    pool: &Pool,
    account_id: &AccountId,
    guest: &GuestCredentials<'_>,
) -> Result<(), DbError> {
    let mut tx = pool.begin().await?;
    sqlx::query!(
        "INSERT INTO accounts (account_id, status, created_at) VALUES ($1, $2, now())",
        account_id.to_string(),
        AccountStatus::Active as _,
    )
    .execute(&mut *tx)
    .await?;
    sqlx::query!(
        "INSERT INTO account_identities (provider, subject, account_id, secret_hash, linked_at)
         VALUES ($1, $2, $3, sha256($4), now())",
        GUEST_PROVIDER,
        guest.guest_id.to_string(),
        account_id.to_string(),
        guest.secret.expose().as_slice(),
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Authenticates the guest, creates or restores the membership, and issues a ticket in one
/// transaction. Rejected logins leave no changes.
pub async fn login_guest(
    pool: &Pool,
    guest: &GuestCredentials<'_>,
    grant: &EntryGrant<'_>,
) -> Result<LoginOutcome, DbError> {
    let mut tx = pool.begin().await?;
    // Keep the verified credential and platform state stable until the ticket is issued.
    let Some(account) = sqlx::query!(
        "SELECT a.account_id, a.status AS \"status: AccountStatus\"
         FROM accounts a JOIN account_identities i USING (account_id)
         WHERE i.provider = $1 AND i.subject = $2 AND i.secret_hash = sha256($3)
         FOR SHARE OF a, i",
        GUEST_PROVIDER,
        guest.guest_id.to_string(),
        guest.secret.expose().as_slice(),
    )
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(LoginOutcome::InvalidCredentials);
    };
    let account_id = AccountId::parse(&account.account_id)?;
    match account.status {
        AccountStatus::Active => {}
        AccountStatus::Sanctioned | AccountStatus::Closing => {
            return Ok(LoginOutcome::AccountBlocked(account_id));
        }
    }

    // PostgreSQL locks the conflicting membership even if its state prevents the update.
    let changed = sqlx::query!(
        "INSERT INTO game_memberships (account_id, game_id, status, joined_at)
         VALUES ($1, $2, $3, now())
         ON CONFLICT (account_id, game_id) DO UPDATE SET status = EXCLUDED.status
         WHERE game_memberships.status = $4",
        account_id.to_string(),
        grant.game_id.as_str(),
        MembershipStatus::Member as _,
        MembershipStatus::Canceled as _,
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    match changed {
        0 => {
            let status = sqlx::query_scalar!(
                "SELECT status AS \"status: MembershipStatus\" FROM game_memberships
                 WHERE account_id = $1 AND game_id = $2",
                account_id.to_string(),
                grant.game_id.as_str(),
            )
            .fetch_one(&mut *tx)
            .await?;
            match status {
                MembershipStatus::Member => {}
                MembershipStatus::Sanctioned => {
                    return Ok(LoginOutcome::AccountBlocked(account_id));
                }
                MembershipStatus::Canceled => {
                    return Err(DbError::Inconsistent(
                        "canceled membership was not restored",
                    ));
                }
            }
        }
        1 => {}
        _ => {
            return Err(DbError::Inconsistent(
                "one login changed several memberships",
            ));
        }
    }

    sqlx::query!(
        "INSERT INTO entry_tickets (ticket_hash, account_id, game_id, expires_at)
         VALUES (sha256($1), $2, $3, clock_timestamp() + $4 * interval '1 second')",
        grant.ticket.expose().as_slice(),
        account_id.to_string(),
        grant.game_id.as_str(),
        f64::from(grant.ttl_secs),
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(LoginOutcome::Issued(account_id))
}

/// Consumes a valid ticket and reads the account and membership states in one transaction.
/// `None` means no row met all consumption conditions. A valid ticket is consumed regardless of status.
pub async fn consume_ticket(
    pool: &Pool,
    ticket: &Ticket,
    game_id: &GameId,
) -> Result<Option<VerifiedTicket>, DbError> {
    let mut tx = pool.begin().await?;
    let Some(account_id) = sqlx::query_scalar!(
        "UPDATE entry_tickets SET consumed_at = clock_timestamp()
         WHERE ticket_hash = sha256($1) AND game_id = $2 AND consumed_at IS NULL
           AND expires_at > clock_timestamp()
         RETURNING account_id",
        ticket.expose().as_slice(),
        game_id.as_str(),
    )
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(None);
    };

    let statuses = sqlx::query!(
        "SELECT a.status AS \"account: AccountStatus\", m.status AS \"membership: MembershipStatus\"
         FROM accounts a JOIN game_memberships m USING (account_id)
         WHERE a.account_id = $1 AND m.game_id = $2",
        account_id, game_id.as_str(),
    )
    .fetch_one(&mut *tx)
    .await?;
    let account_id = AccountId::parse(&account_id)?;
    tx.commit().await?;
    Ok(Some(VerifiedTicket {
        account_id,
        membership: statuses.membership,
        account_status: statuses.account,
    }))
}

#[cfg(test)]
mod tests;
