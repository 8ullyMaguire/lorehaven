-- 0137 — §56 WIP adoption, closure notes and lineage (M45-28).
--
-- Dialect: SQLite. Columns mirror the PostgreSQL file exactly in name and order; the
-- migration-catalogue test asserts that, and types may differ by dialect.
--
-- The reasoning (no change to `works`, no foreign key on the pseud columns, the partial unique
-- index that keeps one OPEN offer per work while history accumulates) is in the PostgreSQL file
-- and is not repeated here.

CREATE TABLE IF NOT EXISTS adoption_offers (
    id                  TEXT NOT NULL PRIMARY KEY,
    work_id             TEXT NOT NULL,
    offered_by_pseud_id TEXT NOT NULL,
    author_pseud_id     TEXT NOT NULL,
    adopted_by_pseud_id TEXT,
    status              TEXT NOT NULL DEFAULT 'open',
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,
    CONSTRAINT adoption_offers_status_check
        CHECK (status IN ('open', 'withdrawn', 'adopted'))
);

CREATE UNIQUE INDEX IF NOT EXISTS adoption_offers_open_work
    ON adoption_offers (work_id) WHERE status = 'open';

CREATE INDEX IF NOT EXISTS adoption_offers_status
    ON adoption_offers (status, created_at);

CREATE TABLE IF NOT EXISTS closure_notes (
    id              TEXT   NOT NULL PRIMARY KEY,
    work_id         TEXT   NOT NULL,
    revision_number INTEGER NOT NULL,
    body            TEXT   NOT NULL,
    author_pseud_id TEXT   NOT NULL,
    created_at      TEXT   NOT NULL
);

CREATE UNIQUE INDEX IF NOT EXISTS closure_notes_revision
    ON closure_notes (work_id, revision_number);
