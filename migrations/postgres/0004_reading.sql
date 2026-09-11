-- Migration 0004 — reading progress, ratings, reviews, history, notes and
-- typography preferences (spec §9).
--
-- Dialect: PostgreSQL.
-- Identifiers are native UUID columns; timestamps are RFC 3339 UTC text so the
-- repository layer decodes identically on both engines (see ADR 0004).
--
-- The reasoning behind the shape itself, and the deletion/retention rules, are
-- documented in `migrations/sqlite/0004_reading.sql`; the two files must stay in
-- step. Dialect differences are called out inline.

CREATE TABLE reading_progress (
    id                UUID    PRIMARY KEY,
    account_id        UUID    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    pseud_id          UUID    REFERENCES pseuds (id) ON DELETE CASCADE,
    subject_type      TEXT    NOT NULL,
    subject_id        UUID    NOT NULL,
    chapter_id        UUID,
    content_revision  UUID,
    paragraph_anchor  TEXT,
    position_permille BIGINT NOT NULL DEFAULT 0 CHECK (position_permille BETWEEN 0 AND 1000),
    device_id         TEXT,
    created_at        TEXT    NOT NULL,
    updated_at        TEXT    NOT NULL,
    version           BIGINT NOT NULL DEFAULT 1
);

-- A NULL device_id is treated as distinct in a partial index, exactly as the
-- SQLite migration does with its two partial unique indexes.
CREATE UNIQUE INDEX reading_progress_unique
    ON reading_progress (account_id, pseud_id, subject_type, subject_id, device_id)
    WHERE device_id IS NOT NULL;
CREATE UNIQUE INDEX reading_progress_no_device
    ON reading_progress (account_id, pseud_id, subject_type, subject_id)
    WHERE device_id IS NULL;
CREATE INDEX reading_progress_subject ON reading_progress (subject_type, subject_id);

CREATE TABLE rating (
    id           UUID PRIMARY KEY,
    account_id   UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    pseud_id     UUID NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    work_id      UUID NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    stars        BIGINT NOT NULL CHECK (stars BETWEEN 1 AND 5),
    is_public   BOOLEAN NOT NULL DEFAULT FALSE,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    version      BIGINT NOT NULL DEFAULT 1,
    deleted_at   TEXT
);

CREATE UNIQUE INDEX rating_pseud_work ON rating (pseud_id, work_id) WHERE deleted_at IS NULL;
CREATE INDEX rating_work_public ON rating (work_id) WHERE is_public = TRUE AND deleted_at IS NULL;

CREATE TABLE review (
    id               UUID PRIMARY KEY,
    account_id       UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    pseud_id         UUID NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    work_id          UUID NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    body             TEXT NOT NULL,
    contains_spoilers BOOLEAN NOT NULL DEFAULT FALSE,
    is_public       BOOLEAN NOT NULL DEFAULT FALSE,
    published_at     TEXT,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL,
    version          BIGINT NOT NULL DEFAULT 1,
    deleted_at       TEXT
);

CREATE UNIQUE INDEX review_pseud_work ON review (pseud_id, work_id) WHERE deleted_at IS NULL;
CREATE INDEX review_work_public ON review (work_id) WHERE is_public = TRUE AND published_at IS NOT NULL AND deleted_at IS NULL;

CREATE TABLE reading_history_entry (
    id             UUID PRIMARY KEY,
    account_id     UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    pseud_id       UUID NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    subject_type   TEXT NOT NULL,
    subject_id     UUID NOT NULL,
    last_read_at   TEXT NOT NULL,
    revision_seen  UUID,
    created_at     TEXT NOT NULL
);

CREATE UNIQUE INDEX reading_history_unique
    ON reading_history_entry (account_id, pseud_id, subject_type, subject_id);
CREATE INDEX reading_history_account ON reading_history_entry (account_id, last_read_at DESC);

CREATE TABLE reader_note (
    id           UUID PRIMARY KEY,
    account_id   UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    pseud_id     UUID NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    subject_type TEXT NOT NULL,
    subject_id   UUID NOT NULL,
    anchor       TEXT,
    body         TEXT NOT NULL,
    created_at   TEXT NOT NULL,
    updated_at   TEXT NOT NULL,
    version      BIGINT NOT NULL DEFAULT 1,
    deleted_at   TEXT
);

CREATE INDEX reader_note_subject ON reader_note (pseud_id, subject_type, subject_id) WHERE deleted_at IS NULL;

CREATE TABLE typography_preference (
    account_id        UUID PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    -- DOUBLE PRECISION, not REAL: the repository decodes these as f64, and
    -- SQLite's REAL is already 8 bytes, so REAL (4 bytes) here is the one
    -- declaration that would not round-trip what the API accepted.
    font_scale        DOUBLE PRECISION NOT NULL DEFAULT 1.0,
    line_height       DOUBLE PRECISION NOT NULL DEFAULT 1.6,
    measure           BIGINT NOT NULL DEFAULT 66,
    reader_theme      TEXT NOT NULL DEFAULT 'sepia',
    distraction_free  BOOLEAN NOT NULL DEFAULT FALSE,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    version           BIGINT NOT NULL DEFAULT 1
);
