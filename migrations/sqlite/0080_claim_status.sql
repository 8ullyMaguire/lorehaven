-- M18-05: A claim needs a state, not two nullable columns (spec §18).
--
-- `claims` had no state at all: `fulfilled_by_work` and `fulfilled_at`, both
-- nullable, were the whole vocabulary. That cannot express "this claim was
-- claimed and then expired" separately from "this claim was claimed and is
-- still outstanding", because expiring a claim had to write `fulfilled_at =
-- NULL` — and `fulfilled_at IS NULL` is precisely the predicate the expiry
-- statement filters on. So an expired claim became indistinguishable from an
-- unfulfilled one, and `expire_claims` re-matched the same row on every
-- subsequent run. The job that calls it runs on a schedule, so its
-- `rows_affected` count was a growing tally of every claim that had ever gone
-- stale rather than the work done by that run. It never fell to zero for a
-- request nobody ever fulfilled.
--
-- The fix is a state that names what happened. `status` is the discriminator;
-- the two nullable columns stay because they still carry the data (which work
-- fulfilled it, when), and a status of 'fulfilled' with a NULL work would be a
-- row that claims to have been fulfilled by nothing.
--
-- Backfill is the part that matters. Pre-existing rows are classified rather
-- than assumed: a row with a work id was fulfilled, a row without one was
-- outstanding, and no row is 'expired' — an expired claim was
-- indistinguishable from an outstanding one before this migration, so calling
-- them expired would be a guess, and the one honest thing to do is let the
-- first post-migration run classify them. This costs at most one extra expiry
-- pass, and a wrong classification would cost far more: an 'expired' claim that
-- was actually outstanding would silently disappear from the claimant's view.
--
-- Dialect: SQLite.

ALTER TABLE claims ADD COLUMN status TEXT NOT NULL DEFAULT 'outstanding';

UPDATE claims
   SET status = 'fulfilled'
 WHERE fulfilled_at IS NOT NULL OR fulfilled_by_work IS NOT NULL;

-- The expiry predicate now keys on the state rather than on the absence of a
-- fulfilment, so an expired claim stops matching its own expiry statement.
CREATE INDEX claims_pending_expiry ON claims (claimed_at)
    WHERE status = 'outstanding';
