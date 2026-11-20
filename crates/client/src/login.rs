//! Pure state transitions for the guest login controls.

use std::time::{Duration, Instant};

use crate::profile::Credentials;

/// An operation requested through the login panel.
#[derive(Clone, Copy, Debug)]
pub(super) enum Action {
    Create,
    LogIn,
}

/// Login button availability.
#[derive(Debug, PartialEq, Eq)]
pub(super) struct Buttons {
    pub(super) create: bool,
    pub(super) log_in: bool,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::profile::tests::credentials;

    enum Input {
        Start(Action),
        Finish(Outcome),
    }

    #[test]
    fn operations_follow_their_prerequisites_and_restore_controls_after_completion() {
        use std::mem::discriminant;

        let ticket = || Ticket {
            prefix: "12345678".into(),
            entry_url: "http://127.0.0.1:7200".into(),
            received: Instant::now(),
            ttl: Duration::from_secs(60),
        };
        let create = Buttons {
            create: true,
            log_in: false,
        };
        let log_in = Buttons {
            create: false,
            log_in: true,
        };
        let busy = Buttons {
            create: false,
            log_in: false,
        };
        let failed = || State::Failed("test failure".into());
        let failure = || Input::Finish(Outcome::Failed("test failure".into()));
        let mut machine = Login::new(None);
        assert!(matches!(machine.state(), State::NoAccount));
        assert_eq!(machine.buttons(), create);
        let rows = [
            (Input::Start(Action::LogIn), State::NoAccount, &create),
            (Input::Start(Action::Create), State::CreatingAccount, &busy),
            (Input::Start(Action::Create), State::CreatingAccount, &busy),
            (
                Input::Finish(Outcome::LoggedIn(ticket())),
                State::CreatingAccount,
                &busy,
            ),
            (failure(), failed(), &create),
            (Input::Start(Action::Create), State::CreatingAccount, &busy),
            (
                Input::Finish(Outcome::Created(credentials())),
                State::Ready,
                &log_in,
            ),
            (Input::Start(Action::Create), State::Ready, &log_in),
            (Input::Start(Action::LogIn), State::LoggingIn, &busy),
            (failure(), failed(), &log_in),
            (Input::Start(Action::LogIn), State::LoggingIn, &busy),
            (
                Input::Finish(Outcome::LoggedIn(ticket())),
                State::TicketHeld(ticket()),
                &log_in,
            ),
            (Input::Start(Action::LogIn), State::LoggingIn, &busy),
        ];
        for (index, (input, state, buttons)) in rows.into_iter().enumerate() {
            match input {
                Input::Start(action) => machine.begin(action),
                Input::Finish(outcome) => machine.complete(outcome),
            }
            assert_eq!(
                discriminant(machine.state()),
                discriminant(&state),
                "state after step {index}"
            );
            assert_eq!(&machine.buttons(), buttons, "controls after step {index}");
        }
        assert!(machine.is_saved());
        let restored = Login::new(Some(credentials()));
        assert!(matches!(restored.state(), State::Ready));
        assert_eq!(restored.buttons(), log_in);
    }

    #[test]
    fn save_failure_preserves_the_only_copy_of_the_guest_credentials() {
        let mut login = Login::new(None);
        let credentials = credentials();
        login.begin(Action::Create);

        login.complete(Outcome::Unsaved {
            credentials: credentials.clone(),
            error: "disk full".into(),
        });

        assert!(matches!(login.state(), State::Failed(_)));
        assert_eq!(login.credentials(), Some(&credentials));
        assert!(!login.is_saved());
        assert_eq!(
            login.buttons(),
            Buttons {
                create: false,
                log_in: true
            }
        );
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
        match self.state {
            State::CreatingAccount | State::LoggingIn => Buttons {
                create: false,
                log_in: false,
            },
            State::NoAccount | State::Ready | State::TicketHeld(_) | State::Failed(_) => Buttons {
                create: self.credentials.is_none(),
                log_in: self.credentials.is_some(),
            },
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
