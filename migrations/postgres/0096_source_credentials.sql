-- M53-03: the three §11.6 vault properties the 0006 schema does not carry.
--
-- Dialect: PostgreSQL.
--
-- The counterpart of migrations/sqlite/0096_source_credentials.sql. Tables,
-- columns and indexes must match, because
-- `the_two_dialects_declare_the_same_columns_and_indexes` in
-- crates/db/src/migrate.rs compares exactly that. The SQLite file carries the
-- reasoning -- including why this alters a table rather than creating one, and
-- why the plan's `ciphertext BLOB` and `UNIQUE (pseud_id, source_key)` are not
-- added -- and it is not repeated here.
--
-- What differs, and it is the asymmetry 0094 records: `pseuds.id` is a real UUID
-- in this schema and TEXT in the SQLite one, so the two references take UUID
-- here and TEXT there. That is the only reason these files are not textually
-- identical past the comment, and it is why a migration edited in one dialect
-- has to be edited in the other.

ALTER TABLE source_credentials ADD COLUMN kind TEXT
    CHECK (kind IS NULL OR kind IN ('token', 'password', 'session_cookie'));

-- Origin-bound credential use. NULL, not backfilled -- see the SQLite file.
ALTER TABLE source_credentials ADD COLUMN origin_host TEXT;

-- Explicit consent, rewritten on every store rather than carried forward.
ALTER TABLE source_credentials ADD COLUMN consent_at TEXT;

-- Audit events without secret contents. No column for the ciphertext and no
-- free-text field that could be used to smuggle one.
CREATE TABLE credential_audit_events (
    id            UUID PRIMARY KEY,
    -- Not a foreign key on purpose: a cascade would delete the audit row of the
    -- revocation that caused it. See the SQLite file for the argument in full.
    credential_id UUID NOT NULL,
    -- This one does cascade: deleting a pseud is an erasure request.
    pseud_id      UUID NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    event         TEXT NOT NULL
                  CHECK (event IN ('created', 'used', 'tested', 'failed',
                                   'revoked', 'renewed', 'expired')),
    occurred_at   TEXT NOT NULL
);

CREATE INDEX credential_audit_credential
    ON credential_audit_events (credential_id, occurred_at);

-- The consent sweep, partial on the rows that actually lack a consent record.
CREATE INDEX source_credentials_unconsented
    ON source_credentials (pseud_id) WHERE consent_at IS NULL;