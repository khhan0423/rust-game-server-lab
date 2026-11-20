CREATE TABLE accounts (
    account_id text PRIMARY KEY,
    status smallint NOT NULL CHECK (status IN (0, 1, 2)),
    created_at timestamptz NOT NULL
);

CREATE TABLE account_identities (
    provider smallint NOT NULL CHECK (provider IN (0, 1, 2, 3)),
    subject text NOT NULL,
    account_id text NOT NULL REFERENCES accounts(account_id),
    secret_hash bytea,
    linked_at timestamptz NOT NULL,
    PRIMARY KEY (provider, subject),
    UNIQUE (account_id, provider)
);

CREATE TABLE game_memberships (
    account_id text NOT NULL REFERENCES accounts(account_id),
    game_id text NOT NULL,
    status smallint NOT NULL CHECK (status IN (0, 1, 2)),
    joined_at timestamptz NOT NULL,
    PRIMARY KEY (account_id, game_id)
);

CREATE TABLE entry_tickets (
    ticket_hash bytea PRIMARY KEY,
    account_id text NOT NULL REFERENCES accounts(account_id),
    game_id text NOT NULL,
    expires_at timestamptz NOT NULL,
    consumed_at timestamptz
);
