-- M59 Phase D: preservation targets (spec §11.12a, §9.7 as amended by §2).
--
-- Dialect: PostgreSQL.
--
-- The counterpart of migrations/sqlite/0093_preservation.sql. The declared
-- tables and indexes must match that file's, because
-- `the_two_dialects_declare_the_same_columns_and_indexes` in
-- crates/db/src/migrate.rs compares exactly that. The SQLite file carries the
-- reasoning; it is not repeated. What differs here, and it is not only syntax:
--
-- * `works.redistribution` gets a real CHECK. PostgreSQL accepts
--   `ALTER TABLE ... ADD CONSTRAINT ... CHECK`, and this column is the one
--   place in Phase D where a bad value would be silently treated as `ask` and
--   therefore silently pay less rather than refuse -- a wrong permission read
--   in the direction of *paying* is the failure worth a constraint. SQLite gets
--   the same guarantee from the store's parser (which fails closed), and the
--   two are asserted to accept the same values by
--   `every_redistribution_value_the_store_accepts_is_accepted_by_the_schema`.
--
-- * The `destination_id` reference is `UUID`-typed against
--   `preservation_destinations.id`, which is declared TEXT here and is TEXT in
--   the SQLite file. The reference is written to match, so the constraint is
--   legal in both dialects; `created_by` references `accounts(id)`, which IS a
--   UUID in this schema, so that one takes `::uuid` in the store's statements
--   rather than in the DDL. Migration 0085 records the same asymmetry in
--   prose: `story_identity_members.id` stays TEXT because nothing references
--   it, and anything that does get the type the referenced column has.

CREATE TABLE preservation_destinations (
    id                TEXT PRIMARY KEY,
    name              TEXT NOT NULL,
    base_url          TEXT NOT NULL,
    match_rule        TEXT NOT NULL,
    accepts_automated INTEGER NOT NULL DEFAULT 0,
    enabled           INTEGER NOT NULL DEFAULT 1,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    version           INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX preservation_destinations_enabled ON preservation_destinations (enabled);

ALTER TABLE story_identity_members ADD COLUMN destination_id TEXT REFERENCES preservation_destinations (id) ON DELETE RESTRICT;
ALTER TABLE story_identity_members ADD COLUMN state TEXT NOT NULL DEFAULT 'unverified';
ALTER TABLE story_identity_members ADD COLUMN verified_at TEXT;
ALTER TABLE story_identity_members ADD COLUMN dead_at TEXT;
ALTER TABLE story_identity_members ADD COLUMN evidence_hash TEXT;
ALTER TABLE story_identity_members ADD COLUMN credits_paid INTEGER NOT NULL DEFAULT 0;
ALTER TABLE story_identity_members ADD COLUMN created_by TEXT REFERENCES accounts (id) ON DELETE RESTRICT;
ALTER TABLE story_identity_members ADD COLUMN updated_at TEXT;
ALTER TABLE story_identity_members ADD COLUMN version INTEGER NOT NULL DEFAULT 1;

CREATE UNIQUE INDEX story_identity_members_destination ON story_identity_members (destination_id) WHERE destination_id IS NOT NULL;
CREATE INDEX story_identity_members_state ON story_identity_members (state);

ALTER TABLE works ADD COLUMN redistribution TEXT NOT NULL DEFAULT 'unstated';
ALTER TABLE works ADD CONSTRAINT works_redistribution_valid
  CHECK (redistribution IN ('yes', 'ask', 'no', 'unstated'));
