-- M59 Phase E: retention proposals, ballots, and the record of what changed.
--
-- Dialect: SQLite.
--
-- The counterpart of migrations/postgres/0094_retention_proposals.sql. Tables,
-- columns and indexes must match, because
-- `the_two_dialects_declare_the_same_columns_and_indexes` in
-- crates/db/src/migrate.rs compares exactly that. What differs between the two
-- files is the reference *types*, and 0093's header explains the rule this
-- migration follows: a column that references `accounts(id)` is TEXT here and
-- UUID there, so the PostgreSQL file takes the cast in the DDL and this one
-- does not.
--
-- WHY THREE TABLES AND NOT ONE. A proposal is a question, a ballot is a
-- reader's answer to it, and a change is a thing that happened. Collapsing
-- them would mean either overwriting a question with its outcome (losing what
-- was asked) or storing outcomes as rows in the proposal (making "what is
-- happening now" a query rather than a row). Three tables, three questions:
-- what was proposed, how people answered, what the instance did.
--
-- NO WEIGHT COLUMN, and this is load-bearing rather than a simplification.
-- §45.2 is explicit that governance votes are flat: "every vote weighs 1.
-- Taste affinity, trust level, and private preference are never part of
-- governance." A `weight_bp` here would let a reading habit set instance
-- policy -- the exact inversion §19.2 refuses. The M58 directory vote's
-- `base_weight` is NOT a precedent: that ranks entries in a directory listing,
-- which is not a governance vote, and conflating the two is how the wrong
-- column gets justified later.
--
-- The cost asymmetry (§5.3) is handled by quorum rather than by weight.
-- Narrowing storage is cheap and reversible, so it passes at a low bar;
-- widening it commits storage and bandwidth indefinitely, so it needs more
-- ballots. `quorum_for` in crates/domain/src/retention.rs is the one pure
-- function that encodes this, and it reads a count, never a weight.

CREATE TABLE IF NOT EXISTS retention_proposals (
    id              TEXT PRIMARY KEY,
    -- `cache` or `aggregate`: the two modes of §11.15. A CHECK rather than an
    -- enum, because an unreadable proposal is a proposal a reader cannot vote
    -- on and cannot be shown a reason for.
    proposed_mode   TEXT NOT NULL CHECK (proposed_mode IN ('cache', 'aggregate')),
    -- NULL is the instance-wide setting; a value scopes the proposal to one
    -- source. Scoped proposals are the common case, because the decision a
    -- reader cares about is usually about one archive.
    source_key      TEXT,
    rationale       TEXT NOT NULL,
    opened_by       TEXT NOT NULL REFERENCES accounts (id) ON DELETE RESTRICT,
    -- When the ballot closes. Stored as a timestamp rather than a day count so
    -- that an instance changing its configured cooling period does not silently
    -- move every open proposal's deadline.
    closes_at       TEXT NOT NULL,
    -- open | passed | failed | overridden | expired. The five are not
    -- interchangeable: `overridden` means an operator answered a passed
    -- proposal against its own recommendation, and an operator dashboard that
    -- cannot tell that from `passed` is lying about who decided.
    state           TEXT NOT NULL DEFAULT 'open'
                    CHECK (state IN ('open', 'passed', 'failed', 'overridden', 'expired')),
    tallied_at      TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    version         INTEGER NOT NULL DEFAULT 1
);

-- A reader has one ballot per proposal. The composite primary key IS the
-- anti-buy mechanism: a second vote from the same account updates the first, so
-- a vote cannot be stacked. A separate `id` column would have made that a rule
-- the store had to enforce; here it is the schema.
--
-- ON DELETE CASCADE from `proposals`: a deleted proposal takes its ballots with
-- it, which is right because a ballot is an answer to a question that no longer
-- exists and has nothing to say to anyone.
CREATE TABLE IF NOT EXISTS retention_proposal_votes (
    proposal_id  TEXT NOT NULL REFERENCES retention_proposals (id) ON DELETE CASCADE,
    account_id   TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    -- 1 or 0. An INTEGER rather than a BOOLEAN because SQLite has no BOOLEAN
    -- and a dialect pair that disagree on the declared type is a parity test
    -- failure; INTEGER is what both accept.
    support      INTEGER NOT NULL CHECK (support IN (0, 1)),
    cast_at      TEXT NOT NULL,
    PRIMARY KEY (proposal_id, account_id)
);

-- The record of what the instance actually did, kept separately from the
-- proposal so that "the setting changed" and "the setting changed because of
-- this proposal" are the same fact rather than a coincidence someone has to
-- reconstruct.
--
-- `from_mode` is nullable because an override may apply to a setting that has
-- no explicit value yet -- the instance default is the thing being changed.
CREATE TABLE IF NOT EXISTS retention_policy_changes (
    id          TEXT PRIMARY KEY,
    from_mode   TEXT CHECK (from_mode IS NULL OR from_mode IN ('cache', 'aggregate')),
    to_mode     TEXT NOT NULL CHECK (to_mode IN ('cache', 'aggregate')),
    source_key  TEXT,
    actor       TEXT NOT NULL REFERENCES accounts (id) ON DELETE RESTRICT,
    reason      TEXT NOT NULL,
    decided_at  TEXT NOT NULL
);

-- The open-proposal lookup. `partial` is not available in SQLite, so this is a
-- plain index on (source_key, state) -- which is what `a_second_proposal_on_the
-- same_setting_is_refused_while_one_is_open` actually queries. The index is
-- worth having on a small table because that check runs on every open attempt
-- and the table grows without bound as proposals expire.
CREATE INDEX IF NOT EXISTS idx_retention_proposals_open
    ON retention_proposals (source_key, state);

-- The tally. Proposal id is already the leading column of the primary key, so
-- the ballots are already clustered by it and this index would be redundant on
-- SQLite; it exists because the equivalent PostgreSQL query filters on
-- `support` to count each side separately, and the parity test compares index
-- names.
CREATE INDEX IF NOT EXISTS idx_retention_votes_support
    ON retention_proposal_votes (proposal_id, support);

-- What changed, most recent first. The operator dashboard reads this in order
-- and has to page through it.
CREATE INDEX IF NOT EXISTS idx_retention_changes_recent
    ON retention_policy_changes (decided_at);

-- THE INSTANCE'S OWN ACCOUNT.
--
-- `retention_policy_changes.actor` above is `NOT NULL REFERENCES accounts (id)
-- ON DELETE RESTRICT`, deliberately: every recorded decision names an account,
-- so an audit row is never a row with nobody on it and a decision cannot be
-- orphaned by an account deletion. That constraint needs a third thing, which
-- the two obvious answers are not.
--
-- A **binding-mode settlement** commits a setting on the readers' recorded
-- decision with no operator involved. So its actor is not an operator, and it
-- must not be one of the readers who voted:
--
-- * NULL is refused by `NOT NULL`, and would be wrong even if permitted. "No
--   one did this" is a different claim from "the instance did this", and only
--   the second is true.
-- * the nil UUID is refused by the foreign key, which is the database correctly
--   saying it is not an account.
-- * a reader's id satisfies the constraint and destroys the feature's central
--   privacy property, by putting one of the three voters' names on the row
--   their own ballot produced. §45.2's argument against weights — a reading
--   habit must not set instance policy — is the same argument, and a name in
--   the audit trail is how a preference becomes a reputation.
--
-- So: a real account that is not a person. No login path, an email in the
-- reserved `.invalid` TLD (RFC 2606, so it can never be deliverable and can
-- never collide with a registration under the unique index on
-- `lower(email)`), and `age_state` left `unknown` rather than `adult` because
-- nothing that counts accounts should treat this as a reader.
--
-- The id is a fixed literal rather than a generated one so it is identifiable
-- in a database dump: an operator seeing it knows at a glance that a row was
-- written by the instance and not by a person. The version and variant nibbles
-- are set so it is a well-formed v4-shaped UUID.
INSERT OR IGNORE INTO accounts
    (id, status, email, age_state, created_at, updated_at, version)
VALUES
    ('01959000-0000-4000-8000-000000000001', 'system', 'instance@retention.system.invalid', 'unknown',
     '2026-01-01T00:00:00Z', '2026-01-01T00:00:00Z', 1);
