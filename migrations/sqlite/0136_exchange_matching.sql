-- 0136 — §18.11 gift exchanges: terms and the pairing (M45-27).
--
-- Dialect: SQLite. Columns mirror the PostgreSQL file exactly in name and order;
-- the migration test asserts that, and types may differ by dialect.
--
-- The reasoning (one kind-tagged table, no foreign keys, canonical timestamps,
-- matched_by) is in the PostgreSQL file and is not repeated here.

CREATE TABLE IF NOT EXISTS exchange_terms (
    event_id   TEXT NOT NULL,
    account    TEXT NOT NULL,
    kind       TEXT NOT NULL,
    value      TEXT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (event_id, account, kind, value),
    CONSTRAINT exchange_terms_kind_check
        CHECK (kind IN ('offers_fandom', 'wants_fandom', 'excludes_pseud'))
);

CREATE INDEX IF NOT EXISTS idx_exchange_terms_event
    ON exchange_terms (event_id, account);

CREATE TABLE IF NOT EXISTS exchange_assignments (
    event_id    TEXT NOT NULL,
    giver       TEXT NOT NULL,
    recipient   TEXT NOT NULL,
    assigned_at TEXT NOT NULL,
    revealed_at TEXT,
    matched_by  TEXT NOT NULL,
    PRIMARY KEY (event_id, giver),
    CONSTRAINT exchange_assignments_matched_by_check
        CHECK (matched_by IN ('forced', 'augment', 'improved')),
    CONSTRAINT exchange_assignments_not_self
        CHECK (giver <> recipient)
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_exchange_assignments_recipient
    ON exchange_assignments (event_id, recipient);
