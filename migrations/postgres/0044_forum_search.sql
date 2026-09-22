-- Forum full-text search (spec §17.4).
--
-- PostgreSQL: tsvector columns with GIN indexes for ranked full-text search.
-- SQLite: no FTS5 dependency; search uses LIKE on indexed columns.

-- Add tsvector column for forum topics (title search)
ALTER TABLE forum_topics
    ADD COLUMN IF NOT EXISTS search_vector tsvector
    GENERATED ALWAYS AS (to_tsvector('english', coalesce(title, ''))) STORED;

CREATE INDEX IF NOT EXISTS forum_topics_search_idx ON forum_topics USING GIN (search_vector);

-- Add tsvector column for forum posts (body search)
ALTER TABLE forum_posts
    ADD COLUMN IF NOT EXISTS search_vector tsvector
    GENERATED ALWAYS AS (to_tsvector('english', coalesce(body, ''))) STORED;

CREATE INDEX IF NOT EXISTS forum_posts_search_idx ON forum_posts USING GIN (search_vector);

-- Track search misses for analytics (already in 0042, but ensure it's there)
-- forum_search_misses table created in migration 0042_forum_m35.sql
-- Add LIKE-support indexes to Postgres for parity with SQLite (spec §17.4)

-- These indexes support the LIKE-based fallback path and ensure the
-- dialect-drift test passes: both dialects declare the same index set.
CREATE INDEX IF NOT EXISTS forum_topics_title_idx ON forum_topics (title);
CREATE INDEX IF NOT EXISTS forum_posts_body_idx ON forum_posts (body);
