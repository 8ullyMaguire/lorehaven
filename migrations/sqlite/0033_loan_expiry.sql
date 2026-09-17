-- 0033_loan_expiry: record when a loan's window closed (spec §32.4, M25).
--
-- Dialect: SQLite. Timestamps are RFC 3339 UTC text.
--
-- Active-ness was already derived from `expires_at > now`, so the copy slot
-- frees itself without a sweep. What this column adds is the *transition*: the
-- periodic pass stamps the moment it noticed, which gives the borrower's loan
-- list a state to report, gives the operator's sweep something to count, and
-- lets a re-grant clear it — none of which a comparison against the clock can
-- express, because a comparison has no history.

ALTER TABLE work_loans ADD COLUMN expired_at TEXT;

CREATE INDEX IF NOT EXISTS idx_work_loans_expired ON work_loans(expired_at);
