-- Migration 0005 — jobs, the content-addressed store, the revision cache and
-- encrypted secrets (spec §10).
--
-- Dialect: SQLite.
-- Timestamps are RFC 3339 UTC text; identifiers are canonical UUID text.
-- `payload` and `capability_json`-shaped columns are JSON *documents*; every
-- column a query filters or joins on is a real column, because JSON is for
-- flexible documents and not a substitute for searchable relationships
-- (spec §4.1).
--
-- Design notes:
--
--  * **The lease lives on the job row.** An earlier draft of the plan carried a
--    separate `job_leases` table; the spec (§10.1) names `lease_owner` and
--    `lease_expires_at` as job fields, and a second table would be a second
--    source of truth for "who holds this job" — the one thing a claim must not
--    have two of. `claim_next` is a single statement that takes the lease in the
--    same write that selects the row.
--
--  * **`jobs.state` is a closed set**: queued, leased, running, succeeded,
--    failed, cancelled. `retry_wait` is expressed as `queued` with a future
--    `available_at`, so "which job runs next" stays one indexed query.
--
--  * **`content_blobs` is content-addressed**: the checksum *is* the identity,
--    and the storage key is derived from it. Nothing user-supplied ever becomes
--    a path.
--
--  * **`content_references` is what makes deletion safe**, and it is a security
--    boundary rather than a tidiness measure: physical deduplication means two
--    owners can hold the same bytes, so a read path must authorize against the
--    owner it is serving for and never against the checksum's existence
--    (spec §10.4: "a checksum is not an authorization credential").
--
--  * **`content_references.owner_type`/`owner_id`** name the resource that
--    keeps the blob alive (`work`, `chapter_revision`, `export`,
--    `library_item`). spec §4.4 calls the same columns `blob_id` and
--    `authorized_resource_type`/`_id`; the names here are the plan's and the
--    meaning is identical.
--
--  * **`source_revision_cache_entries` is the temporary fetch cache** of
--    spec §10.4, and it is deliberately not the same thing as a user's
--    snapshot: its keys carry the adapter extraction version and the security
--    scope, and every row expires on its own clock.
--
-- Deletion and retention:
--
--  * `jobs` and `job_attempts` are kept for 30 days after reaching a terminal
--    state and then deleted by a maintenance job: they are diagnostics, not
--    history. `requested_by` is kept even after the account is deleted, so an
--    operator can still see that *a* job ran — it is set NULL on account
--    deletion rather than cascading.
--  * `content_blobs` are never deleted by cascade. A blob is removed only when
--    `content_references` has no row for its checksum, and that check
--    (`delete_if_unreferenced`) is the only thing standing between a blob that
--    nothing wants and data loss. A maintenance job that deletes by age will
--    delete a blob a reader is streaming; do not write one.
--  * `content_references` cascade with their owner, which is why the owner is
--    stored as a type plus an id rather than as a foreign key to one table.
--  * `source_revision_cache_entries` expire and are collected independently of
--    any snapshot, and evicting one never touches the blob if a reference to it
--    remains.
--  * `secrets` cascade with their owner: the ciphertext goes with the row. The
--    key lives outside the database, in `LOREHAVEN_SECRET_KEY` or a key file,
--    and is never stored here.
--  * `encryption_keys` are permanent: a retired key must stay listed so a
--    rotation can still say which key encrypted which row.

CREATE TABLE jobs (
    id                TEXT    PRIMARY KEY,
    kind              TEXT    NOT NULL,
    state             TEXT    NOT NULL DEFAULT 'queued',
    payload           TEXT    NOT NULL DEFAULT '{}',
    idempotency_key   TEXT,
    priority          INTEGER NOT NULL DEFAULT 0,
    attempts          INTEGER NOT NULL DEFAULT 0,
    max_attempts      INTEGER NOT NULL DEFAULT 5,
    available_at      TEXT    NOT NULL,
    lease_owner       TEXT,
    lease_expires_at  TEXT,
    progress_permille INTEGER NOT NULL DEFAULT 0
                              CHECK (progress_permille BETWEEN 0 AND 1000),
    checkpoint        TEXT,
    last_error        TEXT,
    requested_by      TEXT    REFERENCES accounts (id) ON DELETE SET NULL,
    created_at        TEXT    NOT NULL,
    updated_at        TEXT    NOT NULL,
    version           INTEGER NOT NULL DEFAULT 1
);

-- Replaying one idempotency key enqueues one job. Partial, because a NULL key
-- means "this job is not idempotent" and SQLite treats NULLs as distinct
-- anyway; saying it explicitly documents the intent.
CREATE UNIQUE INDEX jobs_idempotency_key
    ON jobs (idempotency_key) WHERE idempotency_key IS NOT NULL;

-- The claim query's index: which job to run next.
CREATE INDEX jobs_claimable ON jobs (state, available_at, priority DESC);
CREATE INDEX jobs_requested_by ON jobs (requested_by, created_at DESC);

CREATE TABLE job_attempts (
    id          TEXT    PRIMARY KEY,
    job_id      TEXT    NOT NULL REFERENCES jobs (id) ON DELETE CASCADE,
    attempt     INTEGER NOT NULL,
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
    byte_size          INTEGER NOT NULL CHECK (byte_size >= 0),
    content_type       TEXT    NOT NULL,
    retention_class    TEXT    NOT NULL DEFAULT 'snapshot'
                               CHECK (retention_class IN ('snapshot', 'fetch_cache', 'preservation')),
    created_at         TEXT    NOT NULL,
    last_referenced_at TEXT    NOT NULL
);

CREATE INDEX content_blobs_last_referenced ON content_blobs (last_referenced_at);

CREATE TABLE content_references (
    id         TEXT    PRIMARY KEY,
    checksum   TEXT    NOT NULL REFERENCES content_blobs (checksum) ON DELETE CASCADE,
    owner_type TEXT    NOT NULL,
    owner_id   TEXT    NOT NULL,
    created_at TEXT    NOT NULL,
    UNIQUE (checksum, owner_type, owner_id)
);

-- The deletion check reads this index, and it is the only query that decides
-- whether a blob may be removed.
CREATE INDEX content_references_checksum ON content_references (checksum);
CREATE INDEX content_references_owner ON content_references (owner_type, owner_id);

CREATE TABLE encryption_keys (
    key_id     TEXT    PRIMARY KEY,
    algorithm  TEXT    NOT NULL,
    created_at TEXT    NOT NULL,
    retired_at TEXT
);

CREATE TABLE secrets (
    id         TEXT    PRIMARY KEY,
    owner_type TEXT    NOT NULL,
    owner_id   TEXT    NOT NULL,
    name       TEXT    NOT NULL,
    key_id     TEXT    NOT NULL REFERENCES encryption_keys (key_id),
    nonce      TEXT    NOT NULL,
    ciphertext TEXT    NOT NULL,
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    version    INTEGER NOT NULL DEFAULT 1,
    UNIQUE (owner_type, owner_id, name)
);

CREATE INDEX secrets_owner ON secrets (owner_type, owner_id);

CREATE TABLE source_revision_cache_entries (
    id              TEXT    PRIMARY KEY,
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
