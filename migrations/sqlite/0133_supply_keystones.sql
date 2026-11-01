-- 0133 — §58 supply: keystone authors, and the routing record (M45-25).
--
-- The PostgreSQL twin carries the reasoning; this file is kept in step statement
-- for statement so the parity test can compare the two declared column sets.
--
-- Scores are basis points (0..10000) for the same reason the PG file gives: this
-- crate binds strings and integers, and a percentile that is only ever compared
-- does not need a float column.

CREATE TABLE IF NOT EXISTS supply_keystones (
    author_key     TEXT    PRIMARY KEY,
    display_name   TEXT    NOT NULL,
    engaged_works  INTEGER NOT NULL,
    mean_pct_bp    INTEGER NOT NULL,
    taste_bonus_bp INTEGER NOT NULL,
    score_bp       INTEGER NOT NULL,
    computed_at    TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_supply_keystones_score
    ON supply_keystones (score_bp DESC, author_key ASC);

CREATE TABLE IF NOT EXISTS supply_routing (
    id           TEXT    PRIMARY KEY,
    account_id   TEXT    NOT NULL,
    work_id      TEXT    NOT NULL,
    author_key   TEXT    NOT NULL,
    reason       TEXT    NOT NULL,
    created_at   TEXT    NOT NULL,
    completed_at TEXT,
    credit_bp    INTEGER,
    UNIQUE (account_id, work_id)
);

CREATE INDEX IF NOT EXISTS idx_supply_routing_account
    ON supply_routing (account_id, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_supply_routing_work
    ON supply_routing (work_id);
