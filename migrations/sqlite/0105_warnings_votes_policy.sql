-- M46-04: work-level warnings, tag votes, and the indexing policy.
--
-- The last of `advanced-search-as-built.md` §2's schema. The taxonomy graph is
-- 0104 and the character/relationship substrate is 0103, which also added
-- `work_tags.prominence` / `source` / `confidence` / `status` — verified present,
-- so nothing here re-adds them.
--
-- ── A correction to the plan: `work_warnings` is a SIBLING, not a re-key ─────
--
-- The plan says "Re-key `content_warnings` from `post_id` to works and chapters".
-- Doing that would break live code:
--
--   * `content_warnings.post_id` is a **forum post**, not a work — it is TEXT and
--     references `forum_posts(id)` (migration 0039). It is written and read by
--     `crates/db/src/spoilers.rs::list_content_warnings` and
--     `insert_content_warning`, reached from `POST /spoilers/...` in
--     `routes/spoilers.rs`, and pinned by five tests in
--     `crates/app/tests/milestone_34_spoilers.rs` plus `milestone_34.rs`.
--   * Re-keying would mean a table rebuild of a table with a live route behind it,
--     which is the operation 0082 and 0103 both decline on the same grounds.
--
-- The two answer different questions and both are needed:
--
--   * `content_warnings` — what a *reader* marked on a *forum post*. Free text,
--     no taxonomy, keyed to a post. That is a reader action, not a work property.
--   * `work_warnings` (this migration) — what the *author declared* about a *work*
--     or one of its chapters. Taxonomy-typed, so it is searchable with the rest of
--     the work's metadata, and it carries the declaration and depiction columns
--     that make "creator chose not to say" expressible.
--
-- Keeping them separate also means a work warning is never silently attached to a
-- post that does not belong to the work, which a single overloaded table would
-- allow.
--
-- Note `warning_node_id` is TEXT and references `taxonomy_nodes(id)`, matching every
-- other node reference in this schema (0011 types `taxonomy_nodes.id` as TEXT on
-- BOTH dialects). Warnings are therefore ordinary taxonomy nodes of kind 'warning',
-- which the plan's widened kind list already includes.

CREATE TABLE work_warnings (
    work_id        TEXT NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    -- NULL means the warning applies to the work as a whole. A chapter-scoped
    -- warning is the one that makes "ch 7, past the reveal" representable, which
    -- a work-only table cannot express.
    --
    -- No foreign key to `chapters` here: this migration is applied to instances
    -- whose chapter table is named differently across milestones, and an FK to a
    -- possibly-absent table fails the whole migration. The store validates it, and
    -- a later migration can add the constraint once the name is settled.
    chapter_id     TEXT,
    -- Taxonomy-typed so warnings are searchable with the rest of a work's metadata
    -- rather than living in free text that no query can reach.
    warning_node_id TEXT NOT NULL REFERENCES taxonomy_nodes(id) ON DELETE CASCADE,
    -- 1 = light, 2 = heavy. Kept as an integer rather than a label because
    -- filters compare it, and a label would need a collation to order.
    severity       INTEGER NOT NULL DEFAULT 1,
    -- How the content appears: on_page | referenced | implied. The distinction
    -- matters for a reader filtering, because "implied" is not a warning anyone
    -- can skip past by scrolling.
    depiction      TEXT NOT NULL DEFAULT 'on_page',
    -- Who said so, and in what capacity:
    --   declared                  the author stated it
    --   none_apply                the author states no warnings apply
    --   creator_chose_not_to_say  deliberately unspecified, which is NOT the same
    --                             as none_apply and must never be displayed as one
    --   reader_flagged            a reader asserted it
    -- The plan's `none_apply` and `creator_chose_not_to_say` are kept distinct on
    -- purpose: collapsing them would let a creator's silence read as a clean bill
    -- of health, which is the failure §6's privacy section warns about.
    declaration    TEXT NOT NULL DEFAULT 'declared',
    -- Free text a creator can write for a reader who wants to avoid the detail
    -- without reading it. NULL rather than '' so "no note" is distinguishable.
    spoiler_safe_note TEXT,
    created_at     TEXT NOT NULL,
    -- The plan lists severity, depiction and declaration as bare column names with
    -- no domain, so the domains are pinned here. CHECK constraints inside CREATE
    -- TABLE are native to both engines -- unlike the ALTER-able constraints in 0104,
    -- which needed triggers on SQLite -- so there is no dialect split for these.
    --
    -- Without them a severity of 3 or a depiction of 'hinted' is stored and then
    -- ignored by whatever filter reads it, which is the silent-wrong-answer failure
    -- this schema exists to prevent. `none_apply` and `creator_chose_not_to_say`
    -- stay separate for the reason given above.
    --
    -- The last column deliberately carries no trailing comma: SQLite rejects a
    -- dangling comma before the closing paren with `near ")": syntax error`, while
    -- PostgreSQL accepts it. That asymmetry is silent until the default engine runs.
    updated_at     TEXT NOT NULL,
    CHECK (severity IN (1, 2)),
    CHECK (depiction IN ('on_page', 'referenced', 'implied')),
    CHECK (declaration IN ('declared', 'none_apply', 'creator_chose_not_to_say', 'reader_flagged'))
);

