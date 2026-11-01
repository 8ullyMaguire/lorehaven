-- 0134 — §59.6 per-user module state (M65-04).
--
-- The PostgreSQL twin carries the reasoning; this file is kept in step statement
-- for statement so the parity test can compare the two declared column sets.
--
-- One row per account per module a reader has ACTUALLY CHANGED. No row means
-- inherit: the instance's decision is the answer, and a table of "yes" rows for
-- every account would make "what did this reader change" unanswerable.

CREATE TABLE IF NOT EXISTS module_user_state (
    account_id TEXT    NOT NULL,
    module_id  TEXT    NOT NULL,
    enabled    INTEGER NOT NULL CHECK (enabled IN (0, 1)),
    updated_at TEXT    NOT NULL,
    PRIMARY KEY (account_id, module_id)
);

CREATE INDEX IF NOT EXISTS idx_module_user_state_module
    ON module_user_state (module_id);
