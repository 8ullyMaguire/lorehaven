-- M46-04: work-level warnings, tag votes, and the indexing policy.
--
-- The PostgreSQL half of `migrations/sqlite/0105_warnings_votes_policy.sql`, which
-- is authoritative on the reasoning. Read that file first; this one records only
-- what differs between the dialects.
--
-- Three differences, all type or capability rather than design:
--
--   1. `works.id` and every other id here is `uuid`, matching 0011 and the rest of
--      the PostgreSQL schema (`taxonomy_nodes.id` is the exception: it is TEXT on
--      BOTH dialects, so `warning_node_id` and `node_id` are TEXT below).
--   2. Uniqueness over a nullable column. PostgreSQL 15 supports
--      `UNIQUE NULLS NOT DISTINCT`, which states the intent directly. The SQLite
--      half uses a unique index on `coalesce(chapter_id, '')` because SQLite rejects
--      an expression inside a table-level UNIQUE constraint. Both refuse the same
--      duplicate; only the spelling differs.
--   3. Foreign keys to `works(id)` are real here and on SQLite alike, but they are
--      only *enforced* on SQLite when `PRAGMA foreign_keys=ON`, which the app's pool
--      sets (`crates/db/src/lib.rs`) and a bare `psql`/`sqlite3` session does not.

CREATE TABLE work_warnings (
    -- UUID, like every other works reference in this schema.
    work_id         UUID NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    -- NULL means the warning applies to the work as a whole. A chapter-scoped
    -- warning is what makes "ch 7, past the reveal" representable.
    --
    -- No foreign key to `chapters` here: this migration applies to instances whose
    -- chapter table is named differently across milestones, and an FK to a possibly
    -- absent table fails the whole migration. The store validates it, and a later
    -- migration can add the constraint once the name is settled.
    chapter_id      TEXT,
    -- TEXT, not UUID: `taxonomy_nodes.id` is TEXT on both dialects.
    warning_node_id TEXT NOT NULL REFERENCES taxonomy_nodes(id) ON DELETE CASCADE,
    severity        INTEGER NOT NULL DEFAULT 1,
    depiction       TEXT NOT NULL DEFAULT 'on_page',
    declaration     TEXT NOT NULL DEFAULT 'declared',
    spoiler_safe_note TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    -- `none_apply` and `creator_chose_not_to_say` stay distinct: collapsing them
    -- would let a creator's silence read as a clean bill of health.
    --
    -- CHECKed rather than left to the store, because an unrecognised value here
    -- would be stored and then ignored by whatever filter reads it -- the
    -- silent-wrong-answer failure. The plan lists these columns as bare names with
    -- no domain, so the domains are pinned here.
    CONSTRAINT work_warnings_severity_ck CHECK (severity IN (1, 2)),
    CONSTRAINT work_warnings_depiction_ck
        CHECK (depiction IN ('on_page', 'referenced', 'implied')),
    CONSTRAINT work_warnings_declaration_ck
        CHECK (declaration IN ('declared', 'none_apply', 'creator_chose_not_to_say', 'reader_flagged')),
    -- One warning per (work, chapter, node), with a NULL chapter treated as equal to
    -- a NULL chapter -- which a plain UNIQUE does not do on this engine either.
    CONSTRAINT work_warnings_uniq
        UNIQUE NULLS NOT DISTINCT (work_id, chapter_id, warning_node_id)
);

-- "Everything tagged as a warning on this work", and behind it "works carrying this
-- warning" -- which is what a reader filtering for something they must avoid runs.
CREATE INDEX work_warnings_work_idx ON work_warnings (work_id);
CREATE INDEX work_warnings_node_idx ON work_warnings (warning_node_id);

