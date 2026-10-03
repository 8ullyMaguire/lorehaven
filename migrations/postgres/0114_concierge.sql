-- M45-22: the personal concierge (spec §54).
--
-- Identical in content to migrations/sqlite/0114_concierge.sql. Read that file's
-- header for why each table exists; this one records only what the dialect changes,
-- because those are the things that bite.
--
--   * `id`, `account_id`, `work_id` are UUID here and TEXT on SQLite, because
--     `accounts.id` and `works.id` are typed per dialect. Every store function binding
--     one of these from a `&str` needs `$n::uuid` on this side or PostgreSQL rejects
--     the bind with 42804 — the class `6160a4f` fixed two instances of.
--   * `created_at`, `notified_at` and `decided_at`-style timestamps are TIMESTAMPTZ
--     here and TEXT on SQLite. Do not let a helper script "fix" a bind on these by
--     adding a cast from the assumption that every timestamp column is TEXT:
--     `fix-timestamptz-binds.py` did exactly that and its own casts were the defect.
--   * `estimated_minutes` is DOUBLE PRECISION, not NUMERIC. The domain type is `f64`,
--     and a NUMERIC column decoded into `f64` fails at request time — or worse, is read
--     through `unwrap_or(0.0)` and silently becomes zero for every row.
--   * `work_ids` is TEXT here too, NOT JSONB. It is an ordered array of ids the
--     rendering produced. §54.1 makes it a record, not a queryable ranking input, and a
--     JSONB column would invite the index somebody builds on it next year. The store
--     encodes and decodes it, and both halves are tested together.
--   * The partial index is portable and kept, so the "who is waiting on this work"
--     query is the same query on both engines.

CREATE TABLE IF NOT EXISTS concierge_sessions (
    id                UUID PRIMARY KEY,
    account_id        UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    mood              TEXT,
    budget_minutes    INTEGER,
    work_ids          TEXT NOT NULL DEFAULT '[]',
    estimated_minutes DOUBLE PRECISION,
    truncated_at      INTEGER,
    rate_source       TEXT NOT NULL DEFAULT 'default'
                      CHECK (rate_source IN ('observed', 'default')),
    created_at        TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS concierge_sessions_account
    ON concierge_sessions (account_id, created_at DESC);

CREATE TABLE IF NOT EXISTS wip_watches (
    id                UUID PRIMARY KEY,
    account_id        UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    work_id           UUID NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    notified_at       TIMESTAMPTZ,
    created_at        TIMESTAMPTZ NOT NULL,
    UNIQUE (account_id, work_id)
);

CREATE INDEX IF NOT EXISTS wip_watches_pending ON wip_watches (work_id) WHERE notified_at IS NULL;