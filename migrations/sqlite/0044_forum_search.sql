-- Forum full-text search (spec §17.4).
--
-- SQLite: no FTS5/tsearch. The parity contract requires both dialects to
-- declare the same shape, so SQLite carries `search_vector` as a plain TEXT
-- column that the repository layer fills with lowercased title/body at
-- write time; LIKE queries run against it. PostgreSQL's 0044 declares the
-- same column as a GENERATED tsvector with a GIN index.

ALTER TABLE forum_topics ADD COLUMN search_vector TEXT NOT NULL DEFAULT '';
ALTER TABLE forum_posts ADD COLUMN search_vector TEXT NOT NULL DEFAULT '';

CREATE INDEX IF NOT EXISTS forum_topics_search_idx ON forum_topics (search_vector);
CREATE INDEX IF NOT EXISTS forum_posts_search_idx ON forum_posts (search_vector);
