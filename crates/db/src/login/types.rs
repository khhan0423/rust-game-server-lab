//! Strong types for login database operations and outcomes.

use std::fmt;

/// Publisher account identifier, stored as text in the login database.
#[derive(Debug, PartialEq, Eq)]
pub struct AccountId(String);

impl AccountId {
    /// Wraps the account identifier assigned by the caller.
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// Borrows the identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Subject of a guest login identity.
#[derive(Debug)]
pub struct GuestId(String);

impl GuestId {
    /// Wraps the subject assigned by the caller.
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// Borrows the subject.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// Identifies a game in the publisher's catalog.
#[derive(Debug, PartialEq, Eq, Hash)]
pub struct GameId(String);

impl GameId {
    /// Wraps a game identifier.
    pub const fn new(value: String) -> Self {
        Self(value)
    }

    /// Borrows the identifier.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::borrow::Borrow<str> for GameId {
    fn borrow(&self) -> &str {
        self.as_str()
    }
}

/// The original bytes of a guest credential. Its debug representation is redacted.
pub struct GuestSecret([u8; 32]);

impl GuestSecret {
    /// Wraps a credential supplied by the caller.
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Exposes the bytes for hashing or the one-time creation response.
    pub const fn expose(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for GuestSecret {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GuestSecret(***)")
    }
}

/// The original bytes of a single-use entry ticket. Its debug representation is redacted.
pub struct Ticket([u8; 32]);

impl Ticket {
    /// Wraps a ticket supplied by the caller.
    pub const fn new(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Exposes the bytes for hashing or the issuance response.
    pub const fn expose(&self) -> &[u8; 32] {
        &self.0
    }
}

impl fmt::Debug for Ticket {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Ticket(***)")
    }
}

/// Identifies the guest and proves possession of the guest credential.
#[derive(Debug)]
pub struct GuestCredentials<'a> {
    /// Guest identity subject.
    pub guest_id: &'a GuestId,
    /// Credential for that subject.
    pub secret: &'a GuestSecret,
}

/// The ticket to issue when login succeeds.
#[derive(Debug)]
pub struct EntryGrant<'a> {
    /// Game to bind to the ticket.
    pub game_id: &'a GameId,
    /// Caller-generated ticket.
    pub ticket: &'a Ticket,
    /// Ticket lifetime in seconds.
    pub ttl_secs: u32,
}

/// Platform-wide account state, matching the database `smallint` constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[repr(i16)]
pub enum AccountStatus {
    /// In good standing.
    Active = 0,
    /// Under a platform sanction.
    Sanctioned = 1,
    /// Pending closure.
    Closing = 2,
}

/// Game-specific account state, matching the database `smallint` constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type)]
#[repr(i16)]
pub enum MembershipStatus {
    /// An active member of the game.
    Member = 0,
    /// Membership was canceled and can be restored by login.
    Canceled = 1,
    /// Under a sanction for this game.
    Sanctioned = 2,
}

/// Result of authenticating a guest and requesting a ticket.
#[derive(Debug, PartialEq, Eq)]
pub enum LoginOutcome {
    /// Ticket was issued to this account.
    Issued(AccountId),
    /// Guest does not exist or provided the wrong secret.
    InvalidCredentials,
    /// Platform or membership state prevents login.
    AccountBlocked(AccountId),
}

/// Current platform and membership state of the owner of a consumed ticket.
#[derive(Debug, PartialEq, Eq)]
pub struct VerifiedTicket {
    /// Owner of the ticket.
    pub account_id: AccountId,
    /// Owner's current membership in this game.
    pub membership: MembershipStatus,
    /// Owner's current platform state.
    pub account_status: AccountStatus,
}
