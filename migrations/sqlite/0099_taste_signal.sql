-- Migration 0099 — tag confirmation and the tasting menu (spec §49.2, §49.5).
--
-- Dialect: SQLite.
--
-- ALTER work_tags rather than CREATE a confirmation table. `work_tags` (0011)
-- is already the record of which tags a work has; a second table saying which
-- of those count would be two tables disagreeing about the same fact, with
-- nothing keeping them in step. Same reasoning as 0098, which added columns to
-- `recommendation_slots` rather than creating a parallel `impressions` table.

ALTER TABLE work_tags
    ADD COLUMN confirmation TEXT NOT NULL DEFAULT 'unconfirmed'
        CHECK (confirmation IN ('unconfirmed', 'reader', 'wrangler', 'inaccurate'));

-- 'unconfirmed' is the honest default, not the convenient one. Every existing
-- row is an author-applied tag, and before this migration nothing distinguished
-- that from a reader's endorsement. Defaulting to 'reader' would manufacture a
-- confirmation for every tag on the instance and have all of them count toward
-- gravity immediately -- which is the exact outcome §49.2 forbids, arrived at by
-- a migration rather than by a decision.

-- §49.2's inaccurate-flag rule: a reader who flags a bad tag must stop paying
-- for it at once, not when the wrangling queue happens to get to it. Recorded as
-- a confirmation state rather than a separate boolean so the four states are
-- mutually exclusive by construction -- a tag cannot be both confirmed and
-- flagged, which a second column would permit.
--
-- `flagged_by` is NULL for states a reader did not personally record, which is
-- why it is nullable rather than defaulting to a placeholder id.
ALTER TABLE work_tags
    ADD COLUMN flagged_by TEXT;
ALTER TABLE work_tags
    ADD COLUMN flagged_at TEXT;

-- The reader's side of §49.5: what was shown, and what they said about it.
--
-- `sample_offset` is where the 300-word window started, so a sample is
-- reproducible from the stored coordinates rather than re-drawn. §49.5 calls the
-- passage a sample of the prose, and a sample you cannot reproduce is not a
-- sample of anything.
CREATE TABLE tasting_samples (
    id TEXT PRIMARY KEY,
    -- The FK the PostgreSQL arm declares, and this one did not until the parity
    -- test said so. Declaring it in only one dialect means a deleted work leaves
    -- samples behind on one engine and not the other, and the divergence is
    -- invisible until a cascade is tested.
    work_id TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    account_id TEXT NOT NULL,
    sample_offset INTEGER NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX tasting_samples_account ON tasting_samples (account_id, id);
CREATE INDEX tasting_samples_work ON tasting_samples (work_id);

-- `reason` is NOT NULL and NOT a bare rating, because §49.5 makes the reason the
-- labelled signal: "didn't like it" does not say whether prose or
-- characterisation was the problem. Making it nullable would mean the queue can
-- collect exactly the rows that teach nothing.
--
-- `verdict` is 'like' or 'dislike', never NULL. A response with no verdict is
-- not an abstention, it is an unanswered sample that must come back.
--
-- `uncertainty_at_draw` is stored so an evaluation can ask whether the queue
-- actually chose uncertain items, and not merely whether it claims to. A number
-- written by the selector is a claim; this is the claim, recorded at the time.
CREATE TABLE tasting_responses (
    id TEXT PRIMARY KEY,
    sample_id TEXT NOT NULL REFERENCES tasting_samples (id) ON DELETE CASCADE,
    account_id TEXT NOT NULL,
    verdict TEXT NOT NULL CHECK (verdict IN ('like', 'dislike')),
    reason TEXT NOT NULL,
    free_text TEXT,
    uncertainty_at_draw REAL NOT NULL,
    session_id TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX tasting_responses_sample ON tasting_responses (sample_id);
CREATE INDEX tasting_responses_account ON tasting_responses (account_id, created_at);

-- §49.7: "no sample is discarded for being a surprise." A decline is a negative
-- carrying its reason, so the response is what the profile is trained on and the
-- sample itself is only kept for audit. `ON DELETE CASCADE` means re-running the
-- queue after a purge takes the responses with it, which is the correct
-- direction: a response to a sample nobody can show is not evidence.