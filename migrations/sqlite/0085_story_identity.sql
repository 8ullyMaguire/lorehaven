-- M59 Phase A0: cross-source identity, the minimum §11.10b needs (spec §11.10).
--
-- Dialect: SQLite.
--
-- The counterpart of migrations/postgres/0085_story_identity.sql. The declared
-- tables, columns, indexes and foreign keys must match the PostgreSQL file's,
-- because `the_two_dialects_declare_the_same_columns_and_indexes` in
-- crates/db/src/migrate.rs compares exactly that. Two of that parser's rules are
-- visible here: an index body must sit on ONE line (it scans line by line for
-- ` ON `, so a wrapped body is invisible to it), and an index name is read as
-- the last whitespace token before ` ON` — which is why there is no
-- `IF NOT EXISTS` prefix on the index below.
--
-- NUMBERING. The M59 plan called this 0084, written before 0083 shipped. 0083
-- reserves 0084 for a *different* change — the `bot_registrations.token_id`
-- foreign key (gap D5), which cannot ride in 0083 because SQLite cannot
-- `ALTER TABLE ... ADD CONSTRAINT` and PostgreSQL needs a UUID->TEXT column type
-- migration first. So identity is 0085 here and the plan's later phases shift to
-- 0086/0087/0088. Nothing has been built against the old numbers.
--
-- WHAT THIS IS. §11.10 names four tables for cross-source identity and none of
-- them has ever existed. The plan's first draft assumed they did; a probe found
-- otherwise. What does exist is `library_items` (0006) keyed on
-- UNIQUE(account_id, source_key, source_work_key), which means importing the
-- same fic from two sites produces two unrelated rows and no link. This phase
-- builds the *minimum* Phase D needs -- the grouping and its members -- and
-- nothing else. No merge machinery, no inference, no preservation state; that
-- last arrives in 0087 as a column on this row rather than a parallel table a
-- later identity milestone would have to reconcile.
--
-- WHY NO INFERENCE PATH IS BUILT. `edition_relation` ships with exactly one
-- value, `cross_posted`, because that is the only relation this instance can
-- establish without guessing: it performed the act itself. A member created by
-- title-and-author similarity, by canonical URL, or by a shared tag set tells a
-- reader that two different texts are one book, and no later fix removes the
-- wrong linkage a reader already believed. The migration therefore has no
-- backfill and no trigger, and `every_member_row_names_a_crosspost_this_instance
-- _performed` is the test that keeps it that way.

CREATE TABLE story_identities (
    id                TEXT PRIMARY KEY,
    -- The local work this identity is about. A crosspost this instance performed
    -- always has one; an identity with no local member is a purely-external
    -- grouping, which A0 does not create. NOT NULL for that reason -- an
    -- identity nothing local can be shown on is not something a reader can ever
    -- be shown, so storing one would be storing something unreachable.
    work_id           TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    -- Canonical metadata is a COPY of the work's, not the authority. §11.10's
    -- merges are reversible, so an identity must be able to be dissolved back
    -- into its members without a work losing its title. A reader-facing page
    -- shows the work's current title; this column exists so that a merge, when
    -- one exists, can record what the group was called while it was a group.
    canonical_title   TEXT NOT NULL DEFAULT '',
    status            TEXT NOT NULL DEFAULT 'active',
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    version           INTEGER NOT NULL DEFAULT 1
);

-- The edition list on a work page: this instance's copy, then each external
-- location. A member row records THAT a copy exists and WHERE. Never a body --
-- §11.10's "do not grant access to another edition's body" is structural here:
-- there is no column to put one in, so a reader holding this instance's copy
-- gains nothing from any member row.
CREATE TABLE story_identity_members (
    id                  TEXT PRIMARY KEY,
    identity_id         TEXT NOT NULL REFERENCES story_identities (id) ON DELETE CASCADE,
    -- Exactly one of the two. A local member names a work; an external member
    -- names a record at a site. Both are CHECK-enforced, because a row with both
    -- or neither is a row that means two things, and a reader cannot be shown a
    -- row that means two things.
    work_id             TEXT REFERENCES works (id) ON DELETE CASCADE,
    external_record_id  TEXT,
    -- One value ships: 'cross_posted'. See the note above the DDL.
    edition_relation    TEXT NOT NULL DEFAULT 'cross_posted',
    external_source_key TEXT,
    external_url        TEXT,
    created_at          TEXT NOT NULL,
    UNIQUE (identity_id, work_id),
    UNIQUE (identity_id, external_source_key, external_url),
    CHECK ((work_id IS NULL) <> (external_record_id IS NULL))
);

-- The work page reads the local member by identity_id, and the members list
-- reads the external ones by the same column; one index serves both because
-- UNIQUE(identity_id, work_id) already indexes the local half and this covers
-- the external half. One line, no IF NOT EXISTS -- both required by the parity
-- test named above.
CREATE INDEX story_identity_members_identity ON story_identity_members (identity_id);

-- §3's two merge tables are created EMPTY, so the data model is honest about
-- having them and `the_two_merge_tables_exist_and_nothing_writes_to_them` can
-- assert the second half. Nothing in this phase inserts into either.
CREATE TABLE identity_merge_proposals (
    id            TEXT PRIMARY KEY,
    -- The two identities being proposed for union. A proposal is a claim, not a
    -- fact, so both sides are named symmetrically here.
    left_identity  TEXT NOT NULL REFERENCES story_identities (id) ON DELETE CASCADE,
    right_identity TEXT NOT NULL REFERENCES story_identities (id) ON DELETE CASCADE,
    state          TEXT NOT NULL DEFAULT 'proposed',
    rationale      TEXT NOT NULL DEFAULT '',
    created_at     TEXT NOT NULL,
    resolved_at    TEXT,
    -- A proposal for a pair already proposed must not be a second row.
    UNIQUE (left_identity, right_identity)
);

CREATE TABLE identity_merge_history (
    id           TEXT PRIMARY KEY,
    -- The identity the merge was applied to. NOT NULL with a cascade: a
    -- recorded merge that outlived its identity would be a history of an event
    -- that cannot be checked against anything.
    identity_id  TEXT NOT NULL REFERENCES story_identities (id) ON DELETE CASCADE,
    proposal_id  TEXT REFERENCES identity_merge_proposals (id) ON DELETE SET NULL,
    -- What the merge did, verbatim. A reversible merge needs a record of the
    -- prior state, not just that a merge happened -- so this is a JSON-ish TEXT
    -- and Phase E, not this phase, is what writes it.
    detail       TEXT NOT NULL DEFAULT '',
    created_at   TEXT NOT NULL
);

-- The proposal list is read by state, and the history by recency.
CREATE INDEX identity_merge_proposals_state ON identity_merge_proposals (state, created_at);
CREATE INDEX identity_merge_history_identity ON identity_merge_history (identity_id, created_at);
