-- Migration 0003 — works, chapters, revisions, publication and collaboration
-- (spec §4.3, §8).
--
-- Dialect: SQLite.
-- Timestamps are RFC 3339 UTC text; identifiers are canonical UUID text.
--
-- Design notes that matter (ADR 0002, and ADR 0005 which this migration
-- introduces):
--
--  * A work's lifecycle, visibility and completion are three separate columns.
--    They answer three different questions and change on three different
--    schedules; a single `status` cannot represent "published, unlisted,
--    still in progress".
--  * `chapter_revisions` is append-only. Restoring an older revision inserts a
--    *new* row that points at the old one, so a reader's stored position always
--    resolves and nothing a reader saw is ever rewritten.
--  * `works.version` and `chapters.version` are the optimistic-concurrency
--    counters (spec §3.4). Every mutating statement carries the expected
--    version in its WHERE clause, so a stale edit affects zero rows and
--    becomes REVISION_CONFLICT rather than a silently lost chapter.
--  * `outbox_events` is written *in the same transaction* as the publication
--    state change (spec §8.4). Nothing sends mail from inside a transaction;
--    delivery happens later, from the row.
--  * Boolean-ish flags are INTEGER 0/1 rather than a dialect-specific BOOLEAN,
--    so one bound value and one decode path serve both engines (ADR 0004).
--
-- Deletion and retention:
--  * works and chapters are deleted *through* the application; `deleted_at`
--    keeps a work's row addressable for moderation and audit. The database
--    cascades only when a work is genuinely removed, so a contributor row, a
--    revision or a publication event never outlives its work as an orphan.
--  * publication_events are the permanent record of what was published and
--    when; they cascade with the work only, never with a chapter.
--  * outbox_events are deleted by the worker once delivered (spec §8.4);
--    undelivered rows are retained until they are delivered or abandoned.

CREATE TABLE works (
    id              TEXT    PRIMARY KEY,
    -- Ownership belongs to the *pseud*, not the account (ADR 0003, spec §8:
    -- "pseud switching does not change ownership").
    owner_pseud_id  TEXT    NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    title           TEXT    NOT NULL DEFAULT '',
    summary         TEXT    NOT NULL DEFAULT '',
    language        TEXT    NOT NULL DEFAULT 'en',
    rating          TEXT    NOT NULL DEFAULT 'general',
    visibility      TEXT    NOT NULL DEFAULT 'public',
    lifecycle       TEXT    NOT NULL DEFAULT 'draft',
    completion      TEXT    NOT NULL DEFAULT 'in_progress',
    -- Set when a work is scheduled; the worker publishes when it comes due.
    scheduled_for   TEXT,
    published_at    TEXT,
    withdrawn_at    TEXT,
    -- A work owner may hide the public rating aggregate without touching
    -- anyone's private rating (spec §9.4).
    show_public_ratings INTEGER NOT NULL DEFAULT 1,
    created_at      TEXT    NOT NULL,
    updated_at      TEXT    NOT NULL,
    version         INTEGER NOT NULL DEFAULT 1,
    deleted_at      TEXT
);

CREATE INDEX works_owner_updated ON works (owner_pseud_id, updated_at);
-- The listing index. `lifecycle` first because every public listing filters on
-- it, and an index that leads with a column nobody filters by is decoration.
CREATE INDEX works_listing ON works (lifecycle, visibility, updated_at);
CREATE INDEX works_lifecycle_scheduled ON works (lifecycle, scheduled_for);

CREATE TABLE work_contributors (
    work_id            TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    pseud_id           TEXT    NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    role               TEXT    NOT NULL,
    public_attribution INTEGER NOT NULL DEFAULT 1,
    created_at         TEXT    NOT NULL,
    PRIMARY KEY (work_id, pseud_id)
);

CREATE INDEX work_contributors_pseud ON work_contributors (pseud_id);

CREATE TABLE chapters (
    id                  TEXT    PRIMARY KEY,
    work_id             TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    -- Position, spaced by ten so an insert between two chapters is one write
    -- rather than a renumbering of everything after it.
    order_key           INTEGER NOT NULL,
    title               TEXT    NOT NULL DEFAULT '',
    -- NULL until the chapter has been saved once. SQLite resolves a foreign key
    -- target when it is used rather than when it is declared, so this may
    -- point at `chapter_revisions`, which is created below.
    current_revision_id TEXT    REFERENCES chapter_revisions (id),
    created_at          TEXT    NOT NULL,
    updated_at          TEXT    NOT NULL,
    version             INTEGER NOT NULL DEFAULT 1,
    deleted_at          TEXT
);

