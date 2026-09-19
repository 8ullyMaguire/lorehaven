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
--    create the new with the wider CHECK, copy the rows, drop the old. All
--    foreign keys (download_grants, bulk_export_items) reference `export_jobs`
--    by name and the rename keeps them pointing at the right table; the only
--    references to the old name are the two we create here.
--
--  * **`bulk_export_items` records each work a bulk export visits.** A row
--    per work with its decision + reason, so "never bypasses download grants"
--    is assertable from the table rather than claimed from the code. The
--    `decision` column is the closed set the export worker writes; `reason`
--    names why a work was skipped.

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
                                  CHECK (state IN ('queued', 'running', 'ready', 'failed',
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

-- Per-item records for a bulk export. One row per work the export visited.
CREATE TABLE bulk_export_items (
    id              TEXT    PRIMARY KEY,
    export_job_id   TEXT    NOT NULL REFERENCES export_jobs (id) ON DELETE CASCADE,
    work_id         TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    -- What this item became: included (rendered into the bundle), skipped
    -- (not eligible or would exceed bounds), failed (the worker could not
    -- render it).
    decision        TEXT    NOT NULL CHECK (decision IN ('included', 'skipped', 'failed')),
    -- Why: 'not_eligible', 'over_bounds', 'render_failed', 'no_chapters', …
    -- Skipped items get a reason; included and failed may.
    reason          TEXT,
    -- The rendered artifact for this item, if included. May be null for
    -- skipped/failed items.
    blob_checksum   TEXT,
    byte_size       INTEGER,
    created_at      TEXT    NOT NULL,
    updated_at      TEXT    NOT NULL,
    version         INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX idx_bulk_export_items_job ON bulk_export_items (export_job_id);
CREATE INDEX idx_bulk_export_items_work ON bulk_export_items (work_id);
