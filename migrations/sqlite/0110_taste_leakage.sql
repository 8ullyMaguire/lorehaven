-- §52.2 and §52.3 — taste leakage, made structural.
--
-- §52 is mostly about how an answer is surfaced, and prose in a spec cannot
-- enforce a batch interval. These tables make the two load-bearing rules
-- impossible to write a row against by accident:
--
--   * a payout carries a BATCH WINDOW, not a rating, and is attributed to the
--     instance (§52.2). The absence of a rating column is the enforcement.
--   * the owner-visible resonance label is computed BY A BATCH, and a label row
--     records the batch it came from (§52.3).
--
-- Not to be confused with `payouts` from §20.9.3, which is a real-money payment
-- through a processor and is cashable. These are closed-loop credit batches and
-- never touch money -- M45-18's dashboard is the row that owns the closed-loop
-- guarantee.

-- ── §52.2 Batched, instance-attributed payouts ────────────────────────────────

-- A batch window. Payouts are attributed to a window rather than to an event, and
-- a window that has not closed is not yet something a payout can be attributed
-- to -- see `taste_leakage_payout_is_inside_its_window`.
CREATE TABLE IF NOT EXISTS taste_leakage_batch_windows (
    id            TEXT PRIMARY KEY,
    opened_at     TEXT NOT NULL,
    -- NULL while open. §52.2: a payment made with no closed window to attribute
    -- it to is precisely the correlation this table family exists to prevent, so
    -- the store refuses to record one.
    closed_at     TEXT,
    -- A window that closes before it opened is a typo rather than a policy, and
    -- without this the interval is inverted and every payout in it is nonsense.
    CHECK (closed_at IS NULL OR closed_at >= opened_at)
);
CREATE INDEX IF NOT EXISTS idx_taste_leakage_windows_open
    ON taste_leakage_batch_windows (closed_at);

