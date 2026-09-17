-- 0029_lending: controlled digital lending tables (spec §32.4, M25).
--
-- Dialect: PostgreSQL. Timestamps are TEXT; ids are UUID; booleans BOOLEAN.

CREATE TABLE IF NOT EXISTS work_loans (
    id                  UUID PRIMARY KEY,
    work_id             UUID NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    borrower_account_id UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    granted_at          TEXT NOT NULL,
    expires_at          TEXT NOT NULL,
    revoked_at          TEXT,
    copy_number         BIGINT NOT NULL,
    created_at          TEXT NOT NULL,
    updated_at          TEXT NOT NULL,
    version             BIGINT NOT NULL DEFAULT 1,
    UNIQUE (work_id, borrower_account_id)
);

CREATE INDEX IF NOT EXISTS idx_work_loans_work ON work_loans(work_id);
CREATE INDEX IF NOT EXISTS idx_work_loans_borrower ON work_loans(borrower_account_id);
CREATE INDEX IF NOT EXISTS idx_work_loans_expires ON work_loans(expires_at);

CREATE TABLE IF NOT EXISTS lending_config (
    singleton           INTEGER PRIMARY KEY DEFAULT 1 CHECK (singleton = 1),
    enabled             BOOLEAN NOT NULL DEFAULT FALSE,
    copies_per_work     BIGINT NOT NULL DEFAULT 1,
    loan_duration_days  BIGINT NOT NULL DEFAULT 14,
    borrower_max_loans  BIGINT NOT NULL DEFAULT 5,
    updated_at          TEXT NOT NULL,
    version             BIGINT NOT NULL DEFAULT 1
);
