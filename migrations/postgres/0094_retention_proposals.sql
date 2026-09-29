-- M59 Phase E: retention proposals, ballots, and the record of what changed.
--
-- Dialect: PostgreSQL.
--
-- The counterpart of migrations/sqlite/0094_retention_proposals.sql. Tables,
-- columns and indexes must match, because
-- `the_two_dialects_declare_the_same_columns_and_indexes` in
-- crates/db/src/migrate.rs compares exactly that. The SQLite file carries the
-- reasoning; it is not repeated here.
--
-- What differs, and it is the same asymmetry 0093 records: `accounts.id` is a
-- real UUID in this schema and TEXT in the SQLite one, so the three references
-- to it take `UUID` here and `TEXT` there. That is the only reason these two
-- files are not textually identical past the comment, and it is why a
-- migration edited in one dialect has to be edited in the other.

CREATE TABLE retention_proposals (
    id              UUID PRIMARY KEY,
    -- `cache` or `aggregate`: the two modes of §11.15. The CHECK is the same
    -- one the SQLite file declares, and
    -- `every_retention_proposal_mode_the_store_accepts_is_accepted_by_the_schema`
    -- asserts the store and the schema agree on the accepted set -- a store
    -- that parses a value the schema forbids is a reader who can open a
    -- proposal that will not survive a restart.
    proposed_mode   TEXT NOT NULL CHECK (proposed_mode IN ('cache', 'aggregate')),
    source_key      TEXT,
    rationale       TEXT NOT NULL,
    opened_by       UUID NOT NULL REFERENCES accounts (id) ON DELETE RESTRICT,
    closes_at       TEXT NOT NULL,
    state           TEXT NOT NULL DEFAULT 'open'
                    CHECK (state IN ('open', 'passed', 'failed', 'overridden', 'expired')),
    tallied_at      TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    version         INTEGER NOT NULL DEFAULT 1
);

-- No weight column. §45.2: "every vote weighs 1. Taste affinity, trust level,
-- and private preference are never part of governance." The cost asymmetry
-- (§5.3) is `quorum_for`, which reads a count. The full argument is in the
-- SQLite file's header, where it is long enough that a reader needs it.
CREATE TABLE retention_proposal_votes (
    proposal_id  UUID NOT NULL REFERENCES retention_proposals (id) ON DELETE CASCADE,
    account_id   UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    support      INTEGER NOT NULL CHECK (support IN (0, 1)),
    cast_at      TEXT NOT NULL,
    PRIMARY KEY (proposal_id, account_id)
);

CREATE TABLE retention_policy_changes (
    id          UUID PRIMARY KEY,
    from_mode   TEXT CHECK (from_mode IS NULL OR from_mode IN ('cache', 'aggregate')),
    to_mode     TEXT NOT NULL CHECK (to_mode IN ('cache', 'aggregate')),
    source_key  TEXT,
    actor       UUID NOT NULL REFERENCES accounts (id) ON DELETE RESTRICT,
    reason      TEXT NOT NULL,
    decided_at  TEXT NOT NULL
);

CREATE INDEX idx_retention_proposals_open
    ON retention_proposals (source_key, state);

CREATE INDEX idx_retention_votes_support
    ON retention_proposal_votes (proposal_id, support);

CREATE INDEX idx_retention_changes_recent
    ON retention_policy_changes (decided_at);