-- Uniqueness for (work, chapter, node), expressed as an expression INDEX because
-- chapter_id is nullable and NULL does not compare equal to NULL on either engine.
--
-- A table-level `UNIQUE (work_id, coalesce(chapter_id, ''), warning_node_id)` is not
-- an option: measured on SQLite, "expressions prohibited in PRIMARY KEY and UNIQUE
-- constraints". `UNIQUE NULLS NOT DISTINCT` states it directly on PostgreSQL 15+
-- but is not portable, so the index below is the form both engines enforce --
-- verified to refuse the duplicate work-scoped warning on both.
CREATE UNIQUE INDEX work_warnings_uniq
    ON work_warnings (work_id, coalesce(chapter_id, ''), warning_node_id);

-- The query that matters is "everything tagged as a warning on this work", and the
-- one behind it is "works carrying this warning", which is what a reader filtering
-- for a trope they must avoid actually runs. Both are indexed rather than left to
-- a scan of the whole table.
CREATE INDEX work_warnings_work_idx ON work_warnings (work_id);
CREATE INDEX work_warnings_node_idx ON work_warnings (warning_node_id);

-- ── Tag votes ────────────────────────────────────────────────────────────────
--
-- The reader layer for tag confidence. `work_tags.confidence` is the aggregate;
-- this is the evidence behind it, one row per (work, tag, reader).
--
-- `voter_pseud_id` rather than an account id, and there is deliberately no way to
-- resolve it: §11.17 treats a reader headcount as not derivable, and a vote table
-- keyed on a pseudonymous identifier is only safe if nothing can reverse it. The
-- store is the only writer and it never stores the mapping.
--
-- One vote per (work, tag, voter) — the primary key, not a UNIQUE constraint, so a
-- second vote is a conflict to be detected rather than a silent second row that
-- double-counts the aggregate.
CREATE TABLE work_tag_votes (
    work_id       TEXT NOT NULL,
    node_id       TEXT NOT NULL REFERENCES taxonomy_nodes(id) ON DELETE CASCADE,
    voter_pseud_id TEXT NOT NULL,
    -- -1, 0 or +1. CHECKed because an unchecked vote weight would let any integer
    -- into the average and there is no other place that would catch it.
    vote          INTEGER NOT NULL,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    PRIMARY KEY (work_id, node_id, voter_pseud_id),
    CHECK (vote IN (-1, 0, 1))
);

-- `work_tags.confidence` is recomputed from this table, and the recomputation reads
-- every vote for one tag across works -- so the index leads with node_id, not with
-- the primary key's order.
CREATE INDEX work_tag_votes_node_idx ON work_tag_votes (node_id);
-- A reader's own votes, for "what did I vote on" and for withdrawing one.
CREATE INDEX work_tag_votes_voter_idx ON work_tag_votes (voter_pseud_id);

