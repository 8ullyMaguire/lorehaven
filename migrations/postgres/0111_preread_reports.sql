-- Gap C step 4: persisted pre-read reports — PostgreSQL twin of
-- migrations/sqlite/0111_preread_reports.sql. The rules and the reasoning are the same;
-- the enforcement differs in three places, all of which PostgreSQL does better:
--
--   * `works.id` is uuid here and TEXT on SQLite, so `work_id` is UUID. This is the
--     sixth table in the codebase with that split.
--   * the JSON columns are `JSONB` rather than TEXT with a LIKE check. Same content,
--     but queryable, and no risk of a malformed object being written.
--   * no `LIKE '{%'` CHECKs: `JSONB` cannot hold a scalar, so the "is an object" rule is
--     structural rather than a constraint someone has to remember to add.
--
-- What is identical and deliberate: there is no `score` column, and no way to compute a
-- composite here. §32.6 forbids displaying composite quality scores publicly and §0.3
-- forbids payment moving any ranking signal. The table's shape is the guarantee.

CREATE TABLE IF NOT EXISTS preread_reports (
    id              UUID PRIMARY KEY,
    work_id         UUID NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    -- §23.7: "provider-specific retention and data-use disclosure", and withdrawing
    -- consent from one provider without discarding another provider's output.
    provider        TEXT NOT NULL,
    task            TEXT NOT NULL,
    -- Operator-configured dimensions keyed by dimension. JSONB so the value is
    -- guaranteed to be an object -- a scalar cannot be stored, so the SQLite version's
    -- LIKE check is unnecessary here.
    dimensions      JSONB NOT NULL,
    -- Configured dimensions that did not come back, and why. An empty object is the good
    -- state (everything was answered); a SQL NULL would be a third thing meaning
    -- "nobody wrote this row properly", so NOT NULL with a default instead.
    missing         JSONB NOT NULL DEFAULT '{}'::jsonb,
    -- No composite score column, on purpose. See the header.
    scored_at       TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    CONSTRAINT preread_dimensions_is_an_object
        CHECK (jsonb_typeof(dimensions) = 'object'),
    CONSTRAINT preread_missing_is_an_object
        CHECK (jsonb_typeof(missing) = 'object')
);

-- The only query is "the latest report for this work".
CREATE INDEX IF NOT EXISTS idx_preread_reports_work_scored
    ON preread_reports (work_id, scored_at DESC);

-- One report per (work, provider, task). Re-scoring replaces rather than accumulates:
-- a report is the current assessment, not a log, and a history would need a retention
-- policy §23.7 does not give us.
CREATE UNIQUE INDEX IF NOT EXISTS idx_preread_reports_unique
    ON preread_reports (work_id, provider, task);