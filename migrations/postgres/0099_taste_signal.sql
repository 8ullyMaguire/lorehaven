-- Migration 0099 — tag confirmation and the tasting menu (spec §49.2, §49.5).
--
-- Dialect: PostgreSQL.
--
-- Mirrors migrations/sqlite/0099_taste_signal.sql statement for statement. The
-- parity test (the_two_dialects_declare_the_same_columns_and_indexes) is what
-- catches a divergence, and it only catches what is written down here — so this
-- file has to carry every column the SQLite one does, including the ones that
-- look redundant.

ALTER TABLE work_tags
    ADD COLUMN confirmation TEXT NOT NULL DEFAULT 'unconfirmed'
        CHECK (confirmation IN ('unconfirmed', 'reader', 'wrangler', 'inaccurate'));

-- 'unconfirmed' is the honest default. Every existing row is author-applied, and
-- nothing before this migration distinguished that from a reader's endorsement.
-- Defaulting to 'reader' would manufacture a confirmation per tag and have all
-- of them count toward gravity at once — the outcome §49.2 forbids, reached by
-- migration rather than decision.

-- §49.2's inaccurate-flag rule, as a confirmation state rather than a separate
-- boolean so the four states are mutually exclusive by construction.
ALTER TABLE work_tags
    ADD COLUMN flagged_by TEXT;
ALTER TABLE work_tags
    ADD COLUMN flagged_at TEXT;

-- The reader's side of §49.5. `sample_offset` makes the 300-word window
-- reproducible from stored coordinates; §49.5 calls the passage a sample of the
-- prose, and a sample you cannot reproduce is a sample of nothing.
CREATE TABLE tasting_samples (
    id TEXT PRIMARY KEY,
    work_id UUID NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    account_id TEXT NOT NULL,
    sample_offset BIGINT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX tasting_samples_account ON tasting_samples (account_id, id);
CREATE INDEX tasting_samples_work ON tasting_samples (work_id);

-- `reason` NOT NULL: §49.5 makes the reason the labelled signal, so the schema
-- must make a bare rating unrepresentable rather than merely discouraged.
-- `uncertainty_at_draw` records the selector's claim at the time, so an
-- evaluation can check the queue chose uncertain items rather than take its
-- word for it.
CREATE TABLE tasting_responses (
    id TEXT PRIMARY KEY,
    sample_id TEXT NOT NULL REFERENCES tasting_samples (id) ON DELETE CASCADE,
    account_id TEXT NOT NULL,
    verdict TEXT NOT NULL CHECK (verdict IN ('like', 'dislike')),
    reason TEXT NOT NULL,
    free_text TEXT,
    uncertainty_at_draw DOUBLE PRECISION NOT NULL,
    session_id TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX tasting_responses_sample ON tasting_responses (sample_id);
CREATE INDEX tasting_responses_account ON tasting_responses (account_id, created_at);

-- §49.7: a decline is a negative carrying its reason. The response is the
-- evidence; the sample is kept for audit and cascades away with its responses.