-- M65: forum search LIKE-support indexes (spec §17.4 dialect parity).
--
-- Both dialects declare the same indexes so the dialect-drift test passes.
-- Postgres: these support the LIKE-based fallback path alongside tsvector+GIN.
-- SQLite: same indexes over the TEXT search_vector columns declared in 0044.

CREATE INDEX IF NOT EXISTS forum_topics_title_idx ON forum_topics (title);
CREATE INDEX IF NOT EXISTS forum_posts_body_idx ON forum_posts (body);
