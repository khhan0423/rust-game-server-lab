//! Platform status changes and their transactionally coupled audit records.

use crate::login::{AccountId, AccountStatus};
use crate::{DbError, Pool};

/// State an operator can set through the sanction control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SanctionStatus {
    /// Lift an existing sanction.
    Active,
    /// Apply a sanction.
    Sanctioned,
}

/// Requested platform state and operator attribution.
#[derive(Debug)]
pub struct StatusChange<'a> {
    /// New state.
    pub status: SanctionStatus,
    /// Who requested the change.
    pub operator: &'a str,
    /// Why the change was requested.
    pub reason: &'a str,
}

/// Outcome of a status request.
#[derive(Debug, PartialEq, Eq)]
pub enum StatusOutcome {
    /// Status and audit record were both committed.
    Changed,
    /// The requested status was already in effect.
    Unchanged,
    /// Target account does not exist.
    NotFound,
    /// Pending closure is outside the sanction workflow.
    Closing,
}

/// Locks the account against concurrent login or admin writes. Status and audit commit together.
pub async fn set_status(
    pool: &Pool,
    id: &AccountId,
    change: &StatusChange<'_>,
) -> Result<StatusOutcome, DbError> {
    let after = match change.status {
        SanctionStatus::Active => AccountStatus::Active,
        SanctionStatus::Sanctioned => AccountStatus::Sanctioned,
    };
    let mut tx = pool.begin().await?;
    let Some(before) = sqlx::query_scalar!(
        "SELECT status AS \"status: AccountStatus\" FROM accounts WHERE account_id = $1 FOR UPDATE",
        id.as_str(),
    )
    .fetch_optional(&mut *tx)
    .await?
    else {
        return Ok(StatusOutcome::NotFound);
    };
    if before == after {
        return Ok(StatusOutcome::Unchanged);
    }
    if before == AccountStatus::Closing {
        return Ok(StatusOutcome::Closing);
    }
    let changed = sqlx::query!(
        "UPDATE accounts SET status = $1 WHERE account_id = $2",
        after as _,
        id.as_str()
    )
    .execute(&mut *tx)
    .await?
    .rows_affected();
    if changed != 1 {
        return Err(DbError::Inconsistent(
            "status change did not update one account",
        ));
    }
    sqlx::query!(
        "INSERT INTO ops_audit (operator, action, account_id, before_status, after_status, reason)
         VALUES ($1, 'set_status', $2, $3, $4, $5)",
        change.operator,
        id.as_str(),
        before as _,
        after as _,
        change.reason,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusOutcome::Changed)
}
