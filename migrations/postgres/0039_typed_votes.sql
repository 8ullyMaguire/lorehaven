-- M32: typed votes, vote budgets, meta-moderation and karma (spec §35.2).
--
-- Weights and karma are integers in basis points, never REAL: ADR 0004 has the
-- repository bind only String and i64, and 0010 already stores confidence as
-- basis points. A float column would need a second binding path and would round
-- differently on the two engines.
--
-- `forum_votes` carries a surrogate `id` although (post_id, pseud) is the real
-- key: spec §35.2's `POST /forum/votes/{id}/meta` has to name a vote, and the
-- composite key is not addressable over HTTP. The one-vote-per-pseud rule is
-- enforced by a unique index so both dialects enforce it identically.

CREATE TABLE forum_vote_types (
    id             TEXT PRIMARY KEY,
    label          TEXT NOT NULL,
    category_scope TEXT,
    position       BIGINT NOT NULL DEFAULT 0,
    weight_bp      BIGINT NOT NULL DEFAULT 1000,
    cost           BIGINT NOT NULL DEFAULT 1,
    is_negative    BIGINT NOT NULL DEFAULT 0
);
CREATE INDEX forum_vote_types_scope ON forum_vote_types (category_scope, position);

CREATE TABLE forum_votes (
    id                TEXT PRIMARY KEY,
    post_id           TEXT NOT NULL REFERENCES forum_posts(id) ON DELETE CASCADE,
    pseud             TEXT NOT NULL,
    vote_type         TEXT NOT NULL,
    weight_at_cast_bp BIGINT NOT NULL DEFAULT 1000,
    created_at        TEXT NOT NULL
);
CREATE UNIQUE INDEX forum_votes_post_pseud ON forum_votes (post_id, pseud);
CREATE INDEX forum_votes_pseud_created ON forum_votes (pseud, created_at);
CREATE INDEX forum_votes_post_type ON forum_votes (post_id, vote_type);

CREATE TABLE forum_meta_votes (
    vote_id    TEXT NOT NULL,
    pseud      TEXT NOT NULL,
    fair       BIGINT NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (vote_id, pseud)
);
CREATE INDEX forum_meta_votes_pseud ON forum_meta_votes (pseud, created_at);

CREATE TABLE forum_karma (
    pseud      TEXT PRIMARY KEY,
    karma_bp   BIGINT NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
);

-- The post author's opt-in to reveal who voted (spec §35.2 transparency
-- tiers). Off by default: a visible vote is a vote that can be socially
-- pressured.
ALTER TABLE forum_posts ADD COLUMN votes_visible INTEGER NOT NULL DEFAULT 0;

-- The default taxonomy, as data (spec §35.2): four positive types and one
-- negative type. `disagree` costs more budget than a positive vote does.
-- A category that wants a different set gets its own rows; no code changes.
INSERT INTO forum_vote_types (id, label, category_scope, position, weight_bp, cost, is_negative) VALUES
    ('insightful', 'Insightful', NULL, 0, 1000, 1, 0),
    ('funny', 'Funny', NULL, 1, 1000, 1, 0),
    ('interesting', 'Interesting', NULL, 2, 1000, 1, 0),
    ('well_written', 'Well written', NULL, 3, 1000, 1, 0),
    ('disagree', 'Disagree', NULL, 4, 1000, 2, 1);