-- The payout itself.
--
-- Deliberately NO rating_id, no rater, no event reference of any kind. §52.2: a
-- payout saying "you were rated highly" makes one rating action at one moment
-- observable, which is a measurement of the operator's lens. The columns below
-- are the whole set an instance-attributed payout needs.
CREATE TABLE IF NOT EXISTS taste_leakage_payouts (
    id             TEXT PRIMARY KEY,
    recipient_pseud_id TEXT NOT NULL,
    work_id        TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    credits        INTEGER NOT NULL CHECK (credits > 0),
    window_id      TEXT NOT NULL REFERENCES taste_leakage_batch_windows (id) ON DELETE RESTRICT,
    attribution    TEXT NOT NULL CHECK (attribution = 'instance'),
    paid_at        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_taste_leakage_payouts_recipient
    ON taste_leakage_payouts (recipient_pseud_id, paid_at);
CREATE INDEX IF NOT EXISTS idx_taste_leakage_payouts_window
    ON taste_leakage_payouts (window_id);

-- §52.2's timing rule, as a constraint rather than a convention.
--
-- `paid_at` must fall inside the window named by `window_id`. This is the part
-- that is easy to miss and the part that matters: a payout timestamp seconds from
-- a rating timestamp is a usable correlation EVEN WITH NO PAYLOAD DETAIL, so the
-- batch interval bounds what an observer can infer about *when*, not only about
-- *whether*.
--
-- SQLite cannot reference another table from a CHECK, so this is a trigger --
-- and note the linked SQLite here is 3.46.0, which predates
-- `ALTER TABLE ... ADD CONSTRAINT` (3.47.0) entirely. See 0109 for the full
-- version story and why the trigger idiom is used rather than the constraint.
CREATE TRIGGER IF NOT EXISTS taste_leakage_payout_inside_window_insert
BEFORE INSERT ON taste_leakage_payouts
FOR EACH ROW
WHEN (
    NOT EXISTS (
        SELECT 1 FROM taste_leakage_batch_windows w
        WHERE w.id = NEW.window_id
          AND w.closed_at IS NOT NULL
          AND NEW.paid_at >= w.opened_at
          AND NEW.paid_at <= w.closed_at
    )
)
BEGIN
    SELECT RAISE(ABORT, 'a payout must be attributed to a closed window and fall inside it');
END;

CREATE TRIGGER IF NOT EXISTS taste_leakage_payout_inside_window_update
BEFORE UPDATE OF window_id, paid_at ON taste_leakage_payouts
FOR EACH ROW
WHEN (
    NOT EXISTS (
        SELECT 1 FROM taste_leakage_batch_windows w
        WHERE w.id = NEW.window_id
          AND w.closed_at IS NOT NULL
          AND NEW.paid_at >= w.opened_at
          AND NEW.paid_at <= w.closed_at
    )
)
BEGIN
    SELECT RAISE(ABORT, 'a payout must be attributed to a closed window and fall inside it');
END;

-- §52.2: a payout answers a supply question, so an author can be paid without any
-- payout artifact existing at their pseud. This makes that structural rather than
-- a promise -- one recipient may hold no rows here at all, and that is fine.
--
-- What is NOT permitted is the reverse: two windows claiming to be open at once
-- would let a payout be attributed to whichever the reader picked.

CREATE TRIGGER IF NOT EXISTS taste_leakage_single_open_window_insert
BEFORE INSERT ON taste_leakage_batch_windows
FOR EACH ROW
WHEN (
    NEW.closed_at IS NULL
    AND EXISTS (
        SELECT 1 FROM taste_leakage_batch_windows w WHERE w.closed_at IS NULL
    )
)
BEGIN
    SELECT RAISE(ABORT, 'at most one batch window may be open');
END;

-- ── §52.1 The reviewed rows themselves ─────────────────────────────────────

-- What an operator reviewed, and what they decided.
--
-- `inferable` is TEXT and holds prose, and `reviewed_by` names the operator
-- because a review with no reviewer is nobody's decision. There is no
-- confidence column, no score, and no dimension: §52.1's row is an artifact plus
-- an inference a person could state out loud, and a column for "how sure" would
-- be the first step back toward the measurement §0.3 forbids.
--
-- `disposition` defaults to `keep` because §52.1's default is to keep, and a
-- review inserted without saying what to do is a review that found nothing wrong.
CREATE TABLE IF NOT EXISTS taste_leakage_reviews (
    id            TEXT PRIMARY KEY,
    artifact      TEXT NOT NULL,
    inferable     TEXT NOT NULL,
    -- plain | derived | measured. §52.1's ease ladder: what an observant reader
    -- could put together without help, what needs a comparison across artifacts,
    -- and what needs measuring (where coarsening is the only real remedy).
    ease          TEXT NOT NULL CHECK (ease IN ('plain', 'derived', 'measured')),
    disposition   TEXT NOT NULL DEFAULT 'keep'
                    CHECK (disposition IN ('keep', 'coarsen', 'remove')),
    reviewed_by   TEXT NOT NULL,
    reviewed_at   TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_taste_leakage_reviews_disposition
    ON taste_leakage_reviews (disposition);

-- ── §52.3 The owner-visible resonance label ─────────────────────────────────

-- §52.3: the label updates from a weekly batch, never on read or on the
-- underlying event. The reason is the threat model rather than taste -- an author
-- who watches a label tick after each event can infer the lens from the tick
-- pattern, and on a shared instance "watch it tick" is something any reader can
-- do.
--
-- A label row therefore carries the BATCH that produced it. A label written
-- without one would be exactly the per-event update this rule forbids, and there
-- is no way to tell afterwards.
CREATE TABLE IF NOT EXISTS taste_leakage_resonance_labels (
    work_id        TEXT PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    owner_pseud_id TEXT NOT NULL,
    -- One of quiet | steady | noticed | landing. Coarse on purpose; §52.3's
    -- "not enough to rank" means the column stores no number at all, so there is
    -- no precision to leak and no float to round.
    label          TEXT NOT NULL
                    CHECK (label IN ('quiet', 'steady', 'noticed', 'landing')),
    computed_at    TEXT NOT NULL,
    -- The batch this reading came from. NOT NULL and a real foreign key: a label
    -- with no batch is a per-event update wearing a batch's name.
    batch_window_id TEXT NOT NULL
                    REFERENCES taste_leakage_batch_windows (id) ON DELETE RESTRICT
);
CREATE INDEX IF NOT EXISTS idx_taste_leakage_labels_owner
    ON taste_leakage_resonance_labels (owner_pseud_id);

-- A closed window may not be edited after the fact, because a payout or a label
-- written inside it is attributed to the interval it describes. Editing the end
-- of an interval retroactively widens what every row in it claims to represent.
CREATE TRIGGER IF NOT EXISTS taste_leakage_window_closed_is_immutable
BEFORE UPDATE OF closed_at ON taste_leakage_batch_windows
FOR EACH ROW
WHEN (OLD.closed_at IS NOT NULL)
BEGIN
    SELECT RAISE(ABORT, 'a closed batch window cannot be re-closed');
END;