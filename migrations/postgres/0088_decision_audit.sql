-- M61: the decision audit trail (spec §11.14, amendment
-- docs/spec-amendments/calibrated-decision-models.md §3.5).
--
-- Dialect: PostgreSQL.
--
-- The counterpart of migrations/sqlite/0088_decision_audit.sql. The declared
-- tables, columns and indexes must match that file exactly, because
-- `the_two_dialects_declare_the_same_columns_and_indexes` in
-- crates/db/src/migrate.rs compares them. Read the SQLite file's header for the
-- reasoning: the privacy argument about `subject`, the NULLable `posterior`,
-- and why both `deterministic` and `outcome` are stored are all design
-- decisions that apply equally to both dialects, and they are explained once.
--
-- The dialect differences here are exactly two, and both are forced:
--
--   * REAL is DOUBLE PRECISION in PostgreSQL, and is exact for the 0.0..=1.0
--     range being stored. SQLite's REAL is an IEEE double too, so a poster and
--     a threshold round-trip identically on both.
--
--   * The index names are the same, but PostgreSQL's IF NOT EXISTS on CREATE
--     INDEX is only available from 9.5, which is below anything this project
--     supports, so both dialects use the same guarded form.

CREATE TABLE IF NOT EXISTS decision_audit (
    id             TEXT PRIMARY KEY,
    -- Which decision surface made the call: 'import_quality', 'positivity', ...
    task           TEXT NOT NULL,
    -- What was classified: an id. NEVER the classified text.
    subject        TEXT NOT NULL,
    -- 'accepted' | 'rejected' | 'held'.
    deterministic  TEXT NOT NULL,
    -- The model's probability, 0.0..=1.0, or NULL when no model was consulted.
    posterior      DOUBLE PRECISION,
    -- The threshold in force at the time, so the row stays readable after the
    -- operator changes it.
    threshold      DOUBLE PRECISION,
    -- What was actually applied.
    outcome        TEXT NOT NULL,
    -- 'deterministic' | 'calibrated'.
    provider       TEXT NOT NULL,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    version        INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX IF NOT EXISTS idx_decision_audit_created
    ON decision_audit (created_at DESC, id DESC);

CREATE INDEX IF NOT EXISTS idx_decision_audit_task
    ON decision_audit (task, created_at DESC);

CREATE INDEX IF NOT EXISTS idx_decision_audit_subject
    ON decision_audit (subject, created_at DESC);
