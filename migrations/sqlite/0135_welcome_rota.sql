-- 0135 — §58.4 the welcome rota (M45-26).
--
-- The PostgreSQL twin carries the reasoning; this file is kept in step statement
-- for statement so the parity test can compare the two declared column sets.
--
-- The promise is made to a PSEUD, not an account: a comment carries
-- `comments.author_pseud`, so only a pseud can keep the promise. A session with
-- no active pseud therefore cannot take a welcome.

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

-- The reader's own rows, open first: every reader door filters on pseud_id and
-- `completed_at IS NULL`, so this is the index the surface actually uses.
CREATE INDEX IF NOT EXISTS idx_welcome_rota_open
    ON welcome_rota (pseud_id, completed_at);

-- "Has this work already been offered?" is asked once per candidate, so the
-- work id needs its own index or the candidate scan becomes quadratic.
CREATE INDEX IF NOT EXISTS idx_welcome_rota_author
    ON welcome_rota (author_key);

CREATE INDEX IF NOT EXISTS idx_welcome_rota_work
    ON welcome_rota (work_id);
