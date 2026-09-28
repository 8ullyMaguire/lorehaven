-- M59 Phase C1: body audience (spec §7.7).
--
-- Dialect: SQLite.
--
-- The counterpart of migrations/postgres/0086_body_audience.sql. The declared
-- columns and indexes must match that file exactly, because
-- `the_two_dialects_declare_the_same_columns_and_indexes` in
-- crates/db/src/migrate.rs compares them.
--
-- WHAT THIS IS. §11.15's baseline gives every eligible reader the cached body.
-- §7.7 narrows it: a work may hold its PROSE for a smaller audience than its
-- page. This is a narrowing, never a widening — the column can only remove
-- access, and an audience narrower than the instance default is the whole
-- feature.
--
-- WHY NULL AND NOT 'anyone'. The column is NULL when the work names no
-- audience, and NULL means INHERIT: whatever the source and the instance say.
-- A stored 'anyone' would mean something different — "this work is public, and
-- that overrides anything narrower above it" — and that is a WIDENING PATH. It
-- is the shape where somebody edits one work's audience in a UI dropdown, the
-- value becomes 'anyone' rather than empty, and a work that was quietly held
-- for trusted readers is now readable by everyone, with no diff anyone would
-- read carefully.
--
-- NO DEFAULT. Deliberately no `DEFAULT 'anyone'`: a default would make every
-- row inserted without the column claim an explicit 'anyone', which is the
-- widening above by another route — the column would be non-NULL everywhere and
-- the inheritance branch would be dead code that still looked alive.

ALTER TABLE works ADD COLUMN body_audience TEXT;

-- THE CONSTRAINT IS A TRIGGER, NOT A CHECK, AND THAT IS FORCED.
--
-- The first version of this file wrote
--
--     ALTER TABLE works ADD CONSTRAINT works_body_audience_valid CHECK (...)
--
-- which is valid SQL and works on a system SQLite 3.53 — and fails on the one
-- this project actually runs. sqlx's `libsqlite3-sys` BUNDLES SQLite 3.46.0
-- (confirmed in the vendored `sqlite3.h`: `#define SQLITE_VERSION "3.46.0"`),
-- and `ALTER TABLE ... ADD CONSTRAINT` arrived in SQLite 3.49.0. The symptom
-- was a migration-time
--
--     error returned from database: (code: 1) near "CONSTRAINT": syntax error
--
-- on every fresh database, before a single assertion. It is worth recording
-- that this reads as correct and passes review; a reviewer with a newer sqlite3
-- on their PATH will reproduce it working, which is the worst way for a
-- migration defect to present.
--
-- Two repairs were considered and one was chosen.
--
--   * A rename-and-recreate rebuild — the shape migration 0075 used — is the
--     one that would give a real CHECK. It is refused here. `works` is
--     referenced by 51 foreign keys across the schema, SQLite REWRITES every
--     referencing foreign key when a table is renamed, and 0075 exists
--     precisely because that rewrite silently repointed
--     `device_deliveries.export_job_id` at a dropped `export_jobs_old`. The
--     repair for that was a 12-step rebuild of the *referencing* table. Doing
--     it to `works` would touch fifty-one tables to add a column constraint,
--     and the `PRAGMA foreign_keys=OFF` window it needs is a no-op inside the
--     transaction `migrate()` runs each migration in — a problem the handoff
--     records as still unresolved and out of scope here.
--
--   * A BEFORE INSERT / BEFORE UPDATE trigger. Chosen. It enforces the same
--     values, at the same moment, with the same failure (the write is refused
--     and the offending value does not land), and it costs one table rebuild
--     of nothing at all. `the_two_dialects_declare_the_same_columns_and_indexes`
--     reads `CREATE TABLE` and `CREATE INDEX` only, so the trigger is invisible
--     to it and the parity with the PostgreSQL file holds.
--
-- WHAT THE TRIGGER COSTS, STATED PLAINLY. A trigger is not a CHECK: SQLite
-- does not report it in `PRAGMA table_info`, a schema dump does not show it,
-- and a future tool that reconstructs `works` from `sqlite_master` will lose
-- it silently. The enforcement is still real — `RAISE(ABORT)` refuses the
-- write — and the parity test does not compare constraint text in either
-- direction, so the two files agreeing is about columns and indexes, which is
-- what it has always compared.
--
-- THE STORED SPELLING. Constrained to the values `BodyAudience` can parse.
-- `trust_at_least` carries its level in the same string, separated by a colon,
-- so `trust_at_least:4` and `role_operator` are the same shape of value and the
-- parser has one split rather than two encodings. A value like
-- `trust_at_least:banana` passes the prefix test and is rejected by the parser
-- at read time, which fails closed.
--
-- This is what stops a future writer from storing a value the domain cannot
-- rank. Without it, `narrowest` meets an audience it has never heard of and
-- the honest answer is "treat it as the widest", which is a silent widening
-- caused by a typo in a migration.
--
-- `anyone` is accepted even though NULL means the same thing, because an
-- explicit 'anyone' is legal and occasionally intended, and refusing to spell
-- it would be its own kind of bug. `role_curator` and `role_vanguard` are here
-- for the same reason: they are roles, not thresholds, and the column must be
-- able to say so.
--
-- An index is NOT created, and the absence is deliberate. Nothing queries
-- "works whose audience is X" yet: the audience is read one row at a time,
-- alongside the work, by the eligibility service. An index on a column with no
-- query behind it is write amplification on every work update for the sake of a
-- future that may not arrive. When a listing surface does need it, this is where
-- the index goes, and the query that wants it is what should be written first.

CREATE TRIGGER works_body_audience_valid_insert
BEFORE INSERT ON works
FOR EACH ROW
WHEN NEW.body_audience IS NOT NULL
 AND NEW.body_audience NOT IN (
       'anyone', 'accounts_only', 'role_operator', 'role_vanguard', 'role_curator'
     )
 AND NEW.body_audience <> 'trust_at_least'
 AND NEW.body_audience NOT LIKE 'trust_at_least:%'
BEGIN
    SELECT RAISE(ABORT, 'works.body_audience is not a legal audience');
END;

CREATE TRIGGER works_body_audience_valid_update
BEFORE UPDATE OF body_audience ON works
FOR EACH ROW
WHEN NEW.body_audience IS NOT NULL
 AND NEW.body_audience NOT IN (
       'anyone', 'accounts_only', 'role_operator', 'role_vanguard', 'role_curator'
     )
 AND NEW.body_audience <> 'trust_at_least'
 AND NEW.body_audience NOT LIKE 'trust_at_least:%'
BEGIN
    SELECT RAISE(ABORT, 'works.body_audience is not a legal audience');
END;
