-- Migration 0006 — the import framework (spec §11, §14.4).
--
-- Dialect: SQLite.
-- Timestamps are RFC 3339 UTC text; identifiers are canonical UUID text.
--
-- Design notes:
--
--  * **`import_jobs` and `import_chapters` are permanent.** They are the
--    provenance record — what we took, from where, when, and what happened to
--    each chapter — and `library_items.provenance_json` points back at them.
--    Spec §11.11 requires an imported work to be able to say where it came from
--    and on what basis, and that is impossible if the record is swept up by a
--    retention job.
--
--  * **`library_items` is per account, not per pseud and not global.** Two
--    readers who import the same URL get two rows: these are private copies, not
--    catalogue entries (spec §11.2, and the plan's "library_items is per
--    account"). Nothing here is visible to another account, and the unique key
--    is what makes re-importing a URL an *update* rather than a duplicate.
--
--  * **`source_credentials` is per pseud, not per account.** spec §11.6 says
--    pseud-scoped by default and "no automatic copying across pseuds". The
--    plan's schema sketch had `account_id` here; the spec's rule wins, because
--    a credential is exactly the thing that must not silently follow an account
--    from one public identity to another.
--
--  * **A credential is a pointer to a `secrets` row, never the secret.** The
--    ciphertext lives in `secrets` (migration 0005) and cascades: deleting the
--    credential deletes the ciphertext with it, and nothing readable is stored
--    in this table. `status` is the operational state — an `expired` credential
--    pauses its imports rather than retrying them (spec §11.6).
--
--  * **A chapter body is a blob, not a column.** `import_chapters` records the
--    checksum; the bytes live in `content_blobs` (migration 0005) and stay alive
--    through a `content_references` row owned by the library item. That is what
--    makes two readers' copies of the same chapter one file on disk, and it is
--    why deleting an import must drop a *reference* rather than a file.
--
--  * **`sources` is the operator's view, not the code's.** The registry is the
--    authority on which adapters exist; this table records what an operator has
--    switched off and why, so a pause survives a restart and a deploy (spec
--    §11.8's `paused` state).
--
--  * **`import_chapters.state` distinguishes `skipped` from `failed`.** A
--    chapter a re-import did not need to fetch is not a chapter that went wrong,
--    and a report that conflates them tells a reader nothing.
--
-- Deletion and retention:
--
--  * `library_items` cascade with the account, and their `content_references`
--    rows cascade with the item — which is what lets a blob become collectable
--    when the last reader holding it goes away.
--  * `import_jobs` and `import_chapters` cascade with the account too: they are
--    permanent *as history*, not permanent past the deletion of the account they
--    describe. The row that must outlive an account is a *published* work, and
--    this milestone does not create one.
--  * `works` and `chapters` are referenced with `ON DELETE SET NULL`: an import
--    that was materialised into a work keeps its provenance row, and the row
--    stops naming a work that no longer exists rather than disappearing.
--  * `content_blobs` are never deleted by cascade. As in 0005, a blob goes only
--    through `delete_if_unreferenced`.

CREATE TABLE sources (
    id              TEXT    PRIMARY KEY,
    -- The registry's key. Unique, because two rows for one source would be two
    -- answers to "is this source enabled".
    key             TEXT    NOT NULL UNIQUE,
    display_name    TEXT    NOT NULL,
    -- The adapter's version, so a row can say what parsed an import when the
    -- parser has since changed (spec §11.8: adapter-version attribution).
    adapter_version TEXT    NOT NULL,
    enabled         INTEGER NOT NULL DEFAULT 1,
    disabled_reason TEXT,
    capability_json TEXT    NOT NULL DEFAULT '{}',
    -- Rolling outcome counts for the source's health (spec §11.8). Stored here
    -- rather than derived on every read: the question "is this source healthy"
    -- is asked on the import page and must not scan the import history.
    health          TEXT    NOT NULL DEFAULT 'unknown'
                            CHECK (health IN ('unknown', 'healthy', 'degraded', 'unavailable', 'paused')),
    last_checked_at TEXT,
    created_at      TEXT    NOT NULL,
    updated_at      TEXT    NOT NULL,
    version         INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE source_credentials (
    id              TEXT    PRIMARY KEY,
    pseud_id        TEXT    NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    source_key      TEXT    NOT NULL,
    -- The ciphertext, by reference. Never the secret itself.
    secret_id       TEXT    NOT NULL REFERENCES secrets (id) ON DELETE CASCADE,
    label           TEXT    NOT NULL,
    status          TEXT    NOT NULL DEFAULT 'active'
                            CHECK (status IN ('active', 'expired', 'rejected', 'revoked')),
    expires_at      TEXT,
    last_checked_at TEXT,
    created_at      TEXT    NOT NULL,
    updated_at      TEXT    NOT NULL,
    version         INTEGER NOT NULL DEFAULT 1,
    UNIQUE (pseud_id, source_key, label)
);

CREATE INDEX source_credentials_pseud ON source_credentials (pseud_id, source_key);
-- The expiry sweep: which credentials have run out.
CREATE INDEX source_credentials_expiry ON source_credentials (expires_at)
    WHERE expires_at IS NOT NULL;

CREATE TABLE library_items (
    id                TEXT    PRIMARY KEY,
    account_id        TEXT    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    -- Set when the import was materialised into a work the reader owns. NULL for
    -- a private-library copy, which is what this milestone produces.
    work_id           TEXT    REFERENCES works (id) ON DELETE SET NULL,
    source_key        TEXT    NOT NULL,
    source_work_key   TEXT    NOT NULL,
    title             TEXT    NOT NULL,
    author_text       TEXT    NOT NULL DEFAULT '',
    author_url        TEXT,
    summary           TEXT    NOT NULL DEFAULT '',
    language          TEXT,
    word_count        INTEGER,
    status            TEXT    NOT NULL DEFAULT 'unknown'
                              CHECK (status IN ('ongoing', 'complete', 'hiatus',
                                                'cancelled', 'unknown')),
    source_url        TEXT    NOT NULL,
    -- The source's own last-changed date, and when we last looked. Kept
    -- separately: conflating them makes "the author updated it" and "we checked
    -- it" the same fact, and an update check needs to tell them apart.
    source_updated_at TEXT,
    last_synced_at    TEXT,
    -- Provenance: which import produced this, under what adapter version, and
    -- from which URL. A JSON document.
    provenance_json   TEXT    NOT NULL DEFAULT '{}',
    created_at        TEXT    NOT NULL,
    updated_at        TEXT    NOT NULL,
    version           INTEGER NOT NULL DEFAULT 1,
    UNIQUE (account_id, source_key, source_work_key)
);

CREATE INDEX library_items_account ON library_items (account_id, updated_at DESC, id DESC);
CREATE INDEX library_items_work ON library_items (work_id) WHERE work_id IS NOT NULL;

CREATE TABLE import_jobs (
    id               TEXT    PRIMARY KEY,
    -- The queue row this import rides on. One import is one job, and the unique
    -- constraint means a retry of the *queue* entry cannot become two imports.
    job_id           TEXT    NOT NULL UNIQUE REFERENCES jobs (id) ON DELETE CASCADE,
    account_id       TEXT    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    -- The pseud that asked, for the credential scope and the audit trail.
    pseud_id         TEXT    NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    source_key       TEXT    NOT NULL,
    -- The URL as pasted. Kept because it is evidence, and because spec §11.3
    -- requires redacting it from access logs — which is only possible if the
    -- one place that legitimately holds it is here.
    source_url       TEXT    NOT NULL,
    destination_type TEXT    NOT NULL
                             CHECK (destination_type IN ('library', 'draft')),
    destination_id   TEXT,
    dry_run          INTEGER NOT NULL DEFAULT 0,
    state            TEXT    NOT NULL DEFAULT 'queued'
                             CHECK (state IN ('queued', 'running', 'paused', 'completed',
                                              'failed', 'cancelled')),
    -- The library item this import produced or updated, once it has one.
    library_item_id  TEXT    REFERENCES library_items (id) ON DELETE SET NULL,
    -- The per-chapter report: what was created, updated, removed, reordered, and
    -- what failed. A JSON *document* — nothing queries inside it.
    report_json      TEXT,
    created_at       TEXT    NOT NULL,
    updated_at       TEXT    NOT NULL,
    version          INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX import_jobs_account ON import_jobs (account_id, created_at DESC);
-- One live import per work per account: the second request for a URL updates
-- what the first produced instead of racing it (spec §11.13 and the plan's
-- "importing the same URL twice updates rather than duplicates").
CREATE INDEX import_jobs_source ON import_jobs (account_id, source_key, source_url);

CREATE TABLE import_chapters (
    id                    TEXT    PRIMARY KEY,
    import_job_id         TEXT    NOT NULL REFERENCES import_jobs (id) ON DELETE CASCADE,
    -- The library item the chapter belongs to, denormalised so a reader's page
    -- can list chapters without joining through the job.
    library_item_id       TEXT    REFERENCES library_items (id) ON DELETE CASCADE,
    -- The source's own chapter identifier when it has one, the ordinal when it
    -- does not. The unique key below is what makes a re-run skip what it already
    -- has rather than storing it twice.
    source_chapter_key    TEXT    NOT NULL,
    ordinal               INTEGER NOT NULL CHECK (ordinal >= 1),
    title                 TEXT    NOT NULL DEFAULT '',
    state                 TEXT    NOT NULL DEFAULT 'pending'
                                  CHECK (state IN ('pending', 'stored', 'skipped', 'failed')),
    -- The body, by checksum into `content_blobs`.
    content_blob_checksum TEXT    REFERENCES content_blobs (checksum),
    -- Set when the chapter was materialised into a work's chapter.
    chapter_id            TEXT    REFERENCES chapters (id) ON DELETE SET NULL,
    note                  TEXT,
    created_at            TEXT    NOT NULL,
    updated_at            TEXT    NOT NULL,
    UNIQUE (import_job_id, source_chapter_key)
);

CREATE INDEX import_chapters_job ON import_chapters (import_job_id, ordinal);
CREATE INDEX import_chapters_item ON import_chapters (library_item_id, ordinal);
