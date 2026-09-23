-- Migration 0066 — Taste Calibration Arena (spec §0.4.2a)
--
-- Dialect: SQLite.
-- Timestamps are RFC 3339 UTC text; identifiers are canonical UUID text.

CREATE TABLE arena_ballots (
    id              TEXT    PRIMARY KEY,
    account_id      TEXT    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    best_work_id    TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    worst_work_id   TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    reason_tags     TEXT    NOT NULL DEFAULT '[]',
    created_at      TEXT    NOT NULL
);

CREATE INDEX idx_arena_ballots_account ON arena_ballots (account_id);
CREATE INDEX idx_arena_ballots_best ON arena_ballots (best_work_id);
CREATE INDEX idx_arena_ballots_worst ON arena_ballots (worst_work_id);

CREATE TABLE arena_weights (
    id              TEXT    PRIMARY KEY,
    account_id      TEXT    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    dimension_key   TEXT    NOT NULL,
    weight          REAL    NOT NULL,
    elo_rating      REAL    NOT NULL DEFAULT 1500.0,
    matches_played  INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT    NOT NULL,
    updated_at      TEXT    NOT NULL,
    UNIQUE (account_id, dimension_key)
);

CREATE INDEX idx_arena_weights_account ON arena_weights (account_id);
