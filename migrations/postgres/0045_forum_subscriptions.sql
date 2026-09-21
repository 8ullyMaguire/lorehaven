-- Forum topic subscriptions and high-water marks (spec §17.4, §5.5).
--
-- Each subscription tracks the last post a user has read in a topic. New posts
-- beyond that mark are "unread". This is the core signal that makes forums
-- feel alive — without it, users must manually check every thread.

CREATE TABLE forum_topic_subscriptions (
    account     uuid NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    topic_id    text NOT NULL REFERENCES forum_topics(id) ON DELETE CASCADE,
    -- The id of the last post the account has read. NULL = nothing read yet.
    last_read_post_id text REFERENCES forum_posts(id) ON DELETE SET NULL,
    created_at  timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (account, topic_id)
);

CREATE INDEX idx_forum_subs_account ON forum_topic_subscriptions (account, created_at DESC);

-- A lightweight per-topic "last post" cache so unread counts don't require
-- a full join against forum_posts on every page load.
ALTER TABLE forum_topics
    ADD COLUMN IF NOT EXISTS last_post_id text REFERENCES forum_posts(id) ON DELETE SET NULL;

CREATE INDEX idx_forum_topics_last_post ON forum_topics (category_id, last_post_at DESC);
