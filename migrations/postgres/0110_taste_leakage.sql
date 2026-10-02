-- §52.2 and §52.3 — taste leakage, made structural.
--
-- PostgreSQL twin of migrations/sqlite/0110_taste_leakage.sql. The rules and the
-- reasoning are the same; the ENFORCEMENT is not, and that difference is the
-- whole reason this file exists separately:
--
--   * SQLite needs TRIGGERS for the §52.2 timing rule, because it cannot
--     reference another table from a CHECK. PostgreSQL has FOREIGN KEYS and
--     CHECK constraints, so the same rules are declarative here.
--   * What neither engine can do is a CHECK that compares a column against
--     another TABLE's column, so `taste_leakage_payout_is_inside_its_window` is
--     a trigger on both sides. §52.2's timing rule is the one that does not
--     port to a constraint, and it is also the one that matters.
--
-- Not to be confused with `payouts` from §20.9.3, which is a real-money payment
-- through a processor and is cashable. These are closed-loop credit batches.

-- ── §52.2 Batched, instance-attributed payouts ────────────────────────────────

CREATE TABLE IF NOT EXISTS taste_leakage_batch_windows (
    id            UUID PRIMARY KEY,
    opened_at     TEXT NOT NULL,
    closed_at     TEXT,
    -- §52.2: a payment made with no closed window to attribute it to is the
    -- correlation this table family exists to prevent.
    CONSTRAINT taste_leakage_window_closes_after_it_opens
        CHECK (closed_at IS NULL OR closed_at >= opened_at)
);
CREATE INDEX IF NOT EXISTS idx_taste_leakage_windows_open
    ON taste_leakage_batch_windows (closed_at);

-- §52.2's "at most one open window", as a partial UNIQUE index rather than a
-- trigger. PostgreSQL can say this declaratively, so it should: the index is
-- smaller, faster, and self-documenting.
CREATE UNIQUE INDEX IF NOT EXISTS idx_taste_leakage_single_open_window
    ON taste_leakage_batch_windows ((TRUE))
    WHERE closed_at IS NULL;

CREATE TABLE IF NOT EXISTS taste_leakage_payouts (
    id             UUID PRIMARY KEY,
    recipient_pseud_id UUID NOT NULL,
    work_id        UUID NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    credits        BIGINT NOT NULL,
    window_id      UUID NOT NULL REFERENCES taste_leakage_batch_windows (id) ON DELETE RESTRICT,
    -- Only 'instance' is storable. §52.2: "you were rated highly" makes one
    -- rating action at one moment observable, which is a measurement of the lens.
    attribution    TEXT NOT NULL DEFAULT 'instance',
    paid_at        TEXT NOT NULL,
    CONSTRAINT taste_leakage_payout_credits_positive CHECK (credits > 0),
    CONSTRAINT taste_leakage_payout_is_instance_attributed
        CHECK (attribution = 'instance')
);
CREATE INDEX IF NOT EXISTS idx_taste_leakage_payouts_recipient
    ON taste_leakage_payouts (recipient_pseud_id, paid_at);
CREATE INDEX IF NOT EXISTS idx_taste_leakage_payouts_window
    ON taste_leakage_payouts (window_id);

-- §52.2's timing rule, as a trigger.
--
-- This is the one rule that does not port to a CHECK on either engine: `paid_at`
-- has to be compared against the OPENED_AT and CLOSED_AT of the window named by
-- `window_id`, and a CHECK constraint cannot reach into another table. It matters
-- because a payout timestamp seconds from a rating timestamp is a usable
-- correlation EVEN WITH NO PAYLOAD DETAIL -- the batch interval bounds what an
-- observer can infer about WHEN, not only about WHETHER.
CREATE OR REPLACE FUNCTION taste_leakage_payout_inside_window()
RETURNS TRIGGER AS $$
BEGIN
    IF NOT EXISTS (
        SELECT 1 FROM taste_leakage_batch_windows w
        WHERE w.id = NEW.window_id
          AND w.closed_at IS NOT NULL
          AND NEW.paid_at >= w.opened_at
          AND NEW.paid_at <= w.closed_at
    ) THEN
        RAISE EXCEPTION
            'a payout must be attributed to a closed window and fall inside it'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS taste_leakage_payout_inside_window_insert ON taste_leakage_payouts;