-- ── Tag votes ────────────────────────────────────────────────────────────────
--
-- The reader layer behind `work_tags.confidence`. `voter_pseud_id` is pseudonymous
-- and there is deliberately no way to reverse it: §11.17 treats a reader headcount as
-- not derivable. The store is the only writer and it stores no mapping.
CREATE TABLE work_tag_votes (
    work_id        UUID NOT NULL,
    -- TEXT, matching taxonomy_nodes.id on both dialects.
    node_id        TEXT NOT NULL REFERENCES taxonomy_nodes(id) ON DELETE CASCADE,
    voter_pseud_id TEXT NOT NULL,
    vote           INTEGER NOT NULL,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    PRIMARY KEY (work_id, node_id, voter_pseud_id),
    -- An unchecked vote weight would let any integer into the average, and there is
    -- no other place that would catch it.
    CONSTRAINT work_tag_votes_vote_ck CHECK (vote IN (-1, 0, 1))
);

-- The confidence recomputation reads every vote for one tag across works, so the
-- index leads with node_id rather than following the primary key's order.
CREATE INDEX work_tag_votes_node_idx ON work_tag_votes (node_id);
CREATE INDEX work_tag_votes_voter_idx ON work_tag_votes (voter_pseud_id);

-- ── Indexing policy ──────────────────────────────────────────────────────────
--
-- Per-work, because a remote instance's visibility rules are not uniform across its
-- catalogue. §7's federation bullet requires the policy to travel with the activity.
--
-- `fetch_remote` and `allow_embedding` are separate decisions: a work can be held
-- for lexical search and still be excluded from embedding on cost or privacy
-- grounds, and `allow_embedding` defaults to 0 because §47.10/§49.9 forbid inferred
-- data influencing anything.
--
-- The three flags are INTEGER here as well as on SQLite. This schema stores
-- booleans as 0/1 on both engines (see `work_characters.is_pov` in 0103), and a
-- column that is BOOLEAN on one engine and INTEGER on the other is exactly the drift
-- the catalogue parity check exists to catch.
CREATE TABLE work_index_policy (
    work_id         UUID NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    fetch_remote    INTEGER NOT NULL DEFAULT 1,
    allow_embedding INTEGER NOT NULL DEFAULT 0,
    allow_passages  INTEGER NOT NULL DEFAULT 1,
    -- NULL means no expiry, which is distinct from "expires at the epoch".
    index_until     TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    PRIMARY KEY (work_id),
    CONSTRAINT work_index_policy_fetch_ck CHECK (fetch_remote IN (0, 1)),
    CONSTRAINT work_index_policy_embedding_ck CHECK (allow_embedding IN (0, 1)),
    CONSTRAINT work_index_policy_passages_ck CHECK (allow_passages IN (0, 1))
);

-- The slow lane reads "everything whose embedding is allowed", so the index leads
-- with that column.
CREATE INDEX work_index_policy_embedding_idx ON work_index_policy (allow_embedding);
CREATE INDEX work_index_policy_until_idx ON work_index_policy (index_until);

-- ── Passages ─────────────────────────────────────────────────────────────────
--
-- The chunked body the slow lane produces and a search result quotes back. Offsets
-- are CHARACTER offsets, not byte offsets: byte offsets into UTF-8 are not what a
-- snippet boundary means, and mixing the two cuts a character in half.
--
-- `model_version` is load-bearing, not bookkeeping: without it a re-embed is
-- indistinguishable from a search across mixed vector spaces, which returns
-- confident nonsense.
CREATE TABLE work_passages (
    id           TEXT PRIMARY KEY,
    work_id      UUID NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    chapter_id   TEXT,
    seq          INTEGER NOT NULL,
    text         TEXT NOT NULL,
    start_offset INTEGER NOT NULL,
    end_offset   INTEGER NOT NULL,
    model_version TEXT,
    created_at   TEXT NOT NULL,
    CONSTRAINT work_passages_offsets_ck
        CHECK (start_offset >= 0 AND end_offset >= start_offset)
);

-- Two passages of the same chapter cannot occupy the same position. As with
-- work_warnings, chapter_id is nullable, so this needs the NULL-equal-to-NULL form.
CREATE UNIQUE INDEX work_passages_uniq
    ON work_passages (work_id, chapter_id, seq);

-- "Passages of this work, in order" carries the ordering in the index rather than
-- leaving a sort in the engine.
CREATE INDEX work_passages_work_idx ON work_passages (work_id, seq);
-- A model-version sweep must find every passage computed by one model.
CREATE INDEX work_passages_model_idx ON work_passages (model_version);
