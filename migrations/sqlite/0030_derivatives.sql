-- 0030_derivatives: derivative pipeline (spec §32.4, M25).
--
-- Dialect: SQLite. Timestamps are RFC 3339 UTC text; checksums are SHA-256 hex.
--
-- A derivative is a rendered artifact (EPUB/PDF/text/OCR/transcode) produced
-- from a parent blob. Each row records the parent checksum and the produced
-- blob checksum so the row can be re-verified on a schedule: if the parent
-- changes, the derivative is stale and must be rebuilt.

CREATE TABLE IF NOT EXISTS derivatives (
  id               TEXT PRIMARY KEY,
  work_id          TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
  edition_kind     TEXT NOT NULL,  -- draft | revised | anthology | translation | narration | printing
  derivative_kind  TEXT NOT NULL,  -- epub | pdf | text | ocr | transcode
  parent_checksum  TEXT NOT NULL,  -- SHA-256 of the source blob
  output_checksum  TEXT,           -- SHA-256 of the produced blob (NULL while building)
  output_bytes     INTEGER,
  output_mime_type TEXT,
  state            TEXT NOT NULL DEFAULT 'queued',  -- queued | ready | stale | failed
  job_id           TEXT,           -- the job that produced this derivative
  error_message    TEXT,
  built_at         TEXT,
  verified_at      TEXT,           -- last re-verification against parent
  created_at       TEXT NOT NULL,
  updated_at       TEXT NOT NULL,
  version          INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX IF NOT EXISTS idx_derivatives_work ON derivatives(work_id);
CREATE INDEX IF NOT EXISTS idx_derivatives_state ON derivatives(state);
CREATE INDEX IF NOT EXISTS idx_derivatives_kind ON derivatives(derivative_kind);
CREATE INDEX IF NOT EXISTS idx_derivatives_edition ON derivatives(edition_kind);
