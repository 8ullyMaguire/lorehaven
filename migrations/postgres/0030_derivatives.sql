-- 0030_derivatives: derivative pipeline (spec §32.4, M25).
--
-- Dialect: PostgreSQL. Timestamps are TIMESTAMPTZ; checksums are TEXT (SHA-256 hex).

CREATE TABLE IF NOT EXISTS derivatives (
  id               UUID PRIMARY KEY,
  work_id          UUID NOT NULL REFERENCES works (id) ON DELETE CASCADE,
  edition_kind     TEXT NOT NULL,
  derivative_kind  TEXT NOT NULL,
  parent_checksum  TEXT NOT NULL,
  output_checksum  TEXT,
  output_bytes     BIGINT,
  output_mime_type TEXT,
  state            TEXT NOT NULL DEFAULT 'queued',
  job_id           UUID,
  error_message    TEXT,
  built_at         TIMESTAMPTZ,
  verified_at      TIMESTAMPTZ,
  created_at       TIMESTAMPTZ NOT NULL,
  updated_at       TIMESTAMPTZ NOT NULL,
  version          INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX IF NOT EXISTS idx_derivatives_work ON derivatives(work_id);
CREATE INDEX IF NOT EXISTS idx_derivatives_state ON derivatives(state);
CREATE INDEX IF NOT EXISTS idx_derivatives_kind ON derivatives(derivative_kind);
CREATE INDEX IF NOT EXISTS idx_derivatives_edition ON derivatives(edition_kind);