CREATE TRIGGER taste_leakage_payout_inside_window_insert
BEFORE INSERT ON taste_leakage_payouts
FOR EACH ROW EXECUTE FUNCTION taste_leakage_payout_inside_window();

DROP TRIGGER IF EXISTS taste_leakage_payout_inside_window_update ON taste_leakage_payouts;
CREATE TRIGGER taste_leakage_payout_inside_window_update
BEFORE UPDATE OF window_id, paid_at ON taste_leakage_payouts
FOR EACH ROW EXECUTE FUNCTION taste_leakage_payout_inside_window();

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
    id            UUID PRIMARY KEY,
    artifact      TEXT NOT NULL,
    inferable     TEXT NOT NULL,
    -- plain | derived | measured. §52.1's ease ladder: what an observant reader
    -- could put together without help, what needs a comparison across artifacts,
    -- and what needs measuring (where coarsening is the only real remedy).
    ease          TEXT NOT NULL,
    disposition   TEXT NOT NULL DEFAULT 'keep',
    reviewed_by   UUID NOT NULL REFERENCES pseuds (id) ON DELETE RESTRICT,
    reviewed_at   TEXT NOT NULL,
    CONSTRAINT taste_leakage_review_ease_is_known
        CHECK (ease IN ('plain', 'derived', 'measured')),
    CONSTRAINT taste_leakage_review_disposition_is_known
        CHECK (disposition IN ('keep', 'coarsen', 'remove'))
);
CREATE INDEX IF NOT EXISTS idx_taste_leakage_reviews_disposition
    ON taste_leakage_reviews (disposition);

-- ── §52.3 The owner-visible resonance label ─────────────────────────────────

-- §52.3: the label updates from a weekly batch, never on read or on the
-- underlying event, and the reason is the threat model -- an author who watches a
-- label tick after each event can infer the lens from the tick pattern, and on a
-- shared instance any reader can do that.
--
-- `batch_window_id` is NOT NULL and a real FK, because a label with no batch is
-- a per-event update wearing a batch's name and there would be no way to tell
-- afterwards.
CREATE TABLE IF NOT EXISTS taste_leakage_resonance_labels (
    work_id        UUID PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    owner_pseud_id UUID NOT NULL,
    -- quiet | steady | noticed | landing. No number is storable, so there is no
    -- precision to leak and no float to round.
    label          TEXT NOT NULL,
    computed_at    TEXT NOT NULL,
    batch_window_id UUID NOT NULL REFERENCES taste_leakage_batch_windows (id) ON DELETE RESTRICT,
    CONSTRAINT taste_leakage_label_is_coarse
        CHECK (label IN ('quiet', 'steady', 'noticed', 'landing'))
);
CREATE INDEX IF NOT EXISTS idx_taste_leakage_labels_owner
    ON taste_leakage_resonance_labels (owner_pseud_id);

-- A closed window may not be re-closed: a payout or label inside it is attributed
-- to the interval it describes, so editing the end retroactively widens what
-- every row in it claims to represent.
CREATE OR REPLACE FUNCTION taste_leakage_window_closed_is_immutable()
RETURNS TRIGGER AS $$
BEGIN
    IF OLD.closed_at IS NOT NULL THEN
        RAISE EXCEPTION 'a closed batch window cannot be re-closed'
            USING ERRCODE = 'check_violation';
    END IF;
    RETURN NEW;
END;
$$ LANGUAGE plpgsql;

DROP TRIGGER IF EXISTS taste_leakage_window_closed_is_immutable ON taste_leakage_batch_windows;
CREATE TRIGGER taste_leakage_window_closed_is_immutable
BEFORE UPDATE OF closed_at ON taste_leakage_batch_windows
FOR EACH ROW EXECUTE FUNCTION taste_leakage_window_closed_is_immutable();