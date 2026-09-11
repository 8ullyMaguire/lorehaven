-- Migration 0006 — the import framework (spec §11, §14.4).
--
-- Dialect: PostgreSQL.
-- Identifiers are native UUID columns; timestamps are RFC 3339 UTC text so the
-- repository layer decodes identically on both engines (see ADR 0004).
--
-- The reasoning behind the shape, and the deletion and retention rules, are
-- documented in `migrations/sqlite/0006_imports.sql`; the two files must stay in
-- step. Dialect differences are called out inline.
--
-- Note on table order: `library_items` is created before `import_jobs` because
-- `import_jobs.library_item_id` is a foreign key to it. SQLite resolves a
-- foreign-key target lazily and would tolerate the other order; PostgreSQL
-- resolves it at creation time and would not. The SQLite file carries the same
-- order so that a reader comparing the two is not left wondering.

CREATE TABLE sources (
    id              UUID    PRIMARY KEY,
    key             TEXT    NOT NULL UNIQUE,
    display_name    TEXT    NOT NULL,
    adapter_version TEXT    NOT NULL,
    -- BIGINT rather than BOOLEAN, matching every other flag in this
    -- schema: the repository decodes i64 on both engines and compares
    -- to zero, so a native BOOLEAN here would be the one column with a
    -- second decode path. BIGINT rather than INTEGER for the same reason
    -- every other integer here is BIGINT: SQLite's INTEGER is 64-bit, so
    -- a 32-bit column is a type the shared decode cannot read (ADR 0004).
    enabled         BIGINT NOT NULL DEFAULT 1,
    disabled_reason TEXT,
    capability_json TEXT    NOT NULL DEFAULT '{}',
    health          TEXT    NOT NULL DEFAULT 'unknown'
                            CHECK (health IN ('unknown', 'healthy', 'degraded', 'unavailable', 'paused')),
    last_checked_at TEXT,
    created_at      TEXT    NOT NULL,
    updated_at      TEXT    NOT NULL,
    version         BIGINT NOT NULL DEFAULT 1
);

CREATE TABLE source_credentials (
    id              UUID    PRIMARY KEY,
    pseud_id        UUID    NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    source_key      TEXT    NOT NULL,
    secret_id       UUID    NOT NULL REFERENCES secrets (id) ON DELETE CASCADE,
    label           TEXT    NOT NULL,
    status          TEXT    NOT NULL DEFAULT 'active'
                            CHECK (status IN ('active', 'expired', 'rejected', 'revoked')),
    expires_at      TEXT,
    last_checked_at TEXT,
    created_at      TEXT    NOT NULL,
    updated_at      TEXT    NOT NULL,
    version         BIGINT NOT NULL DEFAULT 1,
    UNIQUE (pseud_id, source_key, label)
);

CREATE INDEX source_credentials_pseud ON source_credentials (pseud_id, source_key);
CREATE INDEX source_credentials_expiry ON source_credentials (expires_at)
    WHERE expires_at IS NOT NULL;

CREATE TABLE library_items (
    id                UUID    PRIMARY KEY,
    account_id        UUID    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    work_id           UUID    REFERENCES works (id) ON DELETE SET NULL,
    source_key        TEXT    NOT NULL,
    source_work_key   TEXT    NOT NULL,
    title             TEXT    NOT NULL,
    author_text       TEXT    NOT NULL DEFAULT '',
    author_url        TEXT,
    summary           TEXT    NOT NULL DEFAULT '',
    language          TEXT,
    word_count        BIGINT,
    status            TEXT    NOT NULL DEFAULT 'unknown'
                              CHECK (status IN ('ongoing', 'complete', 'hiatus',
                                                'cancelled', 'unknown')),
    source_url        TEXT    NOT NULL,
    source_updated_at TEXT,
    last_synced_at    TEXT,
    provenance_json   TEXT    NOT NULL DEFAULT '{}',
    created_at        TEXT    NOT NULL,
    updated_at        TEXT    NOT NULL,
    version           BIGINT NOT NULL DEFAULT 1,
    UNIQUE (account_id, source_key, source_work_key)
);

CREATE INDEX library_items_account ON library_items (account_id, updated_at DESC, id DESC);
CREATE INDEX library_items_work ON library_items (work_id) WHERE work_id IS NOT NULL;

CREATE TABLE import_jobs (
    id               UUID    PRIMARY KEY,
    job_id           UUID    NOT NULL UNIQUE REFERENCES jobs (id) ON DELETE CASCADE,
    account_id       UUID    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    pseud_id         UUID    NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    source_key       TEXT    NOT NULL,
    source_url       TEXT    NOT NULL,
    destination_type TEXT    NOT NULL
                             CHECK (destination_type IN ('library', 'draft')),
    destination_id   UUID,
    dry_run          BIGINT NOT NULL DEFAULT 0,
    state            TEXT    NOT NULL DEFAULT 'queued'
                             CHECK (state IN ('queued', 'running', 'paused', 'completed',
                                              'failed', 'cancelled')),
    library_item_id  UUID    REFERENCES library_items (id) ON DELETE SET NULL,
    report_json      TEXT,
    created_at       TEXT    NOT NULL,
    updated_at       TEXT    NOT NULL,
    version          BIGINT NOT NULL DEFAULT 1
);

CREATE INDEX import_jobs_account ON import_jobs (account_id, created_at DESC);
CREATE INDEX import_jobs_source ON import_jobs (account_id, source_key, source_url);

CREATE TABLE import_chapters (
    id                    UUID    PRIMARY KEY,
    import_job_id         UUID    NOT NULL REFERENCES import_jobs (id) ON DELETE CASCADE,
    library_item_id       UUID    REFERENCES library_items (id) ON DELETE CASCADE,
    source_chapter_key    TEXT    NOT NULL,
    ordinal               BIGINT NOT NULL CHECK (ordinal >= 1),
    title                 TEXT    NOT NULL DEFAULT '',
    state                 TEXT    NOT NULL DEFAULT 'pending'
                                  CHECK (state IN ('pending', 'stored', 'skipped', 'failed')),
    content_blob_checksum TEXT    REFERENCES content_blobs (checksum),
    chapter_id            UUID    REFERENCES chapters (id) ON DELETE SET NULL,
    note                  TEXT,
    created_at            TEXT    NOT NULL,
    updated_at            TEXT    NOT NULL,
    UNIQUE (import_job_id, source_chapter_key)
);

CREATE INDEX import_chapters_job ON import_chapters (import_job_id, ordinal);
CREATE INDEX import_chapters_item ON import_chapters (library_item_id, ordinal);
