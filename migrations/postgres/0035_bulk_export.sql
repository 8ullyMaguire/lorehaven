-- Migration 0035 — bulk export: query subject type and item records (M23-02).
--
-- Dialect: PostgreSQL.
-- Timestamps are RFC 3339 UTC text; identifiers are canonical UUID text.
--
-- Rebuild export_jobs to widen the subject enum and add converter tracking.
-- PostgreSQL cannot ALTER CONSTRAINT in place, so we rebuild the table.

-- Drop the dependent tables first (their FKs will be invalidated by the rename).
DROP TABLE IF EXISTS device_deliveries;
DROP TABLE IF EXISTS bulk_export_items;
DROP TABLE IF EXISTS download_grants;

ALTER TABLE export_jobs RENAME TO export_jobs_old;

CREATE TABLE export_jobs (
    id                    UUID    PRIMARY KEY,
    job_id                UUID    NOT NULL UNIQUE REFERENCES jobs (id) ON DELETE CASCADE,
    account_id            UUID    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    subject_type          TEXT    NOT NULL CHECK (subject_type IN ('work', 'library_item', 'query')),
    subject_id            UUID    NOT NULL,
    format                TEXT    NOT NULL,
    options_json          TEXT,
    privacy_acknowledged_at TEXT,
    state                 TEXT    NOT NULL DEFAULT 'queued'
                                  CHECK (state IN ('queued', 'running', 
                                                   'ready', 'failed', 
                                                   'cancelled')),
    output_blob_checksum  TEXT,
    output_bytes          BIGINT,
    converter_version     TEXT,
    error_message         TEXT,
    created_at            TIMESTAMPTZ NOT NULL,
    updated_at            TIMESTAMPTZ NOT NULL,
    version               INTEGER NOT NULL DEFAULT 1
);

ALTER TABLE export_jobs_old ALTER COLUMN created_at TYPE TIMESTAMPTZ USING created_at::TIMESTAMPTZ;
ALTER TABLE export_jobs_old ALTER COLUMN updated_at TYPE TIMESTAMPTZ USING updated_at::TIMESTAMPTZ;

INSERT INTO export_jobs
    (id, job_id, account_id, subject_type, subject_id, format, options_json,
     privacy_acknowledged_at, state, output_blob_checksum, output_bytes,
     converter_version, error_message, created_at, updated_at, version)
SELECT id, job_id, account_id, subject_type, subject_id, format, options_json,
       privacy_acknowledged_at, state, output_blob_checksum, output_bytes,
       converter_version, error_message, created_at, updated_at, version
  FROM export_jobs_old;

DROP TABLE export_jobs_old;

CREATE TABLE download_grants (
    id            UUID    PRIMARY KEY,
    export_job_id UUID    NOT NULL REFERENCES export_jobs (id) ON DELETE CASCADE,
    -- SHA-256 of the token that appears in the URL. Never the token.
    token_hash    TEXT    NOT NULL UNIQUE,
    expires_at    TIMESTAMPTZ NOT NULL,
    -- Set the first time it is redeemed, which is what makes it single-use.
    used_at       TIMESTAMPTZ,
    single_use    INTEGER NOT NULL DEFAULT 1,
    created_at    TIMESTAMPTZ NOT NULL
);

CREATE TABLE bulk_export_items (
    id              UUID    PRIMARY KEY,
    export_job_id   UUID    NOT NULL REFERENCES export_jobs (id) ON DELETE CASCADE,
    work_id         UUID    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    decision        TEXT    NOT NULL CHECK (decision IN ('included', 'skipped', 'failed')),
    reason          TEXT,
    blob_checksum   TEXT,
    byte_size       BIGINT,
    created_at      TIMESTAMPTZ NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL,
    version         INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX idx_download_grants_job ON download_grants (export_job_id);
CREATE INDEX idx_bulk_export_items_job ON bulk_export_items (export_job_id);
CREATE INDEX idx_bulk_export_items_work ON bulk_export_items (work_id);
