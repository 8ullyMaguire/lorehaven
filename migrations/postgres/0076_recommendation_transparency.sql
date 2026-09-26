-- Migration 0076 — M29: recommendation transparency and curation labour (spec §33.3)
--
-- Two features that both need the reader-side facts to outlive the request:
--
--   recommendation_slots  what a slot was, and why, recorded at selection time.
--                         §33.3(a) asks a reader to be able to ask "why am I
--                         seeing this" about a slot they already received, so
--                         something has to survive the response.
--   tag_wrangling_proposals  the §33.3(c) queue turning canonicalisation into
--                         visible trust-gated work.
--
-- A third table, reader_attention_settings, holds the §33.3(b) opt-in: the
-- attention report is off until the reader enables it.
--
-- Dialect: PostgreSQL. The SQLite twin is byte-equivalent modulo types.
-- Timestamps are RFC 3339 text; identifiers are UUID.
--
-- The explanation stores the *reader-side* reason only. An operator affinity
-- multiplier is never written here, so no explanation path can surface it even
-- by accident — see the `operator_influence_bp` column's comment.

-- One recorded slot from one recommendation response.
--
-- The rows are deliberately small: the work id, the position it was served at,
-- and the reasons that placed it there. A reader's question is "why this one",
-- which is answered by the top-N of the served set, not by the whole candidate
-- pool.
CREATE TABLE IF NOT EXISTS recommendation_slots (
    id                  UUID PRIMARY KEY,
    pseud_id            UUID NOT NULL REFERENCES pseuds(id) ON DELETE CASCADE,
    work_id             UUID NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    -- Which response this slot belonged to, so a reader asking about slot 3 of
    -- one page gets that page and not a later one.
    request_id          UUID NOT NULL,
    -- 0-based position in the served list.
    position            INTEGER NOT NULL,
    -- The reader-side reasons, as a JSON array of strings. The vocabulary is
    -- closed and asserted by a unit test: the same work in the same slot must
    -- always carry the same reason strings, so a client can compare them.
    reasons             JSONB NOT NULL,
    -- The taste signal's reader-facing bucket, never the raw score. A reader
    -- sees "strong" or "some", not a float that would leak the blend.
    taste_signal        TEXT,
    -- The comparison that seeded it, in §29.2 arena language, when a reader
    -- preference produced the slot.
    seeded_by           TEXT,
    -- The recipe stage that produced it, for recipe dashboards.
    recipe_stage        TEXT,
    -- How much of the final score came from the operator's curation, as a
    -- coarse bucket. The *value* of the operator's multiplier is never stored
    -- here: this column exists so the UI can say "instance curation" and not
    -- say why. Kept as a bucket, not a number, for the same reason.
    instance_curation   TEXT,
    -- The blend's own score, so a reader can see that a slot above it was a
    -- close call. This is reader-visible ranking, not operator taste.
    blend_score         BIGINT NOT NULL DEFAULT 0,
    created_at          TIMESTAMPTZ NOT NULL
);
CREATE INDEX idx_recommendation_slots_request ON recommendation_slots (pseud_id, request_id, position);
CREATE INDEX idx_recommendation_slots_created ON recommendation_slots (created_at);

-- §33.3(b): the attention report is private and off until enabled.
CREATE TABLE IF NOT EXISTS reader_attention_settings (
    pseud_id            UUID PRIMARY KEY REFERENCES pseuds(id) ON DELETE CASCADE,
    enabled             BOOLEAN NOT NULL DEFAULT false,
    created_at          TIMESTAMPTZ NOT NULL,
    updated_at          TIMESTAMPTZ NOT NULL
);

-- §33.3(c): the wrangling queue.
--
-- A proposal is pending, approved or rejected. Approved merges are applied
-- immediately and reversibly: the merge writes the rows below and never
-- destroys the source node, so §33.3's "merges keep their history and stay
-- reversible" is a property of the data, not of a compensating transaction.
CREATE TABLE IF NOT EXISTS tag_wrangling_proposals (
    id                  UUID PRIMARY KEY,
    kind                TEXT NOT NULL CHECK (kind IN ('alias','merge','namespace_move','canonical_rename')),
    from_node_id        TEXT NOT NULL,
    to_node_id          TEXT,
    reason              TEXT NOT NULL,
    -- The pseud that proposed it, and the pseud that approved it. Both are
    -- recorded so §19.1's trust gate is auditable after the fact.
    proposed_by         UUID NOT NULL REFERENCES pseuds(id) ON DELETE CASCADE,
    approved_by         UUID REFERENCES pseuds(id) ON DELETE SET NULL,
    -- Trust level at proposal and at approval, captured because trust can
    -- change later and the gate must be judged on the value in force at the
    -- time.
    proposer_trust      INTEGER NOT NULL,
    approver_trust      INTEGER,
    status              TEXT NOT NULL DEFAULT 'pending'
                            CHECK (status IN ('pending','approved','rejected','reverted')),
    -- The proposal this one reverted, so the history is a chain and not a
    -- flag. §33.3 requires merges stay reversible; a chain is what makes the
    -- reversal auditable.
    reverts_id          UUID REFERENCES tag_wrangling_proposals(id) ON DELETE SET NULL,
    created_at          TIMESTAMPTZ NOT NULL,
    decided_at          TIMESTAMPTZ
);
CREATE INDEX idx_tag_wrangling_proposals_status ON tag_wrangling_proposals (status, created_at);
CREATE INDEX idx_tag_wrangling_proposals_from ON tag_wrangling_proposals (from_node_id);

-- What an applied merge did, so it can be undone exactly rather than guessed.
--
-- `merge_actions` is the whole point of reversibility: reverting a merge means
-- reading these rows and putting back what was there, not inferring it from a
-- canonical that has since been rewritten.
CREATE TABLE IF NOT EXISTS tag_wrangler_merge_actions (
    id                  UUID PRIMARY KEY,
    proposal_id         UUID NOT NULL REFERENCES tag_wrangling_proposals(id) ON DELETE CASCADE,
    action              TEXT NOT NULL CHECK (action IN ('retarget_tags','retarget_aliases','rewrite_canonical')),
    -- The previous value, as text. `node_id` before a retarget, the canonical
    -- before a rewrite.
    previous_value      TEXT,
    -- The work or alias that was moved, for a retarget.
    subject_id          TEXT,
    reverted_at         TIMESTAMPTZ,
    created_at          TIMESTAMPTZ NOT NULL
);
CREATE INDEX idx_tag_wrangler_merge_actions_proposal ON tag_wrangler_merge_actions (proposal_id);
