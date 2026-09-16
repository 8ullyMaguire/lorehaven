-- M23 §32.2 — canon and space entity tables for scoped media queries.
-- "All media in canon Z" and "all media in space Z" are acceptance criteria
-- of spec §32.2. These tables hold the grouping; the doors JOIN through them.
-- PG twin: UUID ids, REAL timestamps, BIGINT position.

CREATE TABLE IF NOT EXISTS canons (
    id              UUID PRIMARY KEY,
    name            TEXT NOT NULL,
    description     TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    version         INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS canon_works (
    canon_id        UUID NOT NULL REFERENCES canons (id) ON DELETE CASCADE,
    work_id         UUID NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    position        INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL,
    PRIMARY KEY (canon_id, work_id)
);
CREATE INDEX IF NOT EXISTS idx_canon_works_work ON canon_works(work_id);

CREATE TABLE IF NOT EXISTS spaces (
    id              UUID PRIMARY KEY,
    name            TEXT NOT NULL,
    description     TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    version         INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE IF NOT EXISTS space_works (
    space_id        UUID NOT NULL REFERENCES spaces (id) ON DELETE CASCADE,
    work_id         UUID NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    position        INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL,
    PRIMARY KEY (space_id, work_id)
);
CREATE INDEX IF NOT EXISTS idx_space_works_work ON space_works(work_id);
