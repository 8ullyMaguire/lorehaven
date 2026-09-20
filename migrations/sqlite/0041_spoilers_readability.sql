-- M34: spoilers, content warnings, readability (spec §35.4).
--
-- Spoiler-aware zones, structured content warnings, reading-time estimates,
-- draft autosave, post scheduling, and collapsible long posts. The schema
-- supports all of these with four new tables plus a column on forum_posts.

-- Per-topic spoiler scope: "spoilers through chapter N" means any post
-- referencing content past that chapter is a spoiler. NULL means no scope.
ALTER TABLE forum_topics ADD COLUMN spoiler_scope_chapter INTEGER DEFAULT NULL;

-- A reader's progress through a work (used to decide whether to hide a post).
CREATE TABLE reader_work_progress (
    account         TEXT NOT NULL,
    work_id         TEXT NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    last_chapter    INTEGER NOT NULL DEFAULT 0,
    updated_at      TEXT NOT NULL,
    PRIMARY KEY (account, work_id)
);

CREATE INDEX idx_reader_work_progress_work ON reader_work_progress(work_id);

-- Structured content warnings (spec §35.4, table content_warnings).
CREATE TABLE content_warnings (
    id              TEXT PRIMARY KEY,
    post_id         TEXT NOT NULL REFERENCES forum_posts(id) ON DELETE CASCADE,
    warning_type    TEXT NOT NULL,  -- violence | sexual_content | self_harm | spoilers | custom
    severity        INTEGER NOT NULL DEFAULT 1,  -- 1=light, 2=heavy
    custom_text     TEXT,
    created_at      TEXT NOT NULL
);

CREATE INDEX idx_content_warnings_post ON content_warnings(post_id);

-- Draft autosave: a post being composed, persisted every 30s.
CREATE TABLE post_drafts (
    id              TEXT PRIMARY KEY,
    account         TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    topic_id        TEXT NOT NULL REFERENCES forum_topics(id) ON DELETE CASCADE,
    body            TEXT NOT NULL DEFAULT '',
    updated_at      TEXT NOT NULL
);

CREATE UNIQUE INDEX idx_post_drafts_account_topic ON post_drafts(account, topic_id);

-- Post scheduling: posts with scheduled_at in the future are held by a
-- worker job and published when due.
ALTER TABLE forum_posts ADD COLUMN scheduled_at TEXT DEFAULT NULL;
ALTER TABLE forum_posts ADD COLUMN published INTEGER NOT NULL DEFAULT 1;  -- 0 = held

CREATE INDEX idx_forum_posts_scheduled ON forum_posts(scheduled_at) WHERE scheduled_at IS NOT NULL;

-- Spoiler collapse configuration (spec §35.4): per-reader prefs.
CREATE TABLE reader_warning_prefs (
    account         TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    warning_type    TEXT NOT NULL,  -- violence | sexual_content | self_harm | spoilers | custom
    action          TEXT NOT NULL DEFAULT 'blur',  -- blur | show
    PRIMARY KEY (account, warning_type)
);
