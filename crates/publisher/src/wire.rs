//! Publisher vocabulary for ticket verification and operator responses.
use db::login::{AccountStatus, MembershipStatus};

pub(super) const fn operator_status(value: AccountStatus) -> &'static str {
    match value {
        AccountStatus::Active => "active",
        AccountStatus::Sanctioned => "sanctioned",
        AccountStatus::Closing => "closing",
    }
}
pub(super) const fn ticket_sanction(value: AccountStatus) -> &'static str {
    match value {
        AccountStatus::Active => "none",
        other @ (AccountStatus::Sanctioned | AccountStatus::Closing) => operator_status(other),
    }
}
pub(super) const fn membership(value: MembershipStatus) -> &'static str {
    match value {
        MembershipStatus::Member => "member",
        MembershipStatus::Canceled => "canceled",
        MembershipStatus::Sanctioned => "sanctioned",
    }
}
