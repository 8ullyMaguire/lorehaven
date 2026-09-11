-- Migration 0008 — exports, download grants and device delivery (spec §13).
--
-- Dialect: PostgreSQL.
-- Timestamps are RFC 3339 UTC text; identifiers are canonical UUID text.
--
-- Design notes:
--
--  * **A download URL is a capability, so the token is stored hashed.** Spec
--    §13.2 allows a download URL that is either authenticated or short-lived and
--    scoped; this is the second, and the two properties that make it safe are
--    that it *expires* and that it can be used *once*. A plaintext token in the
--    table would make a database read sufficient to download somebody's library,
--    so what is stored is the hash — the token itself exists only in the response
--    that minted it.
--
--  * **A grant names no work and no account.** The URL carries a random token and
--    nothing derived from the request it came from. That is deliberate: a
--    capability that leaked through a Referer header or a shared screenshot would
--    otherwise disclose which work the reader had exported.
--
--  * **A delivery row survives its export, and the export does not survive
--    forever.** Exports are swept seven days after creation, output blob and all
--    (spec §13.2 and §10.3's retention classes). A `device_deliveries` row is not
--    swept with it, because "we sent it and it bounced" is exactly the diagnostic
--    an operator needs later, and a cascade would delete the only record of it.
--    The foreign key is therefore `ON DELETE SET NULL` rather than `CASCADE`.
--
--  * **The delivery address is personal data and is stored on the delivery, not
--    on the device.** Spec §13.4 requires a device address to be verifiable,
--    revocable and deletable, and requires the operator's audit log to record it
--    as a hash rather than in the clear. Keeping it on the delivery row means the
--    address that was actually written to is the address that is recorded, and a
--    deleted device deletes its address rather than orphaning it.
--
--  * **`output_blob_checksum` is nullable and that is the honest shape.** A job
--    that has not run yet has no output; a job that failed has none either; and a
--    row claiming a checksum for a file that does not exist is the "empty file
--    reported as success" defect the plan's fourth pitfall names.
--
--  * **`converter_version` is recorded because spec §13.1 requires the converter
--    version in the verification evidence.** An output produced by a different
--    `pandoc` than the one tested is a different artifact, and without this column
--    nothing can tell the two apart afterwards.
--
--  * **`state` is the export's own lifecycle, not the queue's.** `jobs` already
--    tracks the job; this tracks whether the *reader* has something to download,
--    which is a different question and outlives a retry.

CREATE TABLE export_jobs (
    id                    UUID    PRIMARY KEY,
    -- The queue row this export rides on. One export is one job, and the unique
    -- constraint means a retry of the queue entry cannot become two exports.
    job_id                UUID    NOT NULL UNIQUE REFERENCES jobs (id) ON DELETE CASCADE,
    account_id            UUID    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    -- What is being exported. A work is the case spec §13 describes; a library
    -- item is an imported copy, which shares the machinery but not the access
    -- check, so the two are named rather than assumed.
    subject_type          TEXT    NOT NULL CHECK (subject_type IN ('work', 'library_item')),
    subject_id            UUID    NOT NULL,
    -- The format asked for, stored as written so a format this build later drops
    -- is still identifiable in an old row rather than unreadable.
    format                TEXT    NOT NULL,
    -- Typography and other per-export choices (spec §13.2: "user-selected
    -- typography where supported"). A JSON *document*: nothing queries inside it.
    options_json          TEXT,
    -- When the reader acknowledged that the delivery provider will see the
    -- content (spec §13.4). Nullable, and an export that requires delivery is
    -- refused while it is null — the notice is an acknowledgement, not a text.
    privacy_acknowledged_at TEXT,
    state                 TEXT    NOT NULL DEFAULT 'queued'
                                  CHECK (state IN ('queued', 'running', 'ready', 'failed',
                                                   'cancelled')),
    -- The rendered artifact. The bytes live in `content_blobs` (migration 0005);
    -- this is the pointer, and it is null until the job has produced something.
    output_blob_checksum  TEXT,
    output_bytes          BIGINT,
    -- Which converter produced it, for the evidence spec §13.1 asks for.
    converter_version     TEXT,
    -- Why it failed, in the reader's words rather than an operator's.
    error_message         TEXT,
    created_at            TEXT    NOT NULL,
    updated_at            TEXT    NOT NULL,
    version               BIGINT NOT NULL DEFAULT 1
);

-- A reader's own exports, newest first — the only way this table is listed.
CREATE INDEX export_jobs_account ON export_jobs (account_id, created_at DESC, id DESC);
-- The retention sweep: which exports have aged out.
CREATE INDEX export_jobs_created ON export_jobs (created_at);
-- One live export of a subject in a given format per account. Asking twice while
-- one is running is a client mistake, and making it a lookup rather than a second
-- job is what keeps an impatient reader from queueing five EPUBs.
CREATE INDEX export_jobs_subject ON export_jobs (account_id, subject_type, subject_id, format);

CREATE TABLE download_grants (
    id            UUID    PRIMARY KEY,
    export_job_id UUID    NOT NULL REFERENCES export_jobs (id) ON DELETE CASCADE,
    -- SHA-256 of the token that appears in the URL. Never the token.
    token_hash    TEXT    NOT NULL UNIQUE,
    expires_at    TEXT    NOT NULL,
    -- Set the first time it is redeemed, which is what makes it single-use.
    used_at       TEXT,
    single_use    BIGINT NOT NULL DEFAULT 1,
    created_at    TEXT    NOT NULL
);

-- Redeeming a token: one lookup by hash, which is the only read this table gets.
CREATE INDEX download_grants_export ON download_grants (export_job_id);
-- The expiry sweep.
CREATE INDEX download_grants_expiry ON download_grants (expires_at);

CREATE TABLE device_deliveries (
    id            UUID    PRIMARY KEY,
    -- Nullable rather than cascading: the diagnostic outlives the export.
    export_job_id UUID    REFERENCES export_jobs (id) ON DELETE SET NULL,
    account_id    UUID    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    -- Where it was sent. Named rather than inferred, because "sent to a Kindle"
    -- and "sent by email" fail differently and are supported separately.
    target        TEXT    NOT NULL CHECK (target IN ('kindle', 'email')),
    address       TEXT    NOT NULL,
    state         TEXT    NOT NULL DEFAULT 'queued'
                          CHECK (state IN ('queued', 'sent', 'failed')),
    last_error    TEXT,
    created_at    TEXT    NOT NULL,
    delivered_at  TEXT
);

CREATE INDEX device_deliveries_account ON device_deliveries (account_id, created_at DESC);
-- The bounce list an operator reads: what failed, most recent first.
CREATE INDEX device_deliveries_state ON device_deliveries (state, created_at DESC);

CREATE TABLE user_devices (
    id                     UUID    PRIMARY KEY,
    account_id             UUID    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    label                  TEXT    NOT NULL DEFAULT '',
    -- A Web Push subscription, as the browser gave it. A JSON document, and
    -- nullable because a device known only as a delivery address has none.
    push_subscription_json TEXT,
    last_seen_at           TEXT,
    created_at             TEXT    NOT NULL,
    updated_at             TEXT    NOT NULL
);

CREATE INDEX user_devices_account ON user_devices (account_id, updated_at DESC);
