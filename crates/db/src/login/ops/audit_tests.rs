//! Verifies that status changes and their audit record cannot commit independently.

use super::*;

const PAGE: ops::Page = ops::Page {
    limit: 20,
    offset: 0,
};

#[sqlx::test(migrator = "LOGIN_MIGRATOR")]
async fn status_and_audit_commit_together_and_audit_failure_rolls_back(pool: Pool) {
    let guest = Fixture::create(&pool).await;
    let invalid = ops::StatusChange {
        status: ops::SanctionStatus::Sanctioned,
        operator: "operator",
        reason: "",
    };
    assert!(
        ops::set_status(&pool, &guest.account_id, &invalid)
            .await
            .is_err()
    );
    let detail = ops::detail(&pool, &guest.account_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.account.status, AccountStatus::Active);

    let valid = ops::StatusChange {
        reason: "policy violation",
        ..invalid
    };
    assert_eq!(
        ops::set_status(&pool, &guest.account_id, &valid)
            .await
            .unwrap(),
        ops::StatusOutcome::Changed
    );
    let detail = ops::detail(&pool, &guest.account_id)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(detail.account.status, AccountStatus::Sanctioned);
    let audit = ops::audit(&pool, PAGE).await.unwrap();
    assert_eq!(audit.len(), 1);
    assert_eq!(audit[0].account_id, guest.account_id);
    assert_eq!(audit[0].before, AccountStatus::Active);
    assert_eq!(audit[0].after, AccountStatus::Sanctioned);
    assert_eq!(audit[0].reason, valid.reason);
}

#[sqlx::test(migrator = "LOGIN_MIGRATOR")]
async fn setting_the_current_status_writes_no_audit_record(pool: Pool) {
    let guest = Fixture::create(&pool).await;
    let change = ops::StatusChange {
        status: ops::SanctionStatus::Active,
        operator: "operator",
        reason: "reviewed account",
    };
    assert_eq!(
        ops::set_status(&pool, &guest.account_id, &change)
            .await
            .unwrap(),
        ops::StatusOutcome::Unchanged
    );
    assert!(ops::audit(&pool, PAGE).await.unwrap().is_empty());
}
