-- 0031_media_edition_creators: edition-level creator credits (M26 / spec §32.5).
--
-- Dialect: SQLite. Timestamps are RFC 3339 UTC text.
--
-- A narration edition credits its narrator (human or machine producer).
-- Unlike media_creators (work-level), this is per-edition so a single work
-- can have multiple narration editions with different narrators.

CREATE TABLE IF NOT EXISTS media_edition_creators (
  id           TEXT PRIMARY KEY,
  edition_id   TEXT NOT NULL REFERENCES media_editions (id) ON DELETE CASCADE,
  creator_id   TEXT NOT NULL,
  role         TEXT NOT NULL,  -- narrator | translator | adapter | other
  created_at   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_media_edition_creators_edition ON media_edition_creators(edition_id);
