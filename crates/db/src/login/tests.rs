//! Login database behavior against the migrated PostgreSQL schema.

use super::*;

const SECRET: [u8; 32] = [1; 32];

struct Fixture {
    account_id: AccountId,
    guest_id: GuestId,
    secret: GuestSecret,
    game_id: GameId,
}

impl Fixture {
    async fn create(pool: &Pool) -> Self {
        let guest = Self {
            account_id: AccountId::new("018f22ce-8c1f-7000-8000-000000000001".to_owned()),
            guest_id: GuestId::new("018f22ce-8c1f-7000-8000-000000000002".to_owned()),
            secret: GuestSecret::new(SECRET),
            game_id: GameId::new("lab".to_owned()),
        };
        create_guest(pool, &guest.account_id, &guest.credentials())
            .await
            .unwrap();
        guest
    }

    const fn credentials(&self) -> GuestCredentials<'_> {
        GuestCredentials {
            guest_id: &self.guest_id,
            secret: &self.secret,
        }
    }

    const fn grant<'a>(&'a self, ticket: &'a Ticket, ttl_secs: u32) -> EntryGrant<'a> {
        EntryGrant {
            game_id: &self.game_id,
            ticket,
            ttl_secs,
        }
    }
}

#[sqlx::test(migrator = "LOGIN_MIGRATOR")]
async fn guest_creation_stores_the_hash_instead_of_the_secret(pool: Pool) {
    let guest = Fixture::create(&pool).await;

    let hash = sqlx::query_scalar!(
        "SELECT secret_hash FROM account_identities WHERE subject = $1",
        guest.guest_id.as_str()
    )
    .fetch_one(&pool)
    .await
    .unwrap()
    .unwrap();

    // SHA-256 of 32 bytes of value 1, calculated independently of LoginDB.
    assert_eq!(
        hash,
        [
            114, 205, 110, 132, 34, 196, 7, 251, 109, 9, 134, 144, 241, 19, 11, 125, 237, 126, 194,
            247, 245, 225, 211, 11, 217, 213, 33, 240, 21, 54, 55, 147
        ]
    );
}

#[sqlx::test(migrator = "LOGIN_MIGRATOR")]
async fn wrong_secret_and_unknown_guest_are_rejected_identically(pool: Pool) {
    let guest = Fixture::create(&pool).await;
    let wrong_secret = GuestSecret::new([2; 32]);
    let unknown = GuestId::new("018f22ce-8c1f-7000-8000-000000000003".to_owned());
    let ticket = Ticket::new([3; 32]);
    let grant = guest.grant(&ticket, 60);

    for (guest_id, secret) in [(&guest.guest_id, &wrong_secret), (&unknown, &guest.secret)] {
        let outcome = login_guest(&pool, &GuestCredentials { guest_id, secret }, &grant)
            .await
            .unwrap();
        assert_eq!(outcome, LoginOutcome::InvalidCredentials);
    }
}

#[sqlx::test(migrator = "LOGIN_MIGRATOR")]
async fn platform_and_game_sanctions_block_login_but_do_not_block_verification(pool: Pool) {
    let guest = Fixture::create(&pool).await;
    let ticket = Ticket::new([3; 32]);
    let pending = Ticket::new([4; 32]);
    let credentials = guest.credentials();
    assert!(matches!(
        login_guest(&pool, &credentials, &guest.grant(&ticket, 60))
            .await
            .unwrap(),
        LoginOutcome::Issued(_)
    ));
    let grant = guest.grant(&pending, 60);

    for status in [
        AccountStatus::Sanctioned,
        AccountStatus::Closing,
        AccountStatus::Active,
    ] {
        sqlx::query!(
            "UPDATE accounts SET status = $1 WHERE account_id = $2",
            status as _,
            guest.account_id.as_str()
        )
        .execute(&pool)
        .await
        .unwrap();
        let membership = match status {
            AccountStatus::Sanctioned | AccountStatus::Closing => MembershipStatus::Member,
            AccountStatus::Active => MembershipStatus::Sanctioned,
        };
        sqlx::query!(
            "UPDATE game_memberships SET status = $1 WHERE account_id = $2 AND game_id = $3",
            membership as _,
            guest.account_id.as_str(),
            guest.game_id.as_str()
        )
        .execute(&pool)
        .await
        .unwrap();

        assert!(matches!(
            login_guest(&pool, &credentials, &grant).await.unwrap(),
            LoginOutcome::AccountBlocked(_)
        ));
    }
    sqlx::query!(
        "UPDATE accounts SET status = $1 WHERE account_id = $2",
        AccountStatus::Closing as _,
        guest.account_id.as_str()
    )
    .execute(&pool)
    .await
    .unwrap();

    assert_eq!(
        consume_ticket(&pool, &ticket, &guest.game_id)
            .await
            .unwrap(),
        Some(VerifiedTicket {
            account_id: guest.account_id,
            membership: MembershipStatus::Sanctioned,
            account_status: AccountStatus::Closing,
        })
    );
}

#[sqlx::test(migrator = "LOGIN_MIGRATOR")]
async fn only_a_fresh_ticket_for_the_right_game_can_be_consumed_once(pool: Pool) {
    let guest = Fixture::create(&pool).await;
    let expired = Ticket::new([3; 32]);
    let fresh = Ticket::new([4; 32]);
    let unknown = Ticket::new([5; 32]);
    let wrong_game = GameId::new("other".to_owned());
    for (ticket, ttl) in [(&expired, 0), (&fresh, 60)] {
        assert!(matches!(
            login_guest(&pool, &guest.credentials(), &guest.grant(ticket, ttl))
                .await
                .unwrap(),
            LoginOutcome::Issued(_)
        ));
    }

    for (ticket, game) in [
        (&expired, &guest.game_id),
        (&fresh, &wrong_game),
        (&unknown, &guest.game_id),
    ] {
        assert_eq!(consume_ticket(&pool, ticket, game).await.unwrap(), None);
    }
    assert_eq!(
        consume_ticket(&pool, &fresh, &guest.game_id).await.unwrap(),
        Some(VerifiedTicket {
            account_id: guest.account_id,
            membership: MembershipStatus::Member,
            account_status: AccountStatus::Active,
        })
    );
    assert_eq!(
        consume_ticket(&pool, &fresh, &guest.game_id).await.unwrap(),
        None
    );
}

#[sqlx::test(migrator = "LOGIN_MIGRATOR")]
async fn repeated_login_keeps_one_membership_and_restores_it_after_cancellation(pool: Pool) {
    let guest = Fixture::create(&pool).await;

    for (sequence, canceled) in [(3, false), (4, false), (5, true)] {
        if canceled {
            sqlx::query!(
                "UPDATE game_memberships SET status = $1 WHERE account_id = $2 AND game_id = $3",
                MembershipStatus::Canceled as _,
                guest.account_id.as_str(),
                guest.game_id.as_str()
            )
            .execute(&pool)
            .await
            .unwrap();
        }
        let ticket = Ticket::new([sequence; 32]);
        assert!(matches!(
            login_guest(&pool, &guest.credentials(), &guest.grant(&ticket, 60))
                .await
                .unwrap(),
            LoginOutcome::Issued(_)
        ));
        let statuses = sqlx::query_scalar!("SELECT status AS \"status: MembershipStatus\" FROM game_memberships WHERE account_id = $1 AND game_id = $2", guest.account_id.as_str(), guest.game_id.as_str())
            .fetch_all(&pool).await.unwrap();
        assert_eq!(statuses, [MembershipStatus::Member]);
    }
}
