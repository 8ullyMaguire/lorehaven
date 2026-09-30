-- Migration 0098 — the ranking substrate's log columns (spec §47).
--
-- Dialect: PostgreSQL.
-- Identifiers are native UUID columns; timestamps are RFC 3339 UTC text
-- (ADR 0004), matching migrations/postgres/0001_core.sql.
--
-- ALTER, NOT CREATE, for the reason given in the SQLite counterpart: the tables
-- that already record served slots, reads and kudos are
-- `recommendation_slots` (0076), `work_view_log` and `work_kudos` (0068). Adding
-- columns to them keeps one record of who read what; a second copy of each would
-- mean two tables disagreeing with nothing keeping them in step.
--
-- Two dialect differences from the SQLite file, both deliberate:
--
--   * `propensity` is `DOUBLE PRECISION`, not `REAL`. PostgreSQL's `REAL` is a
--     4-byte float; a probability that has been through several multiplications
--     needs the 8-byte form, and a `REAL` here would lose precision that the
--     inverse-propensity correction is sensitive to.
--   * `recommendation_slots.id` is UUID, so the row count here and on SQLite
--     match without the TEXT round-trip the query layer already does elsewhere.

ALTER TABLE recommendation_slots
    ADD COLUMN slot_kind TEXT NOT NULL DEFAULT 'ranked'
        CHECK (slot_kind IN ('ranked', 'exploration', 'exposure_floor'));

-- NULLABLE. A row written before this migration has no selection probability and
-- it cannot be reconstructed: the counterfactual is gone. NULL is the honest
-- encoding, and it is distinguishable from a real value in a way 1.0 is not.
ALTER TABLE recommendation_slots
    ADD COLUMN propensity DOUBLE PRECISION
        CHECK (propensity IS NULL OR (propensity > 0 AND propensity <= 1));

-- 'earned' is correct for historical rows rather than merely convenient: no
-- incentive existed before this migration, so every recorded interaction really
-- was earned. See crates/db/src/ranking.rs for why `InteractionKind::for_source`
-- defaults the OTHER way and this one does not.
ALTER TABLE work_view_log
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'earned'
        CHECK (kind IN ('earned', 'incentivized'));
ALTER TABLE work_view_log
    ADD COLUMN source TEXT NOT NULL DEFAULT 'unknown';
-- 1.0 = "not obscure", so historical rows earn no scout value (§47.7).
ALTER TABLE work_view_log
    ADD COLUMN obscurity_at_read DOUBLE PRECISION NOT NULL DEFAULT 1.0;

ALTER TABLE work_kudos
    ADD COLUMN kind TEXT NOT NULL DEFAULT 'earned'
        CHECK (kind IN ('earned', 'incentivized'));
ALTER TABLE work_kudos
    ADD COLUMN source TEXT NOT NULL DEFAULT 'unknown';
ALTER TABLE work_kudos
    ADD COLUMN obscurity_at_read DOUBLE PRECISION NOT NULL DEFAULT 1.0;

CREATE INDEX work_view_log_earned_work
    ON work_view_log (work_id) WHERE kind = 'earned';
CREATE INDEX work_kudos_earned_work
    ON work_kudos (work_id) WHERE kind = 'earned';
CREATE INDEX recommendation_slots_propensity
    ON recommendation_slots (pseud_id, created_at) WHERE propensity IS NOT NULL;
