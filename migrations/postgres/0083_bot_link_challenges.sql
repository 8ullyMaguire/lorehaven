-- M54: the bot link flow (spec §23.2).
--
-- Dialect: PostgreSQL.
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
-- grant is re-checked against this list server-side at confirmation time. A bot
-- that asked for admin.write and was refused admin.write has still been told
-- what it asked for, which is the disclosure §23.2's security note requires.
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
CREATE INDEX link_challenges_pending ON link_challenges (state, expires_at);

-- api_tokens gains the two columns it is genuinely missing.
--
-- `expires_at` and `last_used_at` already exist here (0001) and are already read
-- by list_tokens. They are simply never written by any code path, so a token can
-- never expire and its age is unobservable. That is fixed in the repository
-- layer, not by adding columns that are already present.
--
-- `acting_pseud_id` is spec §23.1's "explicit acting pseud". A token belongs to
-- an ACCOUNT but acts as a PSEUD, and without this column every token call acts
-- as the account's default pseud — silently conflating two identities that the
-- rest of the schema keeps strictly apart (pseuds are per-account and a reader
-- may wear several faces).
--
-- Declared TEXT, not UUID, to match the other cross-references between this
-- schema's TEXT ids and its uuid ids. A reader's default pseud and a token's
-- acting pseud are compared as strings on both backends.
ALTER TABLE api_tokens ADD COLUMN acting_pseud_id TEXT;
-- `kind` is read by issue_token's `_kind` argument and then discarded, so every
-- token in the database is indistinguishable from a personal one however it was
-- issued. DEFAULT 'personal' matches every token issued before this migration,
-- which were all issued through the personal door.
ALTER TABLE api_tokens ADD COLUMN kind TEXT NOT NULL DEFAULT 'personal';

-- D5 (`bot_registrations.token_id` has no foreign key) is NOT fixed here. It
-- needs its own migration, and the reason is a type problem rather than a
-- tidiness one: on PostgreSQL `api_tokens.id` is a UUID while
-- `bot_registrations.token_id` is TEXT, so the constraint cannot be created
-- without first migrating the column's type and every query that binds it.
-- Splitting it keeps this migration to the link flow and defers a type
-- migration to the migration that can afford one. See 0084.
