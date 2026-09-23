-- Migration 0069 — Directory Category Governance (spec §45)
--
-- Dialect: SQLite.
-- Timestamps are RFC 3339 UTC text; identifiers are canonical UUID text.
--
-- Categories become DB rows with a state lifecycle; proposals and votes
-- drive rename/merge/deprecate/create through quorum. Entry moderation
-- (move/remove between categories) also supports quorum review.

CREATE TABLE categories (
    id              TEXT    PRIMARY KEY,
    slug            TEXT    NOT NULL UNIQUE,
    label           TEXT    NOT NULL,
    state           TEXT    NOT NULL DEFAULT 'active'
                    CHECK (state IN ('active','deprecated','merged')),
    merged_into     TEXT    REFERENCES categories (id),
    source          TEXT    NOT NULL DEFAULT 'seed'
                    CHECK (source IN ('seed','config','community')),
    created_by      TEXT    NOT NULL,
    created_at      TEXT    NOT NULL
);
CREATE INDEX idx_categories_state ON categories (state);
CREATE INDEX idx_categories_slug ON categories (slug);

CREATE TABLE category_proposals (
    id              TEXT    PRIMARY KEY,
    category_slug   TEXT    NOT NULL,
    action          TEXT    NOT NULL
                    CHECK (action IN ('rename','merge','deprecate','create','delete')),
    payload         TEXT    NOT NULL DEFAULT '{}',
    status          TEXT    NOT NULL DEFAULT 'open'
                    CHECK (status IN ('open','passed','failed','vetoed','expired')),
    yes_votes       INTEGER NOT NULL DEFAULT 0,
    no_votes        INTEGER NOT NULL DEFAULT 0,
    quorum_needed   INTEGER NOT NULL,
    closes_at       TEXT    NOT NULL,
    created_by      TEXT    NOT NULL,
    decided_by      TEXT,
    decision_reason TEXT,
    created_at      TEXT    NOT NULL,
    decided_at      TEXT
);
CREATE INDEX idx_cat_proposals_status ON category_proposals (status, closes_at);
CREATE INDEX idx_cat_proposals_category ON category_proposals (category_slug, status);

CREATE TABLE category_votes (
    id              TEXT    PRIMARY KEY,
    proposal_id     TEXT    NOT NULL REFERENCES category_proposals (id) ON DELETE CASCADE,
    account_id      TEXT    NOT NULL,
    value           TEXT    NOT NULL CHECK (value IN ('yes','no')),
    created_at      TEXT    NOT NULL,
    UNIQUE (proposal_id, account_id)
);
CREATE INDEX idx_cat_votes_proposal ON category_votes (proposal_id);

CREATE TABLE category_changelog (
    id              TEXT    PRIMARY KEY,
    category_slug   TEXT    NOT NULL,
    event           TEXT    NOT NULL,
    actor           TEXT    NOT NULL,
    document        TEXT    NOT NULL DEFAULT '{}',
    created_at      TEXT    NOT NULL
);
CREATE INDEX idx_cat_changelog_slug ON category_changelog (category_slug, created_at DESC);

CREATE TABLE entry_moderation_proposals (
    id              TEXT    PRIMARY KEY,
    entry_id        TEXT    NOT NULL REFERENCES directory_entries (id) ON DELETE CASCADE,
    action          TEXT    NOT NULL CHECK (action IN ('move','remove')),
    target_category TEXT,
    status          TEXT    NOT NULL DEFAULT 'open'
                    CHECK (status IN ('open','passed','failed','vetoed','expired')),
    yes_votes       INTEGER NOT NULL DEFAULT 0,
    no_votes        INTEGER NOT NULL DEFAULT 0,
    quorum_needed   INTEGER NOT NULL DEFAULT 2,
    closes_at       TEXT    NOT NULL,
    created_by      TEXT    NOT NULL,
    decided_by      TEXT,
    created_at      TEXT    NOT NULL,
    decided_at      TEXT
);
CREATE INDEX idx_entry_mod_status ON entry_moderation_proposals (status, closes_at);
CREATE INDEX idx_entry_mod_entry ON entry_moderation_proposals (entry_id, status);

CREATE TABLE entry_moderation_votes (
    id              TEXT    PRIMARY KEY,
    proposal_id     TEXT    NOT NULL REFERENCES entry_moderation_proposals (id) ON DELETE CASCADE,
    account_id      TEXT    NOT NULL,
    value           TEXT    NOT NULL CHECK (value IN ('yes','no')),
    created_at      TEXT    NOT NULL,
    UNIQUE (proposal_id, account_id)
);
CREATE INDEX idx_entry_mod_votes_proposal ON entry_moderation_votes (proposal_id);
