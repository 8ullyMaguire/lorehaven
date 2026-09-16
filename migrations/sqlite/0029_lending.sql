-- 0029_lending: controlled digital lending tables (spec §32.4, M25).
--
-- Dialect: SQLite. Timestamps are RFC 3339 UTC text; ids are canonical UUID text.
--
-- An instance may opt into lending. When off, rights are still served but loan
-- requests are refused. When on, a work with lending_class = 'lending' may be
-- loaned to one reader at a time. A loan grants a bounded window, may expire,
-- may be revoked, and never multiplies copies beyond the configured cap.

CREATE TABLE IF NOT EXISTS work_loans (
    id                  TEXT PRIMARY KEY,
    work_id             TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    borrower_account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    granted_at          TEXT NOT NULL,
    expires_at          TEXT NOT NULL,
    revoked_at          TEXT,
    copy_number         INTEGER NOT NULL,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,
    version             INTEGER NOT NULL DEFAULT 1,
    UNIQUE (work_id, borrower_account_id)
);

CREATE INDEX IF NOT EXISTS idx_work_loans_work ON work_loans(work_id);
CREATE INDEX IF NOT EXISTS idx_work_loans_borrower ON work_loans(borrower_account_id);
CREATE INDEX IF NOT EXISTS idx_work_loans_expires ON work_loans(expires_at);

-- Instance-wide lending configuration.
CREATE TABLE IF NOT EXISTS lending_config (
    singleton           INTEGER PRIMARY KEY DEFAULT 1 CHECK (singleton = 1),
    enabled             INTEGER NOT NULL DEFAULT 0,
    copies_per_work     INTEGER NOT NULL DEFAULT 1,
    loan_duration_days  INTEGER NOT NULL DEFAULT 14,
    borrower_max_loans  INTEGER NOT NULL DEFAULT 5,
    updated_at          TEXT NOT NULL,
    version             INTEGER NOT NULL DEFAULT 1
);
