-- 0031_media_edition_creators: edition-level creator credits (M26 / spec §32.5).
--
-- Dialect: PostgreSQL. Timestamps are TIMESTAMPTZ.

CREATE TABLE IF NOT EXISTS media_edition_creators (
  id           UUID PRIMARY KEY,
  edition_id   UUID NOT NULL REFERENCES media_editions (id) ON DELETE CASCADE,
  creator_id   UUID NOT NULL,
  role         TEXT NOT NULL,
  created_at   TIMESTAMPTZ NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_media_edition_creators_edition ON media_edition_creators(edition_id);
