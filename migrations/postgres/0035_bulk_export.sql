-- Migration 0035 — bulk export: query subject type and item records (M23-02).
--
-- Dialect: PostgreSQL.
-- Timestamps are RFC 3339 UTC text; identifiers are canonical UUID text.
--
-- SQLite cannot extend a CHECK in place and rebuilds the table. PostgreSQL
-- can ALTER CONSTRAINT, but we rebuild the same way so the two dialects
-- produce the same final schema — a rebuild is the one path that works for
-- both and keeps the migration test (which compares table shapes) honest.
--
-- Foreign keys are dropped before the rename and recreated after, because
-- PostgreSQL's ALTER TABLE RENAME leaves referencing tables pointing at
-- export_jobs_old, and the subsequent DROP then breaks them.

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

INSERT INTO export_jobs
    (id, job_id, account_id, subject_type, subject_id, format, options_json,
     privacy_acknowledged_at, state, output_blob_checksum, output_bytes,
     converter_version, error_message, created_at, updated_at, version)
SELECT id, job_id, account_id, subject_type, subject_id, format, options_json,
       privacy_acknowledged_at, state, output_blob_checksum, output_bytes,
       converter_version, error_message, created_at, updated_at, version
  FROM export_jobs_old;

DROP TABLE export_jobs_old;

-- Recreate the referencing tables (their FKs were invalidated by the rename).
DROP TABLE IF EXISTS download_grants;
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

DROP TABLE IF EXISTS bulk_export_items;
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