-- 0057_flexible_bounties: add type and funding columns to bounties (spec §20.3.2, M18 Phase 4.1).
--
-- Dialect: SQLite.
--
-- Adds support for crowdfunded and reverse bounty types. Standard bounties
-- work as before; crowdfunded bounties activate when funded_amount >= amount
-- * threshold; reverse bounties are prepaid by the creator.

ALTER TABLE bounties ADD COLUMN type TEXT NOT NULL DEFAULT 'standard';
ALTER TABLE bounties ADD COLUMN funded_amount INTEGER NOT NULL DEFAULT 0;
ALTER TABLE bounties ADD COLUMN activated_at TEXT;

-- Audit trail for crowdfunded bounty contributions.
CREATE TABLE IF NOT EXISTS bounty_contributions (
    id           TEXT    PRIMARY KEY,
    bounty_id    TEXT    NOT NULL REFERENCES bounties (id) ON DELETE CASCADE,
    contributor  TEXT    NOT NULL,
    amount       INTEGER NOT NULL,
    contributed_at TEXT  NOT NULL
);
