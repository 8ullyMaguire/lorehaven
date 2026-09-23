-- Migration 0066 — Taste Calibration Arena (spec §0.4.2a)
--
-- Dialect: PostgreSQL.
-- Timestamps are TIMESTAMPTZ; identifiers are UUID.

CREATE TABLE arena_ballots (
    id              UUID    PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id      UUID    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    best_work_id    UUID    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    worst_work_id   UUID    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    reason_tags     JSONB   NOT NULL DEFAULT '[]',
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now()
);

CREATE INDEX idx_arena_ballots_account ON arena_ballots (account_id);
CREATE INDEX idx_arena_ballots_best ON arena_ballots (best_work_id);
CREATE INDEX idx_arena_ballots_worst ON arena_ballots (worst_work_id);

CREATE TABLE arena_weights (
    id              UUID    PRIMARY KEY DEFAULT gen_random_uuid(),
    account_id      UUID    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    dimension_key   TEXT    NOT NULL,
    weight          DOUBLE PRECISION NOT NULL,
    elo_rating      DOUBLE PRECISION NOT NULL DEFAULT 1500.0,
    matches_played  INTEGER NOT NULL DEFAULT 0,
    created_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    UNIQUE (account_id, dimension_key)
);

CREATE INDEX idx_arena_weights_account ON arena_weights (account_id);
