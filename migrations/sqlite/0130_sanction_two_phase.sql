-- 0130 — two-phase sanctions (capability ledger 3.4, ported from gravity's
-- /discipline/permanent-ban + /permanent-ban/execute pair).
--
-- WHY A STATE COLUMN AND NOT A SECOND TABLE. A filed sanction and an executed one are the
-- same object at two moments, and a separate `sanction_cases` table would let the two drift
-- (a case executed with no row, a row with no case). The state is also what every reader has
-- to consult: a pending sanction that bites the moment it is filed is worse than no two-phase
-- at all, because it looks like the safeguard works.
--
-- DEFAULT 'active' is deliberate and it is the migration's one judgement call: every existing
-- row was applied when it was written, and backfilling them as 'pending' would silently lift
-- every sanction on the instance at the next deploy.
ALTER TABLE sanctions ADD COLUMN state TEXT NOT NULL DEFAULT 'active';

CREATE INDEX IF NOT EXISTS idx_sanctions_state ON sanctions(state);
