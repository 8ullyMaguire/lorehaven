-- M26 / spec §32.5 — narration edition audio checksum (for TTS narration
-- jobs that store synthesized audio). The media_editions table was created
-- in migration 0024 (media_generalization.sql); we add a column rather than
-- a new table because audio is one property of an edition, not a separate
-- entity. Applied migrations are immutable — this is a new migration.

ALTER TABLE media_editions ADD COLUMN audio_checksum TEXT;
CREATE INDEX IF NOT EXISTS idx_media_editions_audio_checksum ON media_editions(audio_checksum);
