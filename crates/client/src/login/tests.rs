//! Exercises login transitions without a window or worker.

use std::mem::discriminant;

use super::*;
use crate::profile::tests::credentials;

enum Input {
    Start(Action),
    Finish(Outcome),
    Reset(Reset),
}

const BUSY: Buttons = Buttons {
    create: false,
    log_in: false,
    reset: ResetControls::Disabled,
};

#[test]
fn operations_follow_their_prerequisites_and_restore_controls_after_completion() {
    let received = Instant::now();
    let ticket = || Ticket {
        prefix: "12345678".into(),
        entry_url: "http://127.0.0.1:7200".into(),
        received,
        ttl: Duration::from_secs(60),
    };
    let create = Buttons {
        create: true,
        ..BUSY
    };
    let log_in = Buttons {
        log_in: true,
        reset: ResetControls::Available,
        ..BUSY
    };
    let confirmation = Buttons {
        reset: ResetControls::Confirming,
        ..BUSY
    };
    let reset_with_ticket =
        |event, buttons| (Input::Reset(event), State::TicketHeld(ticket()), buttons);
    let failed = || State::Failed("test failure".into());
    let failure = || Input::Finish(Outcome::Failed("test failure".into()));
    let mut machine = Login::new(None);
    assert!(matches!(machine.state(), State::NoAccount));
    assert_eq!(machine.buttons(), create);
    let rows = [
        (Input::Reset(Reset::Request), State::NoAccount, &create),
        (Input::Start(Action::LogIn), State::NoAccount, &create),
        (Input::Start(Action::Create), State::CreatingAccount, &BUSY),
        (Input::Start(Action::Create), State::CreatingAccount, &BUSY),
        (
            Input::Finish(Outcome::LoggedIn(ticket())),
            State::CreatingAccount,
            &BUSY,
        ),
        (failure(), failed(), &create),
        (Input::Start(Action::Create), State::CreatingAccount, &BUSY),
        (
            Input::Finish(Outcome::Created(credentials())),
            State::Ready,
            &log_in,
        ),
        (Input::Reset(Reset::Confirmed), State::Ready, &log_in),
        (Input::Start(Action::Create), State::Ready, &log_in),
        (Input::Start(Action::LogIn), State::LoggingIn, &BUSY),
        (Input::Reset(Reset::Request), State::LoggingIn, &BUSY),
        (failure(), failed(), &log_in),
        (Input::Start(Action::LogIn), State::LoggingIn, &BUSY),
        (
            Input::Finish(Outcome::LoggedIn(ticket())),
            State::TicketHeld(ticket()),
            &log_in,
        ),
        reset_with_ticket(Reset::Request, &confirmation),
        (
            Input::Start(Action::LogIn),
            State::TicketHeld(ticket()),
            &confirmation,
        ),
        reset_with_ticket(Reset::Cancel, &log_in),
        (Input::Start(Action::LogIn), State::LoggingIn, &BUSY),
        (
            Input::Finish(Outcome::LoggedIn(ticket())),
            State::TicketHeld(ticket()),
            &log_in,
        ),
        reset_with_ticket(Reset::Request, &confirmation),
        (Input::Reset(Reset::Confirmed), State::NoAccount, &create),
    ];
    for (index, (input, state, buttons)) in rows.into_iter().enumerate() {
        match input {
            Input::Start(action) => machine.begin(action),
            Input::Finish(outcome) => machine.complete(outcome),
            Input::Reset(event) => machine.reset(event),
        }
        assert_eq!(
            discriminant(machine.state()),
            discriminant(&state),
            "state after step {index}"
        );
        assert_eq!(&machine.buttons(), buttons, "controls after step {index}");
        if let (State::TicketHeld(actual), State::TicketHeld(expected)) = (machine.state(), &state)
        {
            assert_eq!(actual.prefix, expected.prefix, "ticket after step {index}");
            assert_eq!(machine.credentials(), Some(&credentials()));
            assert!(machine.is_saved());
        }
    }
    assert!(!machine.is_saved());
    assert_eq!(machine.credentials(), None);
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
    login.reset(Reset::Request);

    assert!(matches!(login.state(), State::Failed(_)));
    assert_eq!(login.credentials(), Some(&credentials));
    assert!(!login.is_saved());
    assert_eq!(
        login.buttons(),
        Buttons {
            create: false,
            log_in: true,
            reset: ResetControls::Disabled,
        }
    );
}
