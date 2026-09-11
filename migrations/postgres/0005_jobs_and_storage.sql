-- Migration 0005 — jobs, the content-addressed store, the revision cache and
-- encrypted secrets (spec §10).
--
-- Dialect: PostgreSQL.
-- Identifiers are native UUID columns; timestamps are RFC 3339 UTC text so the
-- repository layer decodes identically on both engines (see ADR 0004).
--
-- The reasoning behind the shape itself, and the deletion/retention rules, are
-- documented in `migrations/sqlite/0005_jobs_and_storage.sql`; the two files
-- must stay in step. Dialect differences are called out inline.

CREATE TABLE jobs (
    id                UUID    PRIMARY KEY,
    kind              TEXT    NOT NULL,
    state             TEXT    NOT NULL DEFAULT 'queued',
    payload           TEXT    NOT NULL DEFAULT '{}',
    idempotency_key   TEXT,
    priority          BIGINT NOT NULL DEFAULT 0,
    attempts          BIGINT NOT NULL DEFAULT 0,
    max_attempts      BIGINT NOT NULL DEFAULT 5,
    available_at      TEXT    NOT NULL,
    lease_owner       TEXT,
    lease_expires_at  TEXT,
    progress_permille BIGINT NOT NULL DEFAULT 0
                              CHECK (progress_permille BETWEEN 0 AND 1000),
    checkpoint        TEXT,
    last_error        TEXT,
    requested_by      UUID    REFERENCES accounts (id) ON DELETE SET NULL,
    created_at        TEXT    NOT NULL,
    updated_at        TEXT    NOT NULL,
    version           BIGINT NOT NULL DEFAULT 1
);

CREATE UNIQUE INDEX jobs_idempotency_key
    ON jobs (idempotency_key) WHERE idempotency_key IS NOT NULL;

CREATE INDEX jobs_claimable ON jobs (state, available_at, priority DESC);
CREATE INDEX jobs_requested_by ON jobs (requested_by, created_at DESC);

CREATE TABLE job_attempts (
    id          UUID    PRIMARY KEY,
    job_id      UUID    NOT NULL REFERENCES jobs (id) ON DELETE CASCADE,
    attempt     BIGINT NOT NULL,
    started_at  TEXT    NOT NULL,
    finished_at TEXT,
    outcome     TEXT,
    error       TEXT,
    worker      TEXT    NOT NULL
);

CREATE INDEX job_attempts_job ON job_attempts (job_id, attempt);

CREATE TABLE content_blobs (
    checksum           TEXT    PRIMARY KEY,
    storage_key        TEXT    NOT NULL UNIQUE,
    byte_size          BIGINT NOT NULL CHECK (byte_size >= 0),
    content_type       TEXT    NOT NULL,
    retention_class    TEXT    NOT NULL DEFAULT 'snapshot'
                               CHECK (retention_class IN ('snapshot', 'fetch_cache', 'preservation')),
    created_at         TEXT    NOT NULL,
    last_referenced_at TEXT    NOT NULL
);

CREATE INDEX content_blobs_last_referenced ON content_blobs (last_referenced_at);

CREATE TABLE content_references (
    id         UUID    PRIMARY KEY,
    checksum   TEXT    NOT NULL REFERENCES content_blobs (checksum) ON DELETE CASCADE,
    owner_type TEXT    NOT NULL,
    owner_id   TEXT    NOT NULL,
    created_at TEXT    NOT NULL,
    UNIQUE (checksum, owner_type, owner_id)
);

CREATE INDEX content_references_checksum ON content_references (checksum);
CREATE INDEX content_references_owner ON content_references (owner_type, owner_id);

CREATE TABLE encryption_keys (
    key_id     TEXT    PRIMARY KEY,
    algorithm  TEXT    NOT NULL,
    created_at TEXT    NOT NULL,
    retired_at TEXT
);

CREATE TABLE secrets (
    id         UUID    PRIMARY KEY,
    owner_type TEXT    NOT NULL,
    owner_id   TEXT    NOT NULL,
    name       TEXT    NOT NULL,
    key_id     TEXT    NOT NULL REFERENCES encryption_keys (key_id),
    nonce      TEXT    NOT NULL,
    ciphertext TEXT    NOT NULL,
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    version    BIGINT NOT NULL DEFAULT 1,
    UNIQUE (owner_type, owner_id, name)
);

CREATE INDEX secrets_owner ON secrets (owner_type, owner_id);

CREATE TABLE source_revision_cache_entries (
    id              UUID    PRIMARY KEY,
    source_key      TEXT    NOT NULL,
    revision_key    TEXT    NOT NULL,
    adapter_version TEXT    NOT NULL,
    security_scope  TEXT    NOT NULL,
    checksum        TEXT    NOT NULL REFERENCES content_blobs (checksum) ON DELETE CASCADE,
    expires_at      TEXT    NOT NULL,
    created_at      TEXT    NOT NULL,
    UNIQUE (source_key, revision_key, adapter_version, security_scope)
);

CREATE INDEX source_revision_cache_expiry ON source_revision_cache_entries (expires_at);
