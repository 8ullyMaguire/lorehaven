-- M45-57: curator-submitted source adapters (spec §55.2, §19.4).
--
-- Identical to migrations/sqlite/0113_source_adapters.sql in content. The two files exist
-- separately because most migrations in this codebase genuinely differ — but this pair is
-- worth reading closely for what it does *not* change:
--
--   * `id`, `submitter`, `reviewer_account` are UUID here and TEXT on the SQLite twin,
--     because `accounts.id` is typed per dialect. Any store function binding these from a
--     `&str` needs `$n::uuid` on this side or PostgreSQL rejects the bind with 42804;
--     `6160a4f` fixed two such casts in `taste_vectors.rs` and they are the reason this
--     comment exists.
--   * `created_at`/`decided_at` are TIMESTAMPTZ here and TEXT on SQLite. This is the
--     asymmetry that has bitten before: `fix-timestamptz-binds.py` assumed these columns
--     were TEXT everywhere and its own added casts were themselves defects, so the script
--     is disabled and only its self-test runs.
--   * `manifest` and `source_manifest` stay TEXT, NOT JSONB. They are curator-authored
--     YAML, and §55.3's schema rejects unknown fields — a JSONB column would accept any
--     JSON at all, which is the opposite of what the review argument needs. The store
--     parses on read and the parse failure is the review's first gate.
--   * `reason` and `note` are TEXT, nullable, and that is a content decision rather than
--     a dialect one: §19.5 requires a reason for an emergency action and §55.5 requires
--     a reviewer's reasoning, so neither column can be NOT NULL.

CREATE TABLE IF NOT EXISTS extension_submissions (
    id                UUID PRIMARY KEY,
    submitter         UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    manifest          TEXT NOT NULL,
    source_manifest   TEXT,
    state             TEXT NOT NULL DEFAULT 'pending'
                      CHECK (state IN ('pending', 'approved', 'rejected', 'revoked')),
    reason            TEXT,
    created_at        TIMESTAMPTZ NOT NULL,
    decided_at        TIMESTAMPTZ
);

CREATE INDEX IF NOT EXISTS extension_submissions_pending
    ON extension_submissions (state, created_at DESC);

CREATE TABLE IF NOT EXISTS adapter_reviews (
    id                UUID PRIMARY KEY,
    submission_id     UUID NOT NULL REFERENCES extension_submissions (id) ON DELETE CASCADE,
    reviewer_account  UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    verdict           TEXT NOT NULL CHECK (verdict IN ('approve', 'reject', 'abstain')),
    note              TEXT,
    created_at        TIMESTAMPTZ NOT NULL,
    UNIQUE (submission_id, reviewer_account)
);

CREATE INDEX IF NOT EXISTS adapter_reviews_by_submission
    ON adapter_reviews (submission_id, verdict);
