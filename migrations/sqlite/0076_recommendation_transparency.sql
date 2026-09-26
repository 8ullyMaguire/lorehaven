-- Migration 0076 — M29: recommendation transparency and curation labour (spec §33.3)
--
-- SQLite twin of migrations/postgres/0076_recommendation_transparency.sql.
-- Same tables, same columns, same order, same index names — the parity test
-- compares the declared NAMES line by line, so the index statements here are
-- deliberately kept on one line each.
--
-- Dialect: SQLite. Timestamps are RFC 3339 text; identifiers are canonical
-- UUID text. JSONB becomes TEXT, BOOLEAN becomes INTEGER 0/1, BIGINT becomes
-- INTEGER.

CREATE TABLE IF NOT EXISTS recommendation_slots (
    id                  TEXT    PRIMARY KEY,
    pseud_id            TEXT    NOT NULL REFERENCES pseuds(id) ON DELETE CASCADE,
    work_id             TEXT    NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    request_id          TEXT    NOT NULL,
    position            INTEGER NOT NULL,
    reasons             TEXT    NOT NULL,
    taste_signal        TEXT,
    seeded_by           TEXT,
    recipe_stage        TEXT,
    instance_curation   TEXT,
    blend_score         INTEGER NOT NULL DEFAULT 0,
    created_at          TEXT    NOT NULL
);
CREATE INDEX idx_recommendation_slots_request ON recommendation_slots (pseud_id, request_id, position);
CREATE INDEX idx_recommendation_slots_created ON recommendation_slots (created_at);

CREATE TABLE IF NOT EXISTS reader_attention_settings (
    pseud_id            TEXT    PRIMARY KEY REFERENCES pseuds(id) ON DELETE CASCADE,
    enabled             BOOLEAN NOT NULL DEFAULT 0,
    created_at          TEXT    NOT NULL,
    updated_at          TEXT    NOT NULL
);

CREATE TABLE IF NOT EXISTS tag_wrangling_proposals (
    id                  TEXT    PRIMARY KEY,
    kind                TEXT    NOT NULL CHECK (kind IN ('alias','merge','namespace_move','canonical_rename')),
    from_node_id        TEXT    NOT NULL,
    to_node_id          TEXT,
    reason              TEXT    NOT NULL,
    proposed_by         TEXT    NOT NULL REFERENCES pseuds(id) ON DELETE CASCADE,
    approved_by         TEXT    REFERENCES pseuds(id) ON DELETE SET NULL,
    proposer_trust      INTEGER NOT NULL,
    approver_trust      INTEGER,
    status              TEXT    NOT NULL DEFAULT 'pending' CHECK (status IN ('pending','approved','rejected','reverted')),
    reverts_id          TEXT    REFERENCES tag_wrangling_proposals(id) ON DELETE SET NULL,
    created_at          TEXT    NOT NULL,
    decided_at          TEXT
);
CREATE INDEX idx_tag_wrangling_proposals_status ON tag_wrangling_proposals (status, created_at);
CREATE INDEX idx_tag_wrangling_proposals_from ON tag_wrangling_proposals (from_node_id);

CREATE TABLE IF NOT EXISTS tag_wrangler_merge_actions (
    id                  TEXT    PRIMARY KEY,
    proposal_id         TEXT    NOT NULL REFERENCES tag_wrangling_proposals(id) ON DELETE CASCADE,
    action              TEXT    NOT NULL CHECK (action IN ('retarget_tags','retarget_aliases','rewrite_canonical','set_weight','dedupe_tags')),
    previous_value      TEXT,
    subject_id          TEXT,
    reverted_at         TEXT,
    created_at          TEXT    NOT NULL
);
CREATE INDEX idx_tag_wrangler_merge_actions_proposal ON tag_wrangler_merge_actions (proposal_id);
