-- Forum topic subscriptions and high-water marks (spec §17.4, §5.5).
--
-- SQLite version: same structure, timestamptz becomes text timestamps.

CREATE TABLE forum_topic_subscriptions (
    account     uuid NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    topic_id    text NOT NULL REFERENCES forum_topics(id) ON DELETE CASCADE,
    last_read_post_id text REFERENCES forum_posts(id) ON DELETE SET NULL,
    created_at  text NOT NULL DEFAULT (strftime('%Y-%m-%dT%H:%M:%fZ')),
    PRIMARY KEY (account, topic_id)
);

CREATE INDEX idx_forum_subs_account ON forum_topic_subscriptions (account, created_at DESC);

-- SQLite cannot ALTER TABLE ADD COLUMN with FK, so last_post_id is added
-- to forum_topics only if not present. The column is managed by triggers
-- or application code in SQLite.
ALTER TABLE forum_topics
    ADD COLUMN last_post_id text REFERENCES forum_posts(id) ON DELETE SET NULL;

CREATE INDEX idx_forum_topics_last_post ON forum_topics (category_id, last_post_at DESC);
