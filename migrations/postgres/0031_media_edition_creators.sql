-- 0031_media_edition_creators: edition-level creator credits (M26 / spec §32.5).
--
-- Dialect: PostgreSQL. Timestamps are RFC 3339 UTC TEXT.

CREATE TABLE IF NOT EXISTS media_edition_creators (
  id           UUID PRIMARY KEY,
  edition_id   UUID NOT NULL REFERENCES media_editions (id) ON DELETE CASCADE,
  creator_id   TEXT NOT NULL,             -- external id: provider name for machine narrators
  role         TEXT NOT NULL,
  created_at   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_media_edition_creators_edition ON media_edition_creators(edition_id);