-- Deliberately not UNIQUE: reordering two chapters transiently collides, and a
-- unique index would turn a swap into a two-phase dance with a temporary value.
CREATE INDEX chapters_work_order ON chapters (work_id, order_key);
CREATE INDEX chapters_revision ON chapters (current_revision_id);

CREATE TABLE chapter_revisions (
    id                   TEXT    PRIMARY KEY,
    chapter_id           TEXT    NOT NULL REFERENCES chapters (id) ON DELETE CASCADE,
    revision_number      INTEGER NOT NULL,
    -- The structured editor document is the source of truth; the two derived
    -- columns exist so search, download and the reader do not each re-derive
    -- them differently (ADR 0002).
    document_json        TEXT    NOT NULL,
    sanitized_html       TEXT    NOT NULL,
    plain_text           TEXT    NOT NULL,
    word_count           INTEGER NOT NULL,
    -- The author's note for this revision, if they left one.
    note                 TEXT,
    created_by_pseud_id  TEXT    NOT NULL REFERENCES pseuds (id),
    -- Set when this revision was produced by restoring an earlier one.
    restored_from_id     TEXT    REFERENCES chapter_revisions (id),
    created_at           TEXT    NOT NULL,
    UNIQUE (chapter_id, revision_number)
);

CREATE INDEX chapter_revisions_chapter ON chapter_revisions (chapter_id, revision_number DESC);

CREATE TABLE publication_events (
    id               TEXT PRIMARY KEY,
    work_id          TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    chapter_id       TEXT REFERENCES chapters (id) ON DELETE SET NULL,
    -- publish | schedule | withdraw | restore_revision | update
    action           TEXT NOT NULL,
    actor_pseud_id   TEXT NOT NULL REFERENCES pseuds (id),
    -- The publication service is idempotent: replaying the same key is a no-op
    -- rather than a second notification (spec §8 acceptance).
    idempotency_key  TEXT,
    note             TEXT,
    occurred_at      TEXT NOT NULL
);

CREATE UNIQUE INDEX publication_events_idempotency
    ON publication_events (idempotency_key) WHERE idempotency_key IS NOT NULL;
CREATE INDEX publication_events_work ON publication_events (work_id, occurred_at);

CREATE TABLE collaboration_invites (
    id                   TEXT    PRIMARY KEY,
    work_id              TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    -- The invitation names the *pseud* being invited and the pseud doing the
    -- inviting. An account-level invitation would disclose linkage the platform
    -- promises not to disclose (ADR 0003).
    invited_pseud_id     TEXT    NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    invited_by_pseud_id  TEXT    NOT NULL REFERENCES pseuds (id),
    role                 TEXT    NOT NULL,
    -- pending | accepted | declined | revoked | expired
    status               TEXT    NOT NULL DEFAULT 'pending',
    token_hash           TEXT    NOT NULL,
    message              TEXT,
    created_at           TEXT    NOT NULL,
    updated_at           TEXT    NOT NULL,
    version              INTEGER NOT NULL DEFAULT 1,
    responded_at         TEXT
);

CREATE UNIQUE INDEX collaboration_invites_token ON collaboration_invites (token_hash);
CREATE INDEX collaboration_invites_work ON collaboration_invites (work_id, status);
CREATE INDEX collaboration_invites_invited ON collaboration_invites (invited_pseud_id, status);

-- The transactional outbox (spec §8.4). One row is one side effect that must
-- happen exactly once, written in the same transaction as its cause.
CREATE TABLE outbox_events (
    id           TEXT    PRIMARY KEY,
    -- publish.notify | publish.index | withdraw.deindex | revision.index ...
    topic        TEXT    NOT NULL,
    payload      TEXT    NOT NULL,
    -- Makes the whole side effect idempotent: a second insert with the same key
    -- is refused by the index, so a retried publication cannot notify twice.
    dedupe_key   TEXT,
    created_at   TEXT    NOT NULL,
    available_at TEXT    NOT NULL,
    claimed_at   TEXT,
    delivered_at TEXT,
    attempts     INTEGER NOT NULL DEFAULT 0,
    last_error   TEXT,
    UNIQUE (dedupe_key)
);

CREATE INDEX outbox_events_pending ON outbox_events (delivered_at, available_at);

-- The seed and tests reset content in dependency order; the application deletes
-- through the work, so this exists only for the development seed.
CREATE INDEX works_deleted ON works (deleted_at);
