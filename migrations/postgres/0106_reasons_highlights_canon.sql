-- Migration 0106 — reason-tagged kudos, line-level highlights, and the canon-blind
-- flag (spec §50.1, §50.2; M45-24, M45-31).
--
-- Dialect: PostgreSQL.
--
-- Same three tables as the SQLite arm, same order, same columns and same
-- constraints, so the parity test (the_two_dialects_declare_the_same_columns_and
-- indexes) passes. The parity test compares declared column SETS and index column
-- lists and is blind to column TYPES, so every type difference below is deliberate.
--
-- The type differences are the ones this migration needs:
--
--   * `work_id` and `account_id` are UUID here and TEXT in SQLite, because
--     `works.id` and `accounts.id` are UUID here. Writing them as TEXT would
--     create a foreign key that PostgreSQL accepts and that never matches a row.
--   * `start_offset`/`end_offset` are BIGINT against INTEGER, matching 0102's own
--     convention for a character offset into a work's text.
--   * `text_version` is BIGINT against INTEGER, for the same reason and because
--     `work_coordinates.version` is BIGINT -- a per-work text length can exceed a
--     32-bit count on a very long serial.
--
-- Everything else -- every reason set, every CHECK, every index -- is identical to
-- the SQLite arm on purpose. §50.3 requires that the same input produce the same
-- behaviour on both engines, and a CHECK that differs by dialect is the easiest
-- place for that to stop being true.

ALTER TABLE work_kudos ADD COLUMN reason TEXT
    CONSTRAINT work_kudos_reason_check
    CHECK (reason IS NULL OR reason IN (
        'prose', 'characters', 'pacing', 'trope_execution',
        'worldbuilding', 'not_for_me'
    ));

ALTER TABLE work_kudos ADD COLUMN note TEXT;

CREATE TABLE work_highlights (
    id            UUID    PRIMARY KEY,
    work_id       UUID    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    account_id    UUID    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    start_offset  BIGINT  NOT NULL CHECK (start_offset >= 0),
    end_offset    BIGINT  NOT NULL CHECK (end_offset > start_offset),
    reason        TEXT    NOT NULL
                          CHECK (reason IN (
                              'prose', 'characters', 'pacing',
                              'trope_execution', 'worldbuilding', 'not_for_me'
                          )),
    note          TEXT,
    created_at    TEXT    NOT NULL
);

CREATE INDEX idx_work_highlights_work ON work_highlights (work_id);
CREATE INDEX idx_work_highlights_account ON work_highlights (account_id);
CREATE UNIQUE INDEX idx_work_highlights_span ON work_highlights (work_id, account_id, start_offset, end_offset);

CREATE TABLE canon_agnostic_works (
    work_id      UUID    PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    text_version BIGINT  NOT NULL,
    declared_at  TEXT    NOT NULL
);
