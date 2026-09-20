-- M31: work discussion modes, work-linked forum topics, work reactions.
-- Spec §35.0–35.1. Existing works keep comments_only; the instance default
-- applies to new works only (never retroactively).

ALTER TABLE works ADD COLUMN discussion_mode TEXT NOT NULL DEFAULT 'comments_only';

CREATE TABLE topic_work_links (
    id TEXT PRIMARY KEY,
    topic_id TEXT NOT NULL UNIQUE REFERENCES forum_topics(id) ON DELETE CASCADE,
    work_id TEXT NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    chapter_id TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX topic_work_links_work ON topic_work_links (work_id);

CREATE TABLE work_reactions (
    work_id TEXT NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    pseud TEXT NOT NULL,
    vote_type TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (work_id, pseud)
);
CREATE INDEX work_reactions_type ON work_reactions (work_id, vote_type);
