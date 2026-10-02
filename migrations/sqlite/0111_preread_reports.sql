-- Gap C step 4: persisted pre-read reports.
--
-- One report per (work, task, provider) with the per-dimension scores kept as JSON,
-- rather than a row per dimension. The reasoning is in the table comment below; the short
-- version is that a report is read whole and written whole, and a per-dimension row
-- layout would need three tables' worth of joins to answer the only question anybody
-- asks ("how did this work score?").
--
-- `works.id` is TEXT here and uuid on the PostgreSQL twin — the sixth site in this
-- codebase with that split, which is why the two files exist separately rather than
-- being generated from one source.

CREATE TABLE IF NOT EXISTS preread_reports (
    id              TEXT PRIMARY KEY,
    work_id         TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    -- §23.7 requires a report to record which provider produced it, both for the
    -- "provider-specific retention and data-use disclosure" the author has to see and
    -- for withdrawing consent from one provider without discarding the other's output.
    provider        TEXT NOT NULL,
    task            TEXT NOT NULL,
    -- The operator's configured dimensions, and the verdicts keyed by dimension.
    -- A JSON object rather than a child table: the dimensions are operator
    -- configuration with no schema, so a child table would need its own definition of
    -- what a dimension is.
    dimensions      TEXT NOT NULL,
    -- Configured dimensions that did not come back, and why. NOT NULL and possibly an
    -- empty object -- an empty `missing` is the *good* state (everything was answered),
    -- and a NULL would be a third thing meaning "nobody wrote this row properly".
    missing         TEXT NOT NULL DEFAULT '{}',
    -- §32.6: "does not display composite quality scores publicly by default". There is
    -- deliberately no `score` column and no way to compute one here. A single number
    -- would be a ranking signal waiting to be wired up, and §0.3 forbids payment moving
    -- any ranking signal — so the shape of the table is the guarantee, not a convention.
    --
    -- CHECK rather than a Rust-side clamp, so a bad provider cannot write 1.7 even by
    -- accident.
    scored_at       TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    CONSTRAINT preread_dimensions_is_an_object
        CHECK (dimensions LIKE '{%'),
    CONSTRAINT preread_missing_is_an_object
        CHECK (missing LIKE '{%')
);

-- The only query is "the latest report for this work", so the index leads with work_id
-- and then scored_at: an index on work_id alone would still need a sort to pick the
-- newest.
CREATE INDEX IF NOT EXISTS idx_preread_reports_work_scored
    ON preread_reports (work_id, scored_at DESC);

-- One report per (work, provider, task). Re-scoring replaces the row rather than
-- accumulating one per run: a report is the current assessment, not a log, and a
-- history would need a retention policy that §23.7 does not give us.
CREATE UNIQUE INDEX IF NOT EXISTS idx_preread_reports_unique
    ON preread_reports (work_id, provider, task);