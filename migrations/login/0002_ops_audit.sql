ALTER TABLE entry_tickets ADD COLUMN issued_at timestamptz;
ALTER TABLE entry_tickets ALTER COLUMN issued_at SET DEFAULT clock_timestamp();
-- Expiry is the only chronology available for tickets that predate this migration.
CREATE INDEX entry_tickets_account_issued ON entry_tickets (account_id, (COALESCE(issued_at, expires_at)) DESC);

CREATE TABLE ops_audit (
    id bigint GENERATED ALWAYS AS IDENTITY PRIMARY KEY,
    occurred_at timestamptz NOT NULL DEFAULT clock_timestamp(),
    operator text NOT NULL CHECK (length(btrim(operator)) BETWEEN 1 AND 128),
    action text NOT NULL,
    account_id text NOT NULL REFERENCES accounts(account_id),
    before_status smallint NOT NULL CHECK (before_status IN (0, 1, 2)),
    after_status smallint NOT NULL CHECK (after_status IN (0, 1)),
    reason text NOT NULL CHECK (length(btrim(reason)) BETWEEN 1 AND 1000),
    CHECK (before_status <> after_status)
);
