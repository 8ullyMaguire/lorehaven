-- 0133 — §58 supply: keystone authors, and the routing record (M45-25).
--
-- Why a snapshot table and not a view: keystone detection ranks each work among
-- the engaged rows of its OWN source and shrinks toward 0.5 by n/(n+10), which is
-- a sort plus a window the SQLite arm cannot express without a ranked CTE this
-- crate does not build anywhere else. The detection runs in Rust over the 1,247
-- engaged rows and REPLACES this table, so a page read is one index scan and the
-- numbers a caller sees are one run's, not a mixture of two.
--
-- Scores are stored in BASIS POINTS (0..10000) rather than floating point. The
-- crate's convention is to bind String and i64 only; a DOUBLE here would be the
-- one column where a driver's float handling could disagree between engines, and
-- a percentile is a number we compare and never integrate.
--
-- No foreign keys on supply_routing: a routing row is evidence that a reader was
-- asked, and this row's whole point is that the record survives. Erasure coverage
-- for the account column belongs to §51's cascade list (M45-51), not here.

CREATE TABLE IF NOT EXISTS supply_keystones (
    author_key    TEXT    PRIMARY KEY,
    display_name  TEXT    NOT NULL,
    engaged_works INTEGER NOT NULL,
    mean_pct_bp   INTEGER NOT NULL,
    taste_bonus_bp INTEGER NOT NULL,
    score_bp      INTEGER NOT NULL,
    computed_at   TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_supply_keystones_score
    ON supply_keystones (score_bp DESC, author_key ASC);

CREATE TABLE IF NOT EXISTS supply_routing (
    id            TEXT    PRIMARY KEY,
    account_id    TEXT    NOT NULL,
    work_id       TEXT    NOT NULL,
    author_key    TEXT    NOT NULL,
    reason        TEXT    NOT NULL,
    created_at    TEXT    NOT NULL,
    completed_at  TEXT,
    credit_bp     INTEGER,
    UNIQUE (account_id, work_id)
);

CREATE INDEX IF NOT EXISTS idx_supply_routing_account
    ON supply_routing (account_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_supply_routing_work
    ON supply_routing (work_id);
