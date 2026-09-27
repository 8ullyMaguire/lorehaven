-- M54: the bot link flow (spec §23.2).
--
-- Dialect: SQLite.
--
-- The counterpart of migrations/postgres/0083_bot_link_challenges.sql. The
-- declared table, its columns, its index and its foreign key must match the
-- PostgreSQL file's, because
-- `the_two_dialects_declare_the_same_columns_and_indexes` in
-- crates/db/src/migrate.rs compares exactly that. Two of its rules are visible
-- in this file: an index body must sit on ONE line (the parser scans line by
-- line for ` ON `, so a wrapped body is invisible to it), and an index name is
-- read as the last whitespace token before ` ON` — which is why there is no
-- `IF NOT EXISTS` prefix here, because that would make the parser read the name
-- as `NOT`.
--
-- `link_challenges` is the whole of the linking handshake:
--
--     bot issues a short-lived challenge
--       → user opens Lorehaven, signs in on Lorehaven only
--       → selects a pseud and the scopes to grant
--       → confirms
--       → bot redeems the challenge and receives a revocable, scoped token
--
-- The challenge carries no secret of the reader's. It is a high-entropy public
-- nonce whose *single use* is the security property, and the reader's identity
-- arrives with the confirmation rather than with the challenge — so a challenge
-- leaked from a chat log is worthless to whoever holds it, which is the reason
-- the flow never asks the reader for a password (spec §23.2: "Bots never
-- receive the user's Lorehaven password").
--
-- `expires_at` is short by default, precisely so a challenge pasted into a
-- public channel yesterday cannot be redeemed today.
--
-- `state` is a stored value rather than an inference from `used_at`/`expires_at`,
-- for the same reason migration 0080 gave `claims` one: a row that *can* say
-- 'expired' while the predicate that would make it expired is never evaluated
-- is a state no query can rely on. The redemption arm below is the predicate,
-- and it is the single writer of 'used'.
--
-- `requested_scopes` is what the bot asked for, recorded so the confirmation
-- page can display it. It is not what gets granted: the reader picks, and the
-- grant is re-checked against this list server-side at confirmation time.
--
-- Dialect notes, both of them forced by SQLite rather than chosen:
--
--   * `ALTER TABLE ... ADD COLUMN` is fine here, and is the only way to add
--     columns to an existing table.
--
--   * `ALTER TABLE ... ADD CONSTRAINT` is NOT supported, so the
--     `bot_registrations` foreign key is applied the way migration 0075 did it:
--     rename the table away, CREATE under the real name, copy, drop. The
--     `PRAGMA foreign_keys=OFF` window covers the rename, because SQLite
--     rewrites *referencing* foreign keys when a table is renamed — the exact
--     trap 0075 documents, where the rewrite silently repointed
--     `device_deliveries.export_job_id` at a dropped `export_jobs_old`.
--
--     Nothing references `bot_registrations`, so dropping it cannot cascade into
--     a second table.

CREATE TABLE link_challenges (
    code            TEXT PRIMARY KEY,
    state           TEXT    NOT NULL CHECK (state IN ('pending', 'used', 'expired')),
    bot_id          TEXT    NOT NULL REFERENCES bot_registrations (id) ON DELETE CASCADE,
    requested_scopes TEXT   NOT NULL,
    -- The pseud the reader chose, and the token minted for it. Both NULL until
    -- the reader confirms, because a challenge that arrives without them is
    -- unconfirmed, and an unconfirmed challenge must not be redeemable.
    pseud_id        TEXT,
    token_id        TEXT,
    created_at      TEXT    NOT NULL,
    expires_at      TEXT    NOT NULL,
    used_at         TEXT
);
-- The redemption query is `WHERE code = ?`, so the primary key serves it; this
-- index serves the sweeper that expires stale rows and the operator view.
-- One line, and no IF NOT EXISTS — both required by the parity test above.
CREATE INDEX link_challenges_pending ON link_challenges (state, expires_at);

-- api_tokens gains the two columns it is genuinely missing.
--
-- `expires_at` and `last_used_at` already exist here (0001) and are already
-- read by list_tokens. They are simply never written by any code path, so a
-- token can never expire and its age is unobservable. That is fixed in the
-- repository layer, not by adding columns that are already present.
--
-- `acting_pseud_id` is spec §23.1's "explicit acting pseud". A token belongs to
-- an ACCOUNT but acts as a PSEUD, and without this column every token call acts
-- as the account's default pseud — silently conflating two identities that the
-- rest of the schema keeps strictly apart.
ALTER TABLE api_tokens ADD COLUMN acting_pseud_id TEXT;
-- `kind` is read by issue_token's `_kind` argument and then discarded, so every
-- token in the database is indistinguishable from a personal one however it was
-- issued. DEFAULT 'personal' matches every token issued before this migration.
ALTER TABLE api_tokens ADD COLUMN kind TEXT NOT NULL DEFAULT 'personal';

-- D5 (`bot_registrations.token_id` has no foreign key) is deliberately NOT fixed
-- in this migration, and the PostgreSQL twin explains why: on PostgreSQL
-- `api_tokens.id` is a UUID while `bot_registrations.token_id` is TEXT, so the
-- constraint needs a column type migration first. It gets its own migration
-- (0084) rather than riding along with the link flow.
--
-- Nothing in this file therefore recreates `bot_registrations`, which also keeps
-- this file clear of the rename trap that 0075 exists to document.
