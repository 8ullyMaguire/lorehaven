-- Migration 0100 — the selector's uncertainty on the sample it drew
-- (spec §49.5, M45-19).
--
-- Dialect: SQLite.
--
-- Why this is a migration and not a field the route sends back
-- -----------------------------------------------------------
-- §49.5 makes "chosen by uncertainty" an acceptance criterion, and a criterion
-- needs a number to check. `tasting_responses.uncertainty_at_draw` (0099)
-- records the number, but only once the reader has answered — and the two facts
-- are not the same moment:
--
--   * the queue draws a sample and *knows* its uncertainty at that instant;
--   * the reader answers some seconds later, and by then the weights may have
--     moved, so re-deriving the number would report the selector's opinion
--     after the fact rather than at the time it acted.
--
-- A client-supplied number would be worse than either: the client is the thing
-- being checked. So the number is written where it was computed, on the sample,
-- and the response inherits it at insert time.

ALTER TABLE tasting_samples
    ADD COLUMN uncertainty_at_draw REAL;

-- Why this column, and why it is not a subquery in an index predicate
-- ------------------------------------------------------------------
-- §49.5 bounds sampling per session, and `build_queue` refuses a second sample
-- for a (work, reader) pair -- but that refusal needs a UNIQUE constraint to
-- bite. `INSERT OR IGNORE` with no constraint behind it silently inserts a
-- duplicate, and the reader is offered the same work twice in one session, which
-- is precisely the failure the session bound exists to prevent.
--
-- The first draft of this migration wrote the predicate as a subquery:
--
--     WHERE id NOT IN (SELECT sample_id FROM tasting_responses)
--
-- That is **not valid on either engine**. A partial index predicate must be an
-- expression over the row's own columns; a subquery is rejected by SQLite
-- outright and is not a permitted predicate form in PostgreSQL either. So the
-- predicate is carried by a column instead.
--
-- `answered_at IS NULL` is the honest spelling of "this reader has not answered
-- this sample yet", and it is the same fact `tasting_responses` records — held on
-- the sample as well so the constraint can be expressed at all. It is written by
-- the response path in the same statement that inserts the response, so the two
-- cannot drift: a response without an `answered_at` on its sample, or an
-- `answered_at` with no response, both mean the invariant was broken by a code
-- path that skipped this column, which is why `a_response_and_its_samples_answer
-- _flag_never_disagree` is a test rather than a comment.
--
-- Partial on purpose: answering a card frees the work to be sampled *again* in a
-- later session. Re-calibration on a work whose opinion has changed is
-- legitimate, and freezing it after five seconds of attention would make a
-- reader's first opinion permanent.
ALTER TABLE tasting_samples
    ADD COLUMN answered_at TEXT;

CREATE UNIQUE INDEX IF NOT EXISTS idx_tasting_samples_one_open_per_work
    ON tasting_samples (account_id, work_id)
    WHERE answered_at IS NULL;
