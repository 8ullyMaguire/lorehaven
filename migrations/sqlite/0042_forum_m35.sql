-- M35: moderation ladder, slow mode, featured posts, federation scope, sparklines.
--
-- Graduated sanctions, per-topic rate limits, curation, federation boundaries,
-- and stored activity data for topic cards.

-- Per-topic slow mode: seconds a user must wait between posts. 0 = off.
ALTER TABLE forum_topics ADD COLUMN slow_mode_seconds INTEGER NOT NULL DEFAULT 0;

-- Federation scope: public | local | unlisted. Authors choose at creation.
ALTER TABLE forum_topics ADD COLUMN federation_scope TEXT NOT NULL DEFAULT 'public';

-- Featured posts (best-of curation).
ALTER TABLE forum_posts ADD COLUMN featured INTEGER NOT NULL DEFAULT 0;
ALTER TABLE forum_posts ADD COLUMN featured_by TEXT REFERENCES accounts(id) ON DELETE SET NULL;
ALTER TABLE forum_posts ADD COLUMN featured_at TEXT;

CREATE INDEX idx_forum_posts_featured ON forum_posts(featured) WHERE featured = 1;

-- Audit trail for thread forking (spec §35.5): preserves original authorship.
ALTER TABLE forum_posts ADD COLUMN original_topic_id TEXT REFERENCES forum_topics(id) ON DELETE SET NULL;

CREATE INDEX idx_forum_posts_original_topic ON forum_posts(original_topic_id) WHERE original_topic_id IS NOT NULL;

-- Graduated response ladder (spec §35.5): sanctions on forum participation.
CREATE TABLE forum_sanctions (
    id              TEXT PRIMARY KEY,
    account         TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    category_id     TEXT REFERENCES forum_categories(id) ON DELETE CASCADE,  -- NULL = site-wide
    level           TEXT NOT NULL,  -- verbal_warning | post_throttle | read_only | forum_ban | site_ban
    reason          TEXT NOT NULL,
    actor           TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    created_at      TEXT NOT NULL,
    expires_at      TEXT,           -- NULL = permanent
    active          INTEGER NOT NULL DEFAULT 1,
    appealed        INTEGER NOT NULL DEFAULT 0,
    appeal_outcome  TEXT            -- upheld | reversed
);

CREATE INDEX idx_forum_sanctions_account ON forum_sanctions(account, active);
CREATE INDEX idx_forum_sanctions_category ON forum_sanctions(category_id, active);

-- Post throttle tracking (for post_throttle sanction level).
CREATE TABLE forum_throttle_log (
    account         TEXT NOT NULL,
    topic_id        TEXT NOT NULL REFERENCES forum_topics(id) ON DELETE CASCADE,
    posted_at       TEXT NOT NULL,
    PRIMARY KEY (account, topic_id, posted_at)
);

-- Activity sparklines (spec §35.5): daily reply counts per topic.
CREATE TABLE forum_topic_activity (
    topic_id        TEXT NOT NULL REFERENCES forum_topics(id) ON DELETE CASCADE,
    activity_date   TEXT NOT NULL,  -- YYYY-MM-DD
    reply_count     INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (topic_id, activity_date)
);

CREATE INDEX idx_forum_topic_activity_date ON forum_topic_activity(topic_id, activity_date);

-- Zero-result search tracking (spec §35.5): queries that return nothing.
CREATE TABLE forum_search_misses (
    id              TEXT PRIMARY KEY,
    query_hash      TEXT NOT NULL,  -- SHA-256 of normalized query
    query_text      TEXT NOT NULL,
    first_seen_at   TEXT NOT NULL,
    last_seen_at    TEXT NOT NULL,
    count           INTEGER NOT NULL DEFAULT 1
);

CREATE UNIQUE INDEX idx_forum_search_misses_hash ON forum_search_misses(query_hash);

-- First-votes notification tracking (spec §35.5): one gentle nudge per post.
CREATE TABLE forum_post_first_vote_notified (
    post_id         TEXT NOT NULL REFERENCES forum_posts(id) ON DELETE CASCADE,
    account         TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,  -- post author
    notified_at     TEXT NOT NULL,
    vote_count      INTEGER NOT NULL,  -- how many votes existed at notification
    PRIMARY KEY (post_id, account)
);
