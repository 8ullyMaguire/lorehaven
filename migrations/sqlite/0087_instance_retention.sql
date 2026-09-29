-- M59 Phase C: instance work body retention (spec §11.15).
--
-- Dialect: SQLite.
--
-- The counterpart of migrations/postgres/0087_instance_retention.sql. The
-- declared tables, columns and indexes must match that file exactly, because
-- `the_two_dialects_declare_the_same_columns_and_indexes` in
-- crates/db/src/migrate.rs compares them.
--
-- WHAT THIS IS. §11.15 asks one question: does this instance hold the words of
-- the works it knows about, or is it a catalogue of links? The answer is the
-- operator's, it is stated once, and it applies to the instance — never to a
-- request, a work, an importer, an extension or a federated peer.
--
-- WHY A SINGLETON ROW RATHER THAN A COLUMN ON settings. Two reasons, both
-- load-bearing. First, the row is the OPERATOR'S RECORDED DECISION: it carries
-- who set it and when, so the modlog entry and the setting are the same row and
-- cannot disagree. Second, the alternative -- a key in the existing settings
-- table -- would make the value configurable by editing a file, which §11.15
-- explicitly refuses ("The setting is the operator's, and it is stated once"),
-- and which would make a retention change an unreviewable edit rather than an
-- auditable action.
--
-- THE DEFAULT IS 'cache'. §11.15 names cache as the default, and it is also
-- the behaviour of every instance built before this migration, so a default of
-- 'aggregate' would silently take away storage from a running instance. The
-- direction of the default is the safe one: a body already fetched is content
-- a reader may already have been served, and 'aggregate' would have to
-- un-serve it (see §11.15: narrowing does NOT delete bodies -- that is the
-- deletion workflow of §10.4, and a policy change must not do it).
--
-- WHY THE OVERRIDE TABLE MAY ONLY NARROW, AND WHY IT IS NOT ENFORCED HERE.
-- §11.15: "an override may only narrow". The check lives in the setter, not in
-- this table, because the check needs the instance value and a CHECK constraint
-- cannot see another table's row in either dialect. What the table DOES carry
-- is the reason the value is safe to write blind: `body_mode` is
-- 'aggregate' or 'cache', and 'cache' is only legal here when the instance is
-- already 'cache'. A writer that skips the setter therefore cannot widen.

CREATE TABLE IF NOT EXISTS instance_retention_policy (
    id          TEXT PRIMARY KEY,
    -- 'cache' | 'aggregate'. NOT NULL with a default so a row inserted without
    -- the column is still a complete decision rather than a NULL the reader has
    -- to interpret.
    body_mode   TEXT NOT NULL DEFAULT 'cache',
    -- Who changed it. ON DELETE SET NULL, not CASCADE: deleting the account
    -- that made a retention decision must not delete the decision. The modlog
    -- entry it belongs to outlives its actor too, and a policy row that
    -- vanished with its operator would silently re-fall-back to the default
    -- while every UI still showed 'cache'.
    updated_by  TEXT REFERENCES accounts (id) ON DELETE SET NULL,
    created_at  TEXT NOT NULL,
    updated_at  TEXT NOT NULL,
    version     INTEGER NOT NULL DEFAULT 1
);

-- The singleton. A partial index on the literal id, so a second row cannot be
-- inserted by an id that differs by one byte: a plain UNIQUE(id) would allow
-- 'default ' and 'Default' to coexist and `SELECT ... WHERE id = 'default'`
-- would then read whichever the planner preferred.
CREATE UNIQUE INDEX IF NOT EXISTS idx_instance_retention_policy_singleton
    ON instance_retention_policy (id) WHERE id = 'default';

-- Per-source narrowing (spec §11.15: "Caching most sources while aggregating
-- one is expressible; the reverse on an aggregate instance is not").
CREATE TABLE IF NOT EXISTS instance_retention_source_overrides (
    source_key TEXT PRIMARY KEY,
    -- 'cache' | 'aggregate'. The only value that can ever be stored on an
    -- aggregating instance is 'aggregate'; the reverse is refused by name at the
    -- setter, because restoring storage the operator removed is not something a
    -- per-source override may do behind the instance setting's back.
    body_mode  TEXT NOT NULL,
    updated_by TEXT REFERENCES accounts (id) ON DELETE SET NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    version    INTEGER NOT NULL DEFAULT 1
);
