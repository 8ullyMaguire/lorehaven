-- Migration 0035 — bulk export: query subject type and item records (M23-02).
--
-- Dialect: SQLite.
-- Timestamps are RFC 3339 UTC text; identifiers are canonical UUID text.
--
-- Design notes:
--
--  * **`export_jobs.subject_type` gains 'query'.** Spec §13 and §21: every
--    query can be exported as a grant-gated bundle. SQLite cannot extend a
--    CHECK constraint in place, so the table is rebuilt: rename the old,
--    create the new with the wider CHECK, copy the rows, drop the old.
--    Foreign keys referencing export_jobs are dropped before the rename and
--    recreated after, because SQLite's ALTER TABLE RENAME rewrites
--    referencing FK tables' internal references to the new name (leaving
--    them pointing at export_jobs_old), and the subsequent DROP then breaks
--    them.
--
--  * **`bulk_export_items` records each work a bulk export visits.** A row
--    per work with its decision + reason, so "never bypasses download grants"
--    is assertable from the table rather than claimed from the code. The
--    `decision` column is the closed set the export worker writes; `reason`
--    names why a work was skipped.

-- Drop foreign keys that reference export_jobs before the rename.
PRAGMA foreign_keys=OFF;

-- Rebuild export_jobs with the wider subject_type CHECK.
ALTER TABLE export_jobs RENAME TO export_jobs_old;

CREATE TABLE export_jobs (
    id                    TEXT    PRIMARY KEY,
    job_id                TEXT    NOT NULL UNIQUE REFERENCES jobs (id) ON DELETE CASCADE,
    account_id            TEXT    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    subject_type          TEXT    NOT NULL CHECK (subject_type IN ('work', 'library_item', 'query')),
    subject_id            TEXT    NOT NULL,
    format                TEXT    NOT NULL,
    options_json          TEXT,
    privacy_acknowledged_at TEXT,
    state                 TEXT    NOT NULL DEFAULT 'queued'
                                  CHECK (state IN ('queued', 'running', 
                                                   'ready', 'failed', 
                                                   'cancelled')),
    output_blob_checksum  TEXT,
    output_bytes          INTEGER,
    converter_version     TEXT,
    error_message         TEXT,
    created_at            TEXT    NOT NULL,
    updated_at            TEXT    NOT NULL,
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
    id            TEXT    PRIMARY KEY,
    export_job_id TEXT    NOT NULL REFERENCES export_jobs (id) ON DELETE CASCADE,
    -- SHA-256 of the token that appears in the URL. Never the token.
    token_hash    TEXT    NOT NULL UNIQUE,
    expires_at    TEXT    NOT NULL,
    -- Set the first time it is redeemed, which is what makes it single-use.
    used_at       TEXT,
    single_use    INTEGER NOT NULL DEFAULT 1,
    created_at    TEXT    NOT NULL
);

DROP TABLE IF EXISTS bulk_export_items;
CREATE TABLE bulk_export_items (
    id              TEXT    PRIMARY KEY,
    export_job_id   TEXT    NOT NULL REFERENCES export_jobs (id) ON DELETE CASCADE,
    work_id         TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    decision        TEXT    NOT NULL CHECK (decision IN ('included', 'skipped', 'failed')),
    reason          TEXT,
    blob_checksum   TEXT,
    byte_size       INTEGER,
    created_at      TEXT    NOT NULL,
    updated_at      TEXT    NOT NULL,
    version         INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX idx_download_grants_job ON download_grants (export_job_id);
CREATE INDEX idx_bulk_export_items_job ON bulk_export_items (export_job_id);
CREATE INDEX idx_bulk_export_items_work ON bulk_export_items (work_id);

PRAGMA foreign_keys=ON;