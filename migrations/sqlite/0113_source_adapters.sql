-- M45-57: curator-submitted source adapters (spec §55.2, §19.4).
--
-- Two tables. `extension_submissions` is the §21.5 pipeline's state; `adapter_reviews`
-- is §19.4's three-reviewer record, one row per reviewer.
--
-- The UNIQUE on `(submission_id, reviewer_account_id)` is the whole point of the second
-- table. §19.4 requires "three reviewers with permission-review expertise" for an
-- extension approval, and a table without the constraint records one reviewer's three
-- opinions as three approvals. The constraint is what makes the threshold a fact about
-- the data rather than a rule a query has to remember.

CREATE TABLE IF NOT EXISTS extension_submissions (
    id                TEXT PRIMARY KEY,
    -- The curator who submitted it. §55.2 requires TL3 *at submission*, and the check
    -- lives in the store rather than only in the route because the route is not the only
    -- caller. `trust_levels.level` is an integer ladder (TL_NEW = 0 … TL_TRUSTEE = 6),
    -- so the bar is `>= 3` and is written as a number in the query rather than as a
    -- string a caller could spell differently.
    submitter         TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    -- The §21.1 manifest, verbatim. Stored as the curator sent it so a reviewer reads
    -- exactly what was submitted; the *compiled* form is derived on demand and never
    -- stored, because a stored compiled copy could drift from the text that was reviewed.
    manifest          TEXT NOT NULL,
    -- The declarative adapter's own manifest (§55.3), for a `source_adapters` submission.
    -- NULL for a non-adapter extension. Kept separate from `manifest` because §55.3's
    -- YAML is reviewed by a steward reading selectors, while §21.1's manifest is a
    -- header, and merging them would mean one column that is either.
    source_manifest   TEXT,
    -- §21.5's pipeline state. Mirrors `ExtensionState` in
    -- crates/domain/src/extension.rs, spelled the same way so the store can hand this
    -- string to `ExtensionState::from_str` without a second mapping.
    state             TEXT NOT NULL DEFAULT 'pending'
                      CHECK (state IN ('pending', 'approved', 'rejected', 'revoked')),
    -- A curator's rejection reason or a revocation note (§19.5 requires a reason).
    reason            TEXT,
    created_at        TEXT NOT NULL,
    decided_at        TEXT
);

-- The queue §55.2's workflow reads. Newest first, and partial: the common query is
-- "what is waiting", which `state = 'pending'` serves from this index without touching
-- the decided rows.
CREATE INDEX IF NOT EXISTS extension_submissions_pending
    ON extension_submissions (state, created_at DESC);

-- One reviewer's verdict on one submission.
CREATE TABLE IF NOT EXISTS adapter_reviews (
    id                TEXT PRIMARY KEY,
    submission_id     TEXT NOT NULL REFERENCES extension_submissions (id) ON DELETE CASCADE,
    reviewer_account  TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    verdict           TEXT NOT NULL CHECK (verdict IN ('approve', 'reject', 'abstain')),
    -- What the reviewer said. §55.5's review is only meaningful if the reasoning is
    -- recorded next to the verdict; a bare approve/reject is not reviewable later.
    note              TEXT,
    created_at        TEXT NOT NULL,
    -- §19.4's threshold, as data. A second verdict by the same reviewer is an error,
    -- not a second row, and this is what enforces it.
    UNIQUE (submission_id, reviewer_account)
);

-- Counting approvals for a submission is the query §55.2's publish step makes on every
-- request to publish, so it is indexed rather than left to a scan of the whole table.
CREATE INDEX IF NOT EXISTS adapter_reviews_by_submission
    ON adapter_reviews (submission_id, verdict);
