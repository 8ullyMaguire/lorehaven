-- M59 Phase A0: cross-source identity, the minimum §11.10b needs (spec §11.10).
--
-- Dialect: PostgreSQL.
--
-- The counterpart of migrations/sqlite/0085_story_identity.sql, and the file
-- that carries the reasoning: read it first. Every table, column, index and
-- foreign key here must match the SQLite file's, because
-- `the_two_dialects_declare_the_same_columns_and_indexes` in
-- crates/db/src/migrate.rs compares exactly that, and it reads an index name as
-- the last whitespace token before ` ON` -- which is why there is no
-- `IF NOT EXISTS` prefix on any index below.
--
-- NUMBERING: the M59 plan called this 0084, written before 0083 shipped. 0083
-- reserves 0084 for the `bot_registrations.token_id` foreign key (gap D5), so
-- identity is 0085 and the plan's later phases shift to 0086/0087/0088.
--
-- POSTGRESQL-ONLY DECISIONS, all of them forced by this dialect rather than
-- chosen:
--
--   * `id` is TEXT here, not UUID. `works (id)` is TEXT on this schema, and a
--     foreign key's type must match what it references. Migrations 0001-0022
--     used UUID and later work moved to TEXT, so a new table follows the table
--     it references rather than the schema's oldest habit.
--
--   * `created_at`/`updated_at` are TEXT, matching `works` and every other
--     timestamp in this schema. A TIMESTAMPTZ column would be more correct in
--     isolation and would not be comparable with the row it describes.
--
--   * The `UNIQUE (identity_id, work_id)` and
--     `UNIQUE (identity_id, external_source_key, external_url)` constraints are
--     the same ones SQLite has. PostgreSQL treats multiple NULLs as distinct in
--     a unique index, so a local member (work_id set, the other two NULL) never
--     collides with an external one, and neither do two external members that
--     share neither key. SQLite agrees here. This is worth stating because the
--     usual NULL-in-unique-index divergence runs the other way, and the CHECK
--     below is what actually guarantees the two-halves invariant on both.

CREATE TABLE story_identities (
    id                TEXT PRIMARY KEY,
    -- The local work this identity is about. A crosspost this instance performed
    -- always has one; an identity with no local member is a purely-external
    -- grouping, which A0 does not create.
    -- `UUID`, not `TEXT`: `works.id` is `UUID` on PostgreSQL (0003), and
    -- PostgreSQL will not create a foreign key between a TEXT column and a
    -- UUID one. The first version of this file declared it TEXT, mirroring the
    -- SQLite arm, and the entire PostgreSQL migration chain died at 0085 with
    --
    --     foreign key constraint "story_identities_work_id_fkey"
    --     cannot be implemented
    --
    -- on every scratch database, so every dual-backend test in the project
    -- failed during `migrate()` before a single assertion. This is the same
    -- divergence the handoff records for the ten `TEXT` pseud foreign keys
    -- (`comments.author_pseud` and friends): the SQLite arm is TEXT because
    -- SQLite is dynamically typed and accepts either, and mirroring it into
    -- PostgreSQL is the mistake.
    --
    -- The *local* id columns of these two tables stay TEXT: they are generated
    -- by this instance, not referenced by anything, and nothing in this phase
    -- joins them to a UUID. Only the two columns that reference `works` are
    -- retyped.
    work_id           UUID NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    -- Canonical metadata is a COPY of the work's, not the authority. §11.10's
    -- merges are reversible, so an identity must be able to be dissolved back
    -- into its members without a work losing its title.
    canonical_title   TEXT NOT NULL DEFAULT '',
    status            TEXT NOT NULL DEFAULT 'active',
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    version           INTEGER NOT NULL DEFAULT 1
);

-- The edition list on a work page: this instance's copy, then each external
-- location. A member row records THAT a copy exists and WHERE. Never a body --
-- there is no column to put one in.
CREATE TABLE story_identity_members (
    id                  TEXT PRIMARY KEY,
    identity_id         TEXT NOT NULL REFERENCES story_identities (id) ON DELETE CASCADE,
    -- Exactly one of the two, CHECK-enforced: a row with both or neither is a
    -- row that means two things, and a reader cannot be shown a row that means
    -- two things.
    -- `UUID` for the same reason as `story_identities.work_id` above.
    work_id             UUID REFERENCES works (id) ON DELETE CASCADE,
    external_record_id  TEXT,
    -- One value ships: 'cross_posted'. See the SQLite file for why no inference
    -- path may add a second.
    edition_relation    TEXT NOT NULL DEFAULT 'cross_posted',
    external_source_key TEXT,
    external_url        TEXT,
    created_at          TEXT NOT NULL,
    UNIQUE (identity_id, work_id),
    UNIQUE (identity_id, external_source_key, external_url),
    CHECK ((work_id IS NULL) <> (external_record_id IS NULL))
);

-- The work page reads the local member by identity_id, and the members list
-- reads the external ones by the same column.
CREATE INDEX story_identity_members_identity ON story_identity_members (identity_id);

-- §3's two merge tables, created EMPTY. Nothing in this phase writes to either;
-- Phase E is what proposes a merge, and it is not built here.
CREATE TABLE identity_merge_proposals (
    id            TEXT PRIMARY KEY,
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
    identity_id  TEXT NOT NULL REFERENCES story_identities (id) ON DELETE CASCADE,
    proposal_id  TEXT REFERENCES identity_merge_proposals (id) ON DELETE SET NULL,
    -- What the merge did, verbatim. A reversible merge needs a record of the
    -- prior state, not just that a merge happened.
    detail       TEXT NOT NULL DEFAULT '',
    created_at   TEXT NOT NULL
);

-- The proposal list is read by state, and the history by recency.
CREATE INDEX identity_merge_proposals_state ON identity_merge_proposals (state, created_at);
CREATE INDEX identity_merge_history_identity ON identity_merge_history (identity_id, created_at);
