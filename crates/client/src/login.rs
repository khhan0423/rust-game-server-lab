//! Pure state transitions for the guest login controls.

use std::time::{Duration, Instant};

use crate::profile::Credentials;

/// An operation requested through the login panel.
#[derive(Clone, Copy, Debug)]
pub(super) enum Action {
    Create,
    LogIn,
}

/// Events in the local account reset flow.
#[derive(Clone, Copy, Debug)]
pub(super) enum Reset {
    Request,
    Cancel,
    Confirmed,
}

/// The reset controls that the login panel can show.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ResetControls {
    Disabled,
    Available,
    Confirming,
}

/// Login button availability.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Buttons {
    pub(super) create: bool,
    pub(super) log_in: bool,
    pub(super) reset: ResetControls,
}

/// The non-secret details of the most recent entry ticket.
#[derive(Debug)]
pub(super) struct Ticket {
    pub(super) prefix: String,
    pub(super) entry_url: String,
    pub(super) received: Instant,
    pub(super) ttl: Duration,
}

impl Ticket {
    /// Reports how much of the ticket lifetime remains at the supplied time.
    pub(super) fn remaining(&self, now: Instant) -> Duration {
        self.ttl
            .saturating_sub(now.saturating_duration_since(self.received))
    }
}

/// A completed operation, including credentials kept in memory if saving failed.
#[derive(Debug)]
pub(super) enum Outcome {
    Created(Credentials),
    Unsaved {
        credentials: Credentials,
        error: String,
    },
    LoggedIn(Ticket),
    Failed(String),
}

/// Current status displayed in the login panel.
#[derive(Debug)]
pub(super) enum State {
    NoAccount,
    Ready,
    CreatingAccount,
    LoggingIn,
    TicketHeld(Ticket),
    Failed(String),
}

/// State owned by the UI thread; transitions perform no I/O.
#[derive(Debug)]
pub(super) struct Login {
    credentials: Option<Credentials>,
    saved: bool,
    state: State,
    confirming_reset: bool,
}

impl Login {
    /// Starts with saved credentials or an empty profile.
    pub(super) const fn new(credentials: Option<Credentials>) -> Self {
        let saved = credentials.is_some();
        Self {
            credentials,
            saved,
            state: if saved {
                State::Ready
            } else {
                State::NoAccount
            },
            confirming_reset: false,
        }
    }

    /// Returns the current status.
    pub(super) const fn state(&self) -> &State {
        &self.state
    }

    /// Returns the guest credentials available to this session.
    pub(super) const fn credentials(&self) -> Option<&Credentials> {
        self.credentials.as_ref()
    }

    /// Returns whether the current guest credentials exist on disk.
    pub(super) const fn is_saved(&self) -> bool {
        self.saved
    }

    /// Returns controls that can start an operation in the current state.
    pub(super) const fn buttons(&self) -> Buttons {
        let idle = match self.state {
            State::CreatingAccount | State::LoggingIn => false,
            State::NoAccount | State::Ready | State::TicketHeld(_) | State::Failed(_) => true,
        } && !self.confirming_reset;
        Buttons {
            create: idle && self.credentials.is_none(),
            log_in: idle && self.credentials.is_some(),
            reset: if self.confirming_reset {
                ResetControls::Confirming
            } else if idle && self.saved {
                ResetControls::Available
            } else {
                ResetControls::Disabled
            },
        }
    }

    /// Updates confirmation, or forgets the account once its file has been removed.
    pub(super) fn reset(&mut self, event: Reset) {
        match event {
            Reset::Request if matches!(self.buttons().reset, ResetControls::Available) => {
                self.confirming_reset = true;
            }
            Reset::Cancel => self.confirming_reset = false,
            Reset::Confirmed if self.confirming_reset => *self = Self::new(None),
            Reset::Request | Reset::Confirmed => {}
        }
    }

    /// Records an operation after the command has been accepted by the worker queue.
    pub(super) fn begin(&mut self, action: Action) {
        let buttons = self.buttons();
        match action {
            Action::Create if buttons.create => self.state = State::CreatingAccount,
            Action::LogIn if buttons.log_in => self.state = State::LoggingIn,
            Action::Create | Action::LogIn => {}
        }
    }

    /// Applies a worker outcome only to the operation it completes.
    pub(super) fn complete(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Created(credentials) if matches!(self.state, State::CreatingAccount) => {
                self.credentials = Some(credentials);
                self.saved = true;
                self.state = State::Ready;
            }
            Outcome::Unsaved { credentials, error }
                if matches!(self.state, State::CreatingAccount) =>
            {
                self.credentials = Some(credentials);
                self.state = State::Failed(error);
            }
            Outcome::LoggedIn(ticket) if matches!(self.state, State::LoggingIn) => {
                self.state = State::TicketHeld(ticket);
            }
            Outcome::Failed(error)
                if matches!(self.state, State::CreatingAccount | State::LoggingIn) =>
            {
                self.state = State::Failed(error);
            }
            Outcome::Created(_)
            | Outcome::Unsaved { .. }
            | Outcome::LoggedIn(_)
            | Outcome::Failed(_) => {}
        }
    }
}

#[cfg(test)]
mod tests;
