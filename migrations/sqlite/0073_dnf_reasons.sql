-- M45-21: Structured DNF (did-not-finish) reasons, private by default.
--
-- A reader can mark a work they started as abandoned with a structured
-- reason. The mark is private by default; the author only sees an aggregate
-- when the work setting `allow_dnf_feedback` is enabled.
--
-- Dialect: SQLite.
CREATE TABLE did_not_finish (
    id           TEXT    PRIMARY KEY,
    account_id   TEXT    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    pseud_id     TEXT    NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    work_id      TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    reason       TEXT    NOT NULL CHECK (reason IN (
                     'not_my_taste', 'triggering', 'slow_pacing',
                     'abandoned_by_author', 'dropped_other', 'other'
                   )),
    note         TEXT,
    is_public    INTEGER NOT NULL DEFAULT 0,
    created_at   TEXT    NOT NULL,
    updated_at   TEXT    NOT NULL,
    deleted_at   TEXT
);

CREATE UNIQUE INDEX dnf_pseud_work
    ON did_not_finish (pseud_id, work_id) WHERE deleted_at IS NULL;
CREATE INDEX dnf_work_public
    ON did_not_finish (work_id) WHERE is_public = 1 AND deleted_at IS NULL;

ALTER TABLE works ADD COLUMN allow_dnf_feedback INTEGER NOT NULL DEFAULT 0;
