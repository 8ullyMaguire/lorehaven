-- Migration 0098 — the ranking substrate's log columns (spec §47).
--
-- Dialect: SQLite.
--
-- ALTER, NOT CREATE. The obvious design — a new `impressions` table and a new
-- `interactions` table — duplicates two tables that already exist and already
-- record most of this:
--
--   * `recommendation_slots` (migration 0076, spec §33.3) already logs every
--     served slot with its position, its reasons, its recipe stage and its blend
--     score. It lacks exactly one thing: the probability the reader would have
--     been shown that work.
--   * `work_view_log` and `work_kudos` (migration 0068, spec §9.4/§9.5) already
--     record reads and kudos. They lack the earned/incentivized distinction and
--     the obscurity measured at read time.
--
-- A second copy of each would mean two tables disagreeing about who read what,
-- with nothing keeping them in step. So this migration adds the missing columns
-- to the tables that are already the record of truth.

ALTER TABLE recommendation_slots
    ADD COLUMN slot_kind TEXT NOT NULL DEFAULT 'ranked'
        CHECK (slot_kind IN ('ranked', 'exploration', 'exposure_floor'));

-- NULLABLE, and that is the honest encoding of §47.3.
--
-- `propensity` is the probability the reader would have been shown this work, and
-- inverse-propensity scoring (M45-13) divides by it. A row written before this
-- migration has no such probability and there is no way to recover one: the
-- counterfactual that would have been logged no longer exists. So historical rows
-- are NULL rather than defaulted to 1.0 or to some plausible constant.
--
-- Defaulting them would be the quiet version of the same defect this migration
-- exists to close — a value that looks usable and is not, silently biasing every
-- offline estimate computed from it. NULL is distinguishable; 1.0 is not.
ALTER TABLE recommendation_slots
    ADD COLUMN propensity REAL
        CHECK (propensity IS NULL OR (propensity > 0 AND propensity <= 1));

-- `kind` defaults to 'earned' for historical rows, and that is correct rather
-- than convenient: no incentive existed before this migration, so every recorded
-- interaction genuinely was earned. The default is safe here and unsafe in
-- `InteractionKind::for_source`, which defaults the other way — see
-- crates/db/src/ranking.rs for why the two differ.
ALTER TABLE work_view_log
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'earned'
        CHECK (kind IN ('earned', 'incentivized'));
ALTER TABLE work_view_log
    ADD COLUMN source TEXT NOT NULL DEFAULT 'unknown';
-- 1.0 means "not obscure", so historical rows earn NO scout value (§47.7). The
-- conservative direction again: under-crediting a past engagement loses a
-- payout, over-crediting one mints credit that was never earned.
ALTER TABLE work_view_log
    ADD COLUMN obscurity_at_read REAL NOT NULL DEFAULT 1.0;

ALTER TABLE work_kudos
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'earned'
        CHECK (kind IN ('earned', 'incentivized'));
ALTER TABLE work_kudos
    ADD COLUMN source TEXT NOT NULL DEFAULT 'unknown';
ALTER TABLE work_kudos
    ADD COLUMN obscurity_at_read REAL NOT NULL DEFAULT 1.0;

-- Ranking reads only earned interactions (§47.4). A partial index keeps that
-- filter off the hot path instead of leaving it to every query that forgets.
CREATE INDEX work_view_log_earned_work
    ON work_view_log (work_id) WHERE kind = 'earned';
CREATE INDEX work_kudos_earned_work
    ON work_kudos (work_id) WHERE kind = 'earned';
CREATE INDEX recommendation_slots_propensity
    ON recommendation_slots (pseud_id, created_at) WHERE propensity IS NOT NULL;
