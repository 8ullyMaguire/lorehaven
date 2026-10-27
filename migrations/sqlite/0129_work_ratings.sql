-- 0129 — the works ladder (docs/spec-amendments/arena-work-ladder.md, §12.4a).
-- Identical to the PostgreSQL migration with SQLite's types: TEXT ids, REAL for
-- the three rating numbers, INTEGER for the count.
CREATE TABLE work_ratings (
    work_id    TEXT PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    mu         REAL NOT NULL,
    phi        REAL NOT NULL,
    sigma      REAL NOT NULL,
    matches    INTEGER NOT NULL DEFAULT 0,
    rated_at   TEXT NOT NULL
);

CREATE INDEX work_ratings_mu ON work_ratings (mu DESC);
