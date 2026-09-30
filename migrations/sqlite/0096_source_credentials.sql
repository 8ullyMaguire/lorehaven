-- M53-03: the three §11.6 vault properties the 0006 schema does not carry.
--
-- Dialect: SQLite.
--
-- The counterpart of migrations/postgres/0096_source_credentials.sql. Tables,
-- columns and indexes must match, because
-- `the_two_dialects_declare_the_same_columns_and_indexes` in
-- crates/db/src/migrate.rs compares exactly that. What differs between the two
-- files is the reference *types*: `pseuds.id` is TEXT here and UUID there, so
-- the references take their cast in the PostgreSQL file and not in this one.
--
-- WHY THIS FILE ALTERS A TABLE INSTEAD OF CREATING IT. An earlier draft of this
-- migration did `CREATE TABLE IF NOT EXISTS source_credentials` with the full
-- plan schema. That is a no-op, and it is a no-op that reports success:
-- 0006 already creates this table, `IF NOT EXISTS` suppresses the conflict, the
-- migration applies cleanly, and not one column below exists afterwards. The
-- plan's Step 1 verification -- `sqlite3 :memory: < 0096.sql`, "no error" --
-- passed on a file that had done nothing at all. That is the second time in
-- this repo that a gate which could not fail was trusted (the first was
-- `cargo clippy 2>/dev/null | grep -c`, recorded in docs/verification.md), and
-- it is why every column added here is asserted behaviourally in
-- crates/app/tests/m53_source_credentials.rs rather than by a syntax check.
--
-- WHAT 0006 GOT RIGHT AND THIS MIGRATION KEEPS. The table holds the ciphertext
-- *by reference* -- `secret_id` into `secrets`, which is separately keyed and
-- separately encrypted -- rather than a `ciphertext BLOB` of its own. That is why
-- there is no `ciphertext` column here: a second copy of a secret, bound to
-- different associated data, is a second thing to leak and a second thing to
-- rotate. The plan asked for `ciphertext BLOB NOT NULL`; that would have been the
-- worse schema, and it is not added.
--
-- THE SAME REASONING FOR THE UNIQUE INDEX. The plan asked for
-- `UNIQUE (pseud_id, source_key)`; 0006 already has
-- `UNIQUE (pseud_id, source_key, label)`, and that is kept. Two accounts on one
-- reader's pseud for one source is a real use -- a second login when the first
-- expires, a work account beside a personal one -- and collapsing it to one
-- credential per source would delete a feature §11.6 nowhere forbids. "No
-- automatic copying across pseuds" is a property of the store layer: nothing on
-- the request path may write a row under a pseud other than the caller's. That
-- is tested as a store-layer refusal in the same suite, because forcing it into
-- the schema would buy the property at the price of a real feature.

-- Which kind of secret this row holds: `token` | `password` | `session_cookie`.
--
-- NULLABLE, and this is the most important decision in the file. An adapter
-- declares its own `AuthKind`, so this column is redundant *today* -- it exists
-- to record what the reader was told at the moment they consented, and a row
-- written before consent was recorded has no such moment. Backfilling it from
-- the adapter's current kind would be inventing a consent record, which is the
-- one thing §11.6's "requires explicit consent" exists to prevent. NULL means
-- "not recorded", and the store treats it as un-consented rather than guessing.
ALTER TABLE source_credentials ADD COLUMN kind TEXT
    CHECK (kind IS NULL OR kind IN ('token', 'password', 'session_cookie'));

-- Origin-bound credential use (spec §11.6): a credential is refused for any host
-- other than the one it was bound to, which is what stops a cookie pasted for
-- one site being replayed against a lookalike domain.
--
-- NULLABLE for the same reason as `kind`, with a sharper consequence: we cannot
-- reconstruct which host an existing credential was created for, so an unbound
-- row is not something to tidy into a bound one. It is refused for use, with a
-- message naming the fix, and the fix -- storing it again -- records the origin
-- that was missing.
ALTER TABLE source_credentials ADD COLUMN origin_host TEXT;

-- Explicit consent (spec §11.6): password and session-cookie storage requires
-- it, so it is recorded rather than implied by the row existing. A renewal is a
-- fresh consent, which is what "re-consent for renewal" means -- so this is
-- rewritten on every store and never carried forward.
ALTER TABLE source_credentials ADD COLUMN consent_at TEXT;

-- Audit events without secret contents (spec §11.6).
--
-- No column for the ciphertext, and deliberately no free-text field that could be
-- used to smuggle one: an audit row that *can* hold a secret is an audit log that
-- will eventually hold one, because "put the error message here" is always
-- available at the call site and always tempting.
CREATE TABLE IF NOT EXISTS credential_audit_events (
    id            TEXT PRIMARY KEY,
    -- NOT a foreign key, and that is the decision worth arguing for.
    --
    -- A cascade here would delete an audit row the moment its credential is
    -- revoked — which is precisely the row an operator most wants when they ask
    -- "was this login ever used, and did we stop using it?". §11.6 asks for both
    -- "immediate revocation" and "audit events", and a trail that vanishes on
    -- revocation satisfies the first by destroying the evidence for the second.
    --
    -- So `credential_id` is a historical reference, not a live one: the id it
    -- names may no longer resolve, and that is fine. Writes still go through
    -- `record_credential_event`, which checks the credential exists and belongs
    -- to the calling pseud before inserting, so this cannot be used to invent
    -- audit rows for credentials that were never stored.
    credential_id TEXT NOT NULL,
    -- A foreign key that DOES cascade, because deleting a pseudonym is a data
    -- erasure request. The trail is about a credential's use; the pseud's own
    -- data is not this instance's to keep after somebody asks for it to be gone,
    -- and an audit row keyed to a deleted pseud is a row about a person who no
    -- longer exists here.
    pseud_id      TEXT NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    -- created | used | tested | failed | revoked | renewed | expired
    event         TEXT NOT NULL
                 CHECK (event IN ('created', 'used', 'tested', 'failed',
                                  'revoked', 'renewed', 'expired')),
    occurred_at   TEXT NOT NULL
);

CREATE INDEX IF NOT EXISTS credential_audit_credential
    ON credential_audit_events (credential_id, occurred_at);

-- The consent sweep: which pseuds hold a credential that needs re-consenting.
-- Partial on `consent_at IS NULL` because the sweep only ever asks about the
-- rows that lack one, and a full index would be read only to have almost all of
-- its entries discarded.
CREATE INDEX IF NOT EXISTS source_credentials_unconsented
    ON source_credentials (pseud_id) WHERE consent_at IS NULL;