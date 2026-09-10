-- Migration 0003 — works, chapters, revisions, publication and collaboration
-- (spec §4.3, §8).
--
-- Dialect: PostgreSQL.
-- Identifiers are native UUID columns; timestamps are RFC 3339 UTC text, and
-- 0/1 flags are INTEGER, so the repository layer decodes identically on both
-- engines (ADR 0004). The reasoning behind the shape itself is documented in
-- `migrations/sqlite/0003_works.sql` and in ADR 0002/0005.
--
-- One deliberate dialect difference: PostgreSQL validates a foreign key target
-- when the constraint is created, and `chapters.current_revision_id` points at
-- a table that cannot exist yet because `chapter_revisions.chapter_id` points
-- back at `chapters`. So the columns are created first and the circular
-- constraint is added at the end of this file. SQLite resolves foreign key
-- targets lazily and keeps the declaration inline.
--
-- Deletion and retention: as documented in the SQLite file — content cascades
-- from the work, publication events are the permanent record, and delivered
-- outbox rows are removed by the worker.

CREATE TABLE works (
    id                  UUID    PRIMARY KEY,
    owner_pseud_id      UUID    NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    title               TEXT    NOT NULL DEFAULT '',
    summary             TEXT    NOT NULL DEFAULT '',
    language            TEXT    NOT NULL DEFAULT 'en',
    rating              TEXT    NOT NULL DEFAULT 'general',
    visibility          TEXT    NOT NULL DEFAULT 'public',
    lifecycle           TEXT    NOT NULL DEFAULT 'draft',
    completion          TEXT    NOT NULL DEFAULT 'in_progress',
    scheduled_for       TEXT,
    published_at        TEXT,
    withdrawn_at        TEXT,
    show_public_ratings INTEGER NOT NULL DEFAULT 1,
    created_at          TEXT    NOT NULL,
    updated_at          TEXT    NOT NULL,
    version             INTEGER NOT NULL DEFAULT 1,
    deleted_at          TEXT
);

CREATE INDEX works_owner_updated ON works (owner_pseud_id, updated_at);
CREATE INDEX works_listing ON works (lifecycle, visibility, updated_at);
CREATE INDEX works_lifecycle_scheduled ON works (lifecycle, scheduled_for);
CREATE INDEX works_deleted ON works (deleted_at);

CREATE TABLE work_contributors (
    work_id            UUID    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    pseud_id           UUID    NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    role               TEXT    NOT NULL,
    public_attribution INTEGER NOT NULL DEFAULT 1,
    created_at         TEXT    NOT NULL,
    PRIMARY KEY (work_id, pseud_id)
);

CREATE INDEX work_contributors_pseud ON work_contributors (pseud_id);

CREATE TABLE chapters (
    id                  UUID    PRIMARY KEY,
    work_id             UUID    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    order_key           INTEGER NOT NULL,
    title               TEXT    NOT NULL DEFAULT '',
    current_revision_id UUID,
    created_at          TEXT    NOT NULL,
    updated_at          TEXT    NOT NULL,
    version             INTEGER NOT NULL DEFAULT 1,
    deleted_at          TEXT
);

CREATE INDEX chapters_work_order ON chapters (work_id, order_key);
CREATE INDEX chapters_revision ON chapters (current_revision_id);

CREATE TABLE chapter_revisions (
    id                   UUID    PRIMARY KEY,
    chapter_id           UUID    NOT NULL REFERENCES chapters (id) ON DELETE CASCADE,
    revision_number      INTEGER NOT NULL,
    document_json        TEXT    NOT NULL,
    sanitized_html       TEXT    NOT NULL,
    plain_text           TEXT    NOT NULL,
    word_count           INTEGER NOT NULL,
    note                 TEXT,
    created_by_pseud_id  UUID    NOT NULL REFERENCES pseuds (id),
    restored_from_id     UUID    REFERENCES chapter_revisions (id),
    created_at           TEXT    NOT NULL,
    UNIQUE (chapter_id, revision_number)
);

CREATE INDEX chapter_revisions_chapter ON chapter_revisions (chapter_id, revision_number DESC);

-- The circular constraint, added once both tables exist.
ALTER TABLE chapters
    ADD CONSTRAINT chapters_current_revision_fk
    FOREIGN KEY (current_revision_id) REFERENCES chapter_revisions (id) ON DELETE SET NULL;

CREATE TABLE publication_events (
    id              UUID PRIMARY KEY,
    work_id         UUID NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    chapter_id      UUID REFERENCES chapters (id) ON DELETE SET NULL,
    action          TEXT NOT NULL,
    actor_pseud_id  UUID NOT NULL REFERENCES pseuds (id),
    idempotency_key TEXT,
    note            TEXT,
    occurred_at     TEXT NOT NULL
);

CREATE UNIQUE INDEX publication_events_idempotency
    ON publication_events (idempotency_key) WHERE idempotency_key IS NOT NULL;
CREATE INDEX publication_events_work ON publication_events (work_id, occurred_at);

CREATE TABLE collaboration_invites (
    id                  UUID    PRIMARY KEY,
    work_id             UUID    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    invited_pseud_id    UUID    NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    invited_by_pseud_id UUID    NOT NULL REFERENCES pseuds (id),
    role                TEXT    NOT NULL,
    status              TEXT    NOT NULL DEFAULT 'pending',
    token_hash          TEXT    NOT NULL,
    message             TEXT,
    created_at          TEXT    NOT NULL,
    updated_at          TEXT    NOT NULL,
    version             INTEGER NOT NULL DEFAULT 1,
    responded_at        TEXT
);

CREATE UNIQUE INDEX collaboration_invites_token ON collaboration_invites (token_hash);
CREATE INDEX collaboration_invites_work ON collaboration_invites (work_id, status);
CREATE INDEX collaboration_invites_invited ON collaboration_invites (invited_pseud_id, status);

CREATE TABLE outbox_events (
    id           UUID    PRIMARY KEY,
    topic        TEXT    NOT NULL,
    payload      TEXT    NOT NULL,
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