-- ── Indexing policy ──────────────────────────────────────────────────────────
--
-- `index_policy` is what a remote instance's visibility rules travel with, per §7's
-- federation bullet: "carry the remote `index_policy` and visibility in the
-- activity". It is per-work because visibility is not uniform across a remote
-- catalogue.
--
-- One row per work. `fetch_remote` and `allow_embedding` are the two decisions the
-- indexing lanes make, and they are separate because a work can be indexed for
-- lexical search and still be excluded from embedding on cost or privacy grounds.
CREATE TABLE work_index_policy (
    work_id       TEXT NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    -- Whether this instance may fetch and hold the work at all.
    fetch_remote  INTEGER NOT NULL DEFAULT 1,
    -- Whether the slow lane may compute embeddings. Never inferred from
    -- fetch_remote: §47.10/§49.9 forbid inferred data influencing anything, and an
    -- embedding derived without consent is exactly that.
    allow_embedding INTEGER NOT NULL DEFAULT 0,
    -- Whether the work's body may be chunked into `work_passages`.
    allow_passages INTEGER NOT NULL DEFAULT 1,
    -- Per-work expiry from a remote's policy, if it has one. NULL means no
    -- expiry, which is distinct from "expires at the epoch".
    index_until   TEXT,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    PRIMARY KEY (work_id),
    -- Booleans as 0/1 rather than a dialect BOOLEAN: this schema stores booleans
    -- as INTEGER on both engines (see `work_characters.is_pov` in 0103), and a
    -- column that is BOOLEAN on one engine and INTEGER on the other breaks the
    -- parity the catalogue checks. Values other than 0/1 are refused.
    CHECK (fetch_remote IN (0, 1)),
    CHECK (allow_embedding IN (0, 1)),
    CHECK (allow_passages IN (0, 1))
);

-- The lane that acts on the policy reads "everything not yet fetched, embedding
-- allowed", so the index leads with allow_embedding.
CREATE INDEX work_index_policy_embedding_idx ON work_index_policy (allow_embedding);
CREATE INDEX work_index_policy_until_idx ON work_index_policy (index_until);

-- ── Passages ─────────────────────────────────────────────────────────────────
--
-- The chunked body the slow lane produces, and the thing a search result quotes
-- back. Offsets are CHARACTER offsets into the chapter's text, not byte offsets:
-- byte offsets into UTF-8 are not what a reader's cursor or a snippet boundary
-- means, and mixing the two produces snippets that cut a character in half.
CREATE TABLE work_passages (
    id           TEXT PRIMARY KEY,
    work_id      TEXT NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    -- NULL for a work-level passage (a synopsis or a work-level summary).
    chapter_id   TEXT,
    seq          INTEGER NOT NULL,
    text         TEXT NOT NULL,
    start_offset INTEGER NOT NULL,
    end_offset   INTEGER NOT NULL,
    -- The embedding's model identity. `model_version` is load-bearing: without it
    -- a re-embed is indistinguishable from a search over mixed vector spaces, which
    -- returns confident nonsense. §9's "version embeddings by model_version" is a
    -- correctness requirement, not bookkeeping.
    model_version TEXT,
    created_at   TEXT NOT NULL,
    -- start <= end and non-negative: a passage that runs backwards is a chunker
    -- bug, and catching it here beats a snippet that quotes the end of a chapter
    -- before its start.
    CHECK (start_offset >= 0),
    CHECK (end_offset >= start_offset),
    -- Two passages of the same chapter cannot occupy the same position.
    UNIQUE (work_id, chapter_id, seq)
);

-- The retrieval query is "passages of this work, in order", so the index carries
-- the ordering rather than leaving a sort in the engine.
CREATE INDEX work_passages_work_idx ON work_passages (work_id, seq);
-- A model-version sweep must find every passage computed by one model.
CREATE INDEX work_passages_model_idx ON work_passages (model_version);
