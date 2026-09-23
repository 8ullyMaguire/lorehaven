-- Forum full-text search (spec §17.4).
--
-- SQLite: no FTS5 dependency; search uses LIKE on indexed columns.

-- SQLite doesn't support ALTER TABLE ADD COLUMN with GENERATED ALWAYS in all versions.
-- Create index on existing columns for LIKE search. Index names mirror the
-- PostgreSQL dialect (which indexes the generated search_vector with GIN);
-- the columns legitimately differ by engine — see the allowlist in
-- crates/db/src/migrate.rs the_two_dialects_declare_the_same_columns_and_indexes.
CREATE INDEX IF NOT EXISTS forum_topics_search_idx ON forum_topics (title);
CREATE INDEX IF NOT EXISTS forum_posts_search_idx ON forum_posts (body);
