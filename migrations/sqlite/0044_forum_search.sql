-- Forum full-text search (spec §17.4).
--
-- SQLite: no FTS5 dependency; search uses LIKE on indexed columns.
--
-- SQLite doesn't support ALTER TABLE ADD COLUMN with GENERATED ALWAYS in all versions.
-- Create index on existing columns for LIKE search.
CREATE INDEX IF NOT EXISTS forum_topics_title_idx ON forum_topics (title);
CREATE INDEX IF NOT EXISTS forum_posts_body_idx ON forum_posts (body);
-- Schema parity: Postgres adds tsvector columns in this migration; SQLite
-- cannot use GENERATED ALWAYS but declares the columns for dialect parity.
ALTER TABLE forum_topics ADD COLUMN search_vector TEXT;
ALTER TABLE forum_posts ADD COLUMN search_vector TEXT;
CREATE INDEX IF NOT EXISTS forum_topics_search_idx ON forum_topics (search_vector);
CREATE INDEX IF NOT EXISTS forum_posts_search_idx ON forum_posts (search_vector);
