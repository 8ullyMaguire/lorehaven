-- M35: moderation ladder, slow mode, featured posts, federation scope, sparklines.
--
-- Twin of the SQLite file.

ALTER TABLE forum_topics ADD COLUMN slow_mode_seconds INTEGER NOT NULL DEFAULT 0;
ALTER TABLE forum_topics ADD COLUMN federation_scope TEXT NOT NULL DEFAULT 'public';

ALTER TABLE forum_posts ADD COLUMN featured INTEGER NOT NULL DEFAULT 0;
-- UUID: accounts(id) is a UUID primary key, and a TEXT foreign key cannot be
-- created against one.
ALTER TABLE forum_posts ADD COLUMN featured_by UUID REFERENCES accounts(id) ON DELETE SET NULL;
ALTER TABLE forum_posts ADD COLUMN featured_at TEXT;

CREATE INDEX idx_forum_posts_featured ON forum_posts(featured) WHERE featured = 1;

ALTER TABLE forum_posts ADD COLUMN original_topic_id TEXT REFERENCES forum_topics(id) ON DELETE SET NULL;

CREATE INDEX idx_forum_posts_original_topic ON forum_posts(original_topic_id) WHERE original_topic_id IS NOT NULL;

CREATE TABLE forum_sanctions (
    id              TEXT PRIMARY KEY,
    account         UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    category_id     TEXT REFERENCES forum_categories(id) ON DELETE CASCADE,
    level           TEXT NOT NULL,
    reason          TEXT NOT NULL,
    actor           UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    created_at      TEXT NOT NULL,
    expires_at      TEXT,
    active          INTEGER NOT NULL DEFAULT 1,
    appealed        INTEGER NOT NULL DEFAULT 0,
    appeal_outcome  TEXT
);

CREATE INDEX idx_forum_sanctions_account ON forum_sanctions(account, active);
CREATE INDEX idx_forum_sanctions_category ON forum_sanctions(category_id, active);

CREATE TABLE forum_throttle_log (
    account         TEXT NOT NULL,
    topic_id        TEXT NOT NULL REFERENCES forum_topics(id) ON DELETE CASCADE,
    posted_at       TEXT NOT NULL,
    PRIMARY KEY (account, topic_id, posted_at)
);

CREATE TABLE forum_topic_activity (
    topic_id        TEXT NOT NULL REFERENCES forum_topics(id) ON DELETE CASCADE,
    activity_date   TEXT NOT NULL,
    reply_count     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (topic_id, activity_date)
);

CREATE INDEX idx_forum_topic_activity_date ON forum_topic_activity(topic_id, activity_date);

CREATE TABLE forum_search_misses (
    id              TEXT PRIMARY KEY,
    query_hash      TEXT NOT NULL,
    query_text      TEXT NOT NULL,
    first_seen_at   TEXT NOT NULL,
    last_seen_at    TEXT NOT NULL,
    count           INTEGER NOT NULL DEFAULT 1
);

CREATE UNIQUE INDEX idx_forum_search_misses_hash ON forum_search_misses(query_hash);

CREATE TABLE forum_post_first_vote_notified (
    post_id         TEXT NOT NULL REFERENCES forum_posts(id) ON DELETE CASCADE,
    account         UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    notified_at     TEXT NOT NULL,
    vote_count      INTEGER NOT NULL,
    PRIMARY KEY (post_id, account)
);
