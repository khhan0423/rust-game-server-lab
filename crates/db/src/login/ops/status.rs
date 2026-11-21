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

/// Why an operator change cannot be attributed.
#[derive(Debug, Clone, Copy, thiserror::Error)]
pub enum AttributionError {
    /// The supplied operator name does not meet the audit requirements.
    #[error("operator name must be 1 to 128 printable ASCII bytes after trimming")]
    Operator,
    /// The supplied reason does not meet the audit requirements.
    #[error("audit reason must be 1 to 1000 characters after trimming, without NUL")]
    Reason,
}

/// Validated name recorded on an operator change.
#[derive(Debug, Clone)]
pub struct OperatorName(String);

impl TryFrom<&str> for OperatorName {
    type Error = AttributionError;

    fn try_from(text: &str) -> Result<Self, Self::Error> {
        let text = text.trim();
        if text.is_empty() || text.len() > 128 || !text.bytes().all(|b| (b' '..=b'~').contains(&b))
        {
            return Err(AttributionError::Operator);
        }
        Ok(Self(text.to_owned()))
    }
}

/// Validated explanation recorded on an operator change.
#[derive(Debug)]
pub struct AuditReason<'a>(&'a str);

impl<'a> TryFrom<&'a str> for AuditReason<'a> {
    type Error = AttributionError;

    fn try_from(text: &'a str) -> Result<Self, Self::Error> {
        let text = text.trim();
        if text.is_empty() || text.contains('\0') || text.chars().count() > 1000 {
            return Err(AttributionError::Reason);
        }
        Ok(Self(text))
    }
}

/// Requested platform state and operator attribution.
#[derive(Debug)]
pub struct StatusChange<'a> {
    /// New state.
    status: SanctionStatus,
    /// Who requested the change.
    operator: &'a OperatorName,
    /// Why the change was requested.
    reason: AuditReason<'a>,
}

impl<'a> StatusChange<'a> {
    /// Builds a change from validated attribution.
    pub const fn new(
        status: SanctionStatus,
        operator: &'a OperatorName,
        reason: AuditReason<'a>,
    ) -> Self {
        Self {
            status,
            operator,
            reason,
        }
    }
}

#[cfg(test)]
#[path = "audit_tests.rs"]
mod tests;

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
        id.to_string(),
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
        id.to_string()
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
        change.operator.0.as_str(),
        id.to_string(),
        before as _,
        after as _,
        change.reason.0,
    )
    .execute(&mut *tx)
    .await?;
    tx.commit().await?;
    Ok(StatusOutcome::Changed)
}
