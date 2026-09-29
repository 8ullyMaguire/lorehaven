-- M59 Phase D: preservation targets (spec §11.12a, §9.7 as amended by §2).
--
-- Dialect: SQLite.
--
-- The counterpart of migrations/postgres/0093_preservation.sql. The declared
-- tables, columns and indexes must match that file's, because
-- `the_two_dialects_declare_the_same_columns_and_indexes` in
-- crates/db/src/migrate.rs compares exactly that. Its parser reads CREATE TABLE
-- and CREATE INDEX, and the ADD COLUMN form here therefore does not register in
-- the catalogue -- which is deliberate and is why the columns D adds to
-- `story_identity_members` are also written into `crates/db/src/preservation.rs`
-- as a single constant that the parity of the *behaviour* depends on. The two
-- dialects must still agree, and the test
-- `a_preservation_member_row_carries_the_columns_both_dialects_declare` asserts
-- it by reading both migrations.
--
-- WHY A DESTINATION IS CONFIGURATION AND NOT A TABLE OF WORKS. §3.2: a
-- preservation destination is a named archive, its base URL, the rule that
-- identifies one of its item pages, and whether it accepts automated submission
-- at all. None of that is per-work, so none of it belongs on a work.
--
-- WHY THE TARGET IS AN IDENTITY MEMBER. A `preservation_targets` table and
-- `story_identity_members` would both answer "does this work exist at that
-- location, and what state is it in". Two tables, one fact, and the
-- reconciliation lands on whoever builds §11.10 properly next -- at which point
-- the preservation data has to be migrated, not just joined. So a target is a
-- `story_identity_members` row carrying a destination and a state, and the
-- concept exists exactly once. The consequence, stated rather than discovered
-- later: `story_identity_members` now carries columns only Phase D uses.
--
-- WHY `credits_paid` LIVES ON THE MEMBER. The credit is owed to *a destination
-- holding this work*, which is the member, and the clawback finds it by
-- (destination_id, state) -- a dead member is a dead destination. Putting the
-- column on a `work_id -> credits` table would make the clawback a join that has
-- to be right about which destination died, which is the one thing the job
-- knows.
--
-- THE PARTIAL UNIQUE INDEX IS LOAD-BEARING. A duplicate crosspost is an
-- idempotent no-op rather than a second paid target, and one destination cannot
-- back two member rows. Without it, "crosspost the same work to the same
-- archive" is a way to be paid twice for one fact.
--
-- `preservation_destinations` is ON DELETE RESTRICT: deleting a destination must
-- not silently orphan credits that were paid for it. A destination with a paid,
-- verified target is removed by disabling it (`enabled = 0`), which is the
-- reversible action, not by dropping the row.
--
-- `works.redistribution` is the §2.6 permission gate, and it is a DEPENDENCY of
-- this phase rather than an existing capability: §33 opens with "Nothing in this
-- section is implemented", so `redistribution` on `works` did not exist and this
-- migration is what makes it exist. The default is `unstated` and `unstated` is
-- treated as `ask`, because the reward structure is what makes an unstated work
-- worth asserting on, and an assertion nobody has to make is an assertion
-- nobody makes.
--
-- `works.source_key` is not a column, so "is this work local to this instance"
-- (§2.7) cannot be answered from `works` alone: an imported work is one with a
-- `library_items` row whose `work_id` is this work. That is the query the
-- store uses, and it is a query rather than a column because the moment an
-- import is deleted the work stops being imported -- a column would keep
-- answering "imported" for a work whose import is gone.

CREATE TABLE preservation_destinations (
    id                TEXT PRIMARY KEY,
    name              TEXT NOT NULL,
    base_url          TEXT NOT NULL,
    -- How one of this destination's item pages is recognised. A path template or
    -- a URL prefix, depending on the archive; the code stores it verbatim and
    -- the verification step builds the item URL from `base_url` and this.
    match_rule        TEXT NOT NULL,
    -- §3.2: whether the archive accepts automated submission at all. A
    -- destination that does not is still listable and still counts toward
    -- eligibility only if a human can post there, so this gates the *crosspost*
    -- and not the *record*: a reader who posted it by hand and this instance
    -- verified it has still done a good thing, and refusing to count it would
    -- make the metric a measure of our automation rather than of preservation.
    accepts_automated INTEGER NOT NULL DEFAULT 0,
    enabled           INTEGER NOT NULL DEFAULT 1,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    version           INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX preservation_destinations_enabled ON preservation_destinations (enabled);

-- The preservation state of one (work, destination) pair, as columns on the
-- identity member A0 already created.
--
-- `state` is 'unverified' | 'verified' | 'dead' | 'refused'. 'dead' means the
-- destination no longer carries a record naming this work -- it does NOT mean
-- the work is lost. The work's own origin reachability is a separate fact
-- reported separately (§3.3), and merging the two would tell an operator that a
-- work is gone when what is actually true is that one archive forgot it.
ALTER TABLE story_identity_members ADD COLUMN destination_id TEXT REFERENCES preservation_destinations (id) ON DELETE RESTRICT;
ALTER TABLE story_identity_members ADD COLUMN state TEXT NOT NULL DEFAULT 'unverified';
ALTER TABLE story_identity_members ADD COLUMN verified_at TEXT;
ALTER TABLE story_identity_members ADD COLUMN dead_at TEXT;
ALTER TABLE story_identity_members ADD COLUMN evidence_hash TEXT;
ALTER TABLE story_identity_members ADD COLUMN credits_paid INTEGER NOT NULL DEFAULT 0;
ALTER TABLE story_identity_members ADD COLUMN created_by TEXT REFERENCES accounts (id) ON DELETE RESTRICT;
ALTER TABLE story_identity_members ADD COLUMN updated_at TEXT;
ALTER TABLE story_identity_members ADD COLUMN version INTEGER NOT NULL DEFAULT 1;

-- One destination backs at most one member row. The WHERE is what makes it
-- partial: a local member has destination_id NULL, and a unique index over
-- NULLs would refuse the second local member this instance ever adds.
CREATE UNIQUE INDEX story_identity_members_destination ON story_identity_members (destination_id) WHERE destination_id IS NOT NULL;

-- The clawback job scans for dead destinations by state, and the leaderboard
-- counts verified ones; both are on `state` alone.
CREATE INDEX story_identity_members_state ON story_identity_members (state);

ALTER TABLE works ADD COLUMN redistribution TEXT NOT NULL DEFAULT 'unstated';
