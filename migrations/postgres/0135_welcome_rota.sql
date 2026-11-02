-- 0135 — §58.4 the welcome rota (M45-26).
--
-- Why a table and not a computed view: a welcome is a PROMISE with a deadline.
-- "Handed to this reader at 14:02, due at 14:02 in 72 hours" is not derivable
-- from anything else in the schema — it is a fact about an obligation, and the
-- only record of it is this row.
--
-- No foreign keys, for the reason supply_routing states: the row is evidence
-- that a welcome was owed. A cascade would delete the evidence along with the
-- account. Erasure coverage for the pseud column belongs to §51's cascade list
-- (M45-51), and `pseud_id` is a pseudonym rather than an account identifier.
--
-- Timestamps are canonical `YYYY-MM-DDTHH:MM:SSZ` (M63-31, migration 0132): the
-- window is a plain indexed range, so do not reintroduce a normalising function
-- in a predicate. `due_at` is stored rather than computed from `assigned_at` so
-- a later change to the promise cannot silently move a promise already made.

CREATE TABLE IF NOT EXISTS welcome_rota (
    id           TEXT    PRIMARY KEY,
    work_id      TEXT    NOT NULL,
    pseud_id     TEXT    NOT NULL,
    author_key   TEXT    NOT NULL DEFAULT '',
    assigned_at  TEXT    NOT NULL,
    due_at       TEXT    NOT NULL,
    completed_at TEXT,
    comment_id   TEXT
);

-- Every reader door filters on (pseud_id, completed_at IS NULL); this is the
-- index those queries actually use.
CREATE INDEX IF NOT EXISTS idx_welcome_rota_open
    ON welcome_rota (pseud_id, completed_at);

-- One author gets one welcome, so the author key is looked up on every claim.
CREATE INDEX IF NOT EXISTS idx_welcome_rota_author
    ON welcome_rota (author_key);

-- "Has this work already been offered?" is asked once per candidate.
CREATE INDEX IF NOT EXISTS idx_welcome_rota_work
    ON welcome_rota (work_id);
