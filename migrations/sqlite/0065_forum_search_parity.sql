-- M65: forum search LIKE-support indexes + SQLite search_vector columns (spec §17.4 dialect parity).
--
-- Both dialects declare the same columns and indexes so the dialect-drift test passes.
-- Postgres adds tsvector columns in 0044; SQLite cannot use GENERATED ALWAYS
-- so declares the columns here alongside the LIKE-support indexes.
-- The search_idx indexes on SQLite are declared in 0044 (on title/body for
-- LIKE search); Postgres declares them in 0044 on search_vector with GIN.

ALTER TABLE forum_topics ADD COLUMN search_vector TEXT;
ALTER TABLE forum_posts ADD COLUMN search_vector TEXT;

CREATE INDEX IF NOT EXISTS forum_topics_title_idx ON forum_topics (title);
CREATE INDEX IF NOT EXISTS forum_posts_body_idx ON forum_posts (body);
