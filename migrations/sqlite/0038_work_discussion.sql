-- M31: work discussion modes, work-linked forum topics, work reactions.
-- Spec §35.0–35.1. Existing works keep comments_only; the instance default
-- applies to new works only (never retroactively).

ALTER TABLE works ADD COLUMN discussion_mode TEXT NOT NULL DEFAULT 'comments_only';

CREATE TABLE topic_work_links (
    id TEXT PRIMARY KEY,
    -- The two foreign keys the PostgreSQL arm of this same migration declares.
    -- They were missing here, which made an orphan link insertable on SQLite and
    -- impossible on PostgreSQL, and left dangling links behind when a work was
    -- hard-deleted. The SQLite pool runs with `PRAGMA foreign_keys = ON`, so the
    -- constraints were always enforceable. Found by the backlink suite, which
    -- had no tests until then.
    --
    -- Note `work_id` is TEXT here and UUID in the PostgreSQL arm: that is the
    -- standing ADR 0004 id-type divergence, not an oversight here.
    topic_id TEXT NOT NULL UNIQUE REFERENCES forum_topics(id) ON DELETE CASCADE,
    work_id TEXT NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    chapter_id TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX topic_work_links_work ON topic_work_links (work_id);

CREATE TABLE work_reactions (
    work_id TEXT NOT NULL,
    pseud TEXT NOT NULL,
    vote_type TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (work_id, pseud)
);
CREATE INDEX work_reactions_type ON work_reactions (work_id, vote_type);
