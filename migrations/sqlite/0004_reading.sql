-- Migration 0004 — reading progress, ratings, reviews, history, notes and
-- typography preferences (spec §9).
--
-- Dialect: SQLite.
-- Timestamps are RFC 3339 UTC text; identifiers are canonical UUID text.
--
-- Design notes:
--
--  * `reading_progress` is keyed by (account, pseud, subject, device) so that
--    two devices keep two positions. A NULL device_id is treated as distinct
--    by SQLite's UNIQUE index, so a reader with no device id keeps one row
--    rather than colliding with itself. Documented because it is surprising.
--
--  * `position_fraction` is stored as `position_permille` (INTEGER 0..1000),
--    not as a REAL. The crate's convention binds only String and i64; permille
--    is enough precision for "where was I" and keeps the bind path uniform.
--
--  * `rating` and `review` are private by default. A rating contributes to the
--    public aggregate only when `is_public = 1`; a review is visible to others
--    only when `is_public = 1` AND `published_at IS NOT NULL`. The aggregate
--    query enforces both, and a minimum count, so a stray row can never leak.
--
--  * `reader_note` is per (pseud, subject): a note belongs to the face that
--    wrote it, not to the account.
--
-- Deletion and retention:
--
--  * `reading_progress`, `reading_history_entry`, `reader_note`,
--    `typography_preference` cascade with the account: they are private reading
--    data with no audit value, and a deletion workflow must reach them.
--  * `rating` and `review` soft-delete (`deleted_at`) so a public aggregate can
--    be recomputed after a deletion, and so a moderation review can still see
--    what was written. They cascade with the account only on hard deletion.
--  * Nothing in this migration is ever readable by another account. A future
--    contributor will be tempted to join `rating` into a public query; the
--    `is_public` flag and the minimum-count threshold exist to prevent that.

CREATE TABLE reading_progress (
    id               TEXT    PRIMARY KEY,
    account_id       TEXT    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    pseud_id         TEXT    REFERENCES pseuds (id) ON DELETE CASCADE,
    subject_type     TEXT    NOT NULL,
    subject_id       TEXT    NOT NULL,
    chapter_id       TEXT,
    content_revision TEXT,
    paragraph_anchor TEXT,
    position_permille INTEGER NOT NULL DEFAULT 0,
    device_id        TEXT,
    created_at       TEXT    NOT NULL,
    updated_at       TEXT    NOT NULL,
    version          INTEGER NOT NULL DEFAULT 1
);

CREATE UNIQUE INDEX reading_progress_unique
    ON reading_progress (account_id, pseud_id, subject_type, subject_id, device_id)
    WHERE device_id IS NOT NULL;
CREATE UNIQUE INDEX reading_progress_no_device
    ON reading_progress (account_id, pseud_id, subject_type, subject_id)
    WHERE device_id IS NULL;
CREATE INDEX reading_progress_subject ON reading_progress (subject_type, subject_id);

CREATE TABLE rating (
    id           TEXT    PRIMARY KEY,
    account_id   TEXT    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    pseud_id     TEXT    NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    work_id      TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    stars        INTEGER NOT NULL CHECK (stars BETWEEN 1 AND 5),
    is_public    INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT    NOT NULL,
    updated_at   TEXT    NOT NULL,
    version      INTEGER NOT NULL DEFAULT 1,
    deleted_at   TEXT
);

CREATE UNIQUE INDEX rating_pseud_work ON rating (pseud_id, work_id) WHERE deleted_at IS NULL;
CREATE INDEX rating_work_public ON rating (work_id) WHERE is_public = 1 AND deleted_at IS NULL;

CREATE TABLE review (
    id               TEXT    PRIMARY KEY,
    account_id       TEXT    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    pseud_id         TEXT    NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    work_id          TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    body             TEXT    NOT NULL,
    contains_spoilers INTEGER NOT NULL DEFAULT 0,
    is_public        INTEGER NOT NULL DEFAULT 0,
    published_at     TEXT,
    created_at       TEXT    NOT NULL,
    updated_at       TEXT    NOT NULL,
    version          INTEGER NOT NULL DEFAULT 1,
    deleted_at       TEXT
);

CREATE UNIQUE INDEX review_pseud_work ON review (pseud_id, work_id) WHERE deleted_at IS NULL;
CREATE INDEX review_work_public ON review (work_id) WHERE is_public = 1 AND published_at IS NOT NULL AND deleted_at IS NULL;

CREATE TABLE reading_history_entry (
    id             TEXT    PRIMARY KEY,
    account_id     TEXT    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    pseud_id       TEXT    NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    subject_type   TEXT    NOT NULL,
    subject_id     TEXT    NOT NULL,
    last_read_at   TEXT    NOT NULL,
    revision_seen  TEXT,
    created_at     TEXT    NOT NULL
);

CREATE UNIQUE INDEX reading_history_unique
    ON reading_history_entry (account_id, pseud_id, subject_type, subject_id);
CREATE INDEX reading_history_account ON reading_history_entry (account_id, last_read_at DESC);

CREATE TABLE reader_note (
    id           TEXT    PRIMARY KEY,
    account_id   TEXT    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    pseud_id     TEXT    NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    subject_type TEXT    NOT NULL,
    subject_id   TEXT    NOT NULL,
    anchor       TEXT,
    body         TEXT    NOT NULL,
    created_at   TEXT    NOT NULL,
    updated_at   TEXT    NOT NULL,
    version      INTEGER NOT NULL DEFAULT 1,
    deleted_at   TEXT
);

CREATE INDEX reader_note_subject ON reader_note (pseud_id, subject_type, subject_id) WHERE deleted_at IS NULL;

CREATE TABLE typography_preference (
    account_id        TEXT    PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    font_scale        REAL    NOT NULL DEFAULT 1.0,
    line_height       REAL    NOT NULL DEFAULT 1.6,
    measure           INTEGER NOT NULL DEFAULT 66,
    reader_theme      TEXT    NOT NULL DEFAULT 'sepia',
    distraction_free  INTEGER NOT NULL DEFAULT 0,
    created_at        TEXT    NOT NULL,
    updated_at        TEXT    NOT NULL,
    version           INTEGER NOT NULL DEFAULT 1
);
