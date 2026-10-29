-- 0130 — two-phase sanctions (capability ledger 3.4, ported from gravity's
-- /discipline/permanent-ban + /permanent-ban/execute pair).
--
-- See the sqlite twin for why this is a state on the row rather than a second table, and why
-- the default is 'active' (backfilling 'pending' would lift every live sanction on deploy).
ALTER TABLE sanctions ADD COLUMN IF NOT EXISTS state TEXT NOT NULL DEFAULT 'active';

CREATE INDEX IF NOT EXISTS idx_sanctions_state ON sanctions(state);
