-- M34: spoilers, content warnings, readability (spec §35.4).
--
-- Twin of the SQLite file, with the same structure.

ALTER TABLE forum_topics ADD COLUMN spoiler_scope_chapter INTEGER DEFAULT NULL;

CREATE TABLE reader_work_progress (
    -- UUID to match work_id, which references a UUID primary key. No foreign
    -- key on account here: the SQLite twin has none, and inventing one would
    -- give the two dialects different deletion behaviour.
    account         UUID NOT NULL,
    work_id         UUID NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    last_chapter    INTEGER NOT NULL DEFAULT 0,
    updated_at      TEXT NOT NULL,
    PRIMARY KEY (account, work_id)
);

CREATE INDEX idx_reader_work_progress_work ON reader_work_progress(work_id);

CREATE TABLE content_warnings (
    id              TEXT PRIMARY KEY,
    post_id         TEXT NOT NULL REFERENCES forum_posts(id) ON DELETE CASCADE,
    warning_type    TEXT NOT NULL,
    severity        INTEGER NOT NULL DEFAULT 1,
    custom_text     TEXT,
    created_at      TEXT NOT NULL
);

CREATE INDEX idx_content_warnings_post ON content_warnings(post_id);

CREATE TABLE post_drafts (
    id              TEXT PRIMARY KEY,
    account         UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    topic_id        TEXT NOT NULL REFERENCES forum_topics(id) ON DELETE CASCADE,
    body            TEXT NOT NULL DEFAULT '',
    updated_at      TEXT NOT NULL
);

CREATE UNIQUE INDEX idx_post_drafts_account_topic ON post_drafts(account, topic_id);

ALTER TABLE forum_posts ADD COLUMN scheduled_at TEXT DEFAULT NULL;
ALTER TABLE forum_posts ADD COLUMN published INTEGER NOT NULL DEFAULT 1;

CREATE INDEX idx_forum_posts_scheduled ON forum_posts(scheduled_at) WHERE scheduled_at IS NOT NULL;

CREATE TABLE reader_warning_prefs (
    account         UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    warning_type    TEXT NOT NULL,
    action          TEXT NOT NULL DEFAULT 'blur',
    PRIMARY KEY (account, warning_type)
);
