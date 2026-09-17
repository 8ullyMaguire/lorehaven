-- 0034_bounties: add account and fulfillment columns to existing bounties table (spec §20, M16).
--
-- Dialect: PostgreSQL. Timestamps are RFC 3339 TEXT, matching the SQLite twin.

ALTER TABLE bounties ADD COLUMN account TEXT;
ALTER TABLE bounties ADD COLUMN amount INTEGER DEFAULT 0;
ALTER TABLE bounties ADD COLUMN fulfilled_at TEXT;
ALTER TABLE bounties ADD COLUMN fulfilled_by TEXT;

CREATE INDEX IF NOT EXISTS idx_bounties_account ON bounties(account, state);
CREATE INDEX IF NOT EXISTS idx_bounties_state ON bounties(state);