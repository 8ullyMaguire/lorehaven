-- 0131 — the catalogue's quality signal, promoted out of provenance_json.
--
-- The PostgreSQL twin carries the reasoning; this file is kept in step statement
-- for statement so the parity test can compare the two declared column sets.
--
-- `likes` is NOT NULL DEFAULT 0 (every row has an endorsement count, and zero is
-- a fact), `views` is nullable (several sources publish none, and a zero would
-- claim they do).
ALTER TABLE library_items ADD COLUMN likes INTEGER NOT NULL DEFAULT 0;
ALTER TABLE library_items ADD COLUMN views INTEGER;

CREATE INDEX IF NOT EXISTS idx_library_items_source_likes
    ON library_items (source_key, likes DESC);

CREATE INDEX IF NOT EXISTS idx_library_items_title_author_norm
    ON library_items (lower(trim(title)), lower(trim(author_text)));
