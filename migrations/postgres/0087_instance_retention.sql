-- M59 Phase C: instance work body retention (spec §11.15).
--
-- Dialect: PostgreSQL.
--
-- The counterpart of migrations/sqlite/0087_instance_retention.sql. The
-- declared tables, columns and indexes must match that file exactly, because
-- `the_two_dialects_declare_the_same_columns_and_indexes` in
-- crates/db/src/migrate.rs compares them. That test compares column NAMES, not
-- types, which is why `updated_by` is UUID here and TEXT in the SQLite file
-- without being a divergence: `accounts.id` is a UUID in this dialect, and a
-- TEXT foreign key onto a UUID column is a type error at insert rather than a
-- schema that merely looks different.
--
-- The reasoning for the shape — singleton row, `cache` default, the
-- SET NULL on `updated_by`, and why an override may only narrow — is written
-- out in the SQLite file, which is the one that carries the comments. This file
-- deliberately repeats none of it: two copies of a rationale drift.

CREATE TABLE IF NOT EXISTS instance_retention_policy (
    id          TEXT PRIMARY KEY,
    -- 'cache' | 'aggregate'.
    body_mode   TEXT NOT NULL DEFAULT 'cache',
    -- Who changed it. ON DELETE SET NULL: deleting the account that made a
    -- retention decision must not delete the decision.
    updated_by  UUID REFERENCES accounts (id) ON DELETE SET NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    version     INTEGER NOT NULL DEFAULT 1
);

-- The singleton. `WHERE id = 'default'` rather than a bare UNIQUE(id), so a
-- second row cannot be inserted under an id differing by one byte.
CREATE UNIQUE INDEX IF NOT EXISTS idx_instance_retention_policy_singleton
    ON instance_retention_policy (id) WHERE id = 'default';

-- Per-source narrowing (spec §11.15).
CREATE TABLE IF NOT EXISTS instance_retention_source_overrides (
    source_key TEXT PRIMARY KEY,
    -- 'cache' | 'aggregate'. The reverse of the instance's own mode is refused
    -- by name at the setter.
    body_mode  TEXT NOT NULL,
    updated_by UUID REFERENCES accounts (id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    version    INTEGER NOT NULL DEFAULT 1
);
