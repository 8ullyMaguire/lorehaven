-- Migration 0001 — identity core (spec §4.2).
--
-- Dialect: PostgreSQL.
-- Identifiers are native UUID columns; timestamps are RFC 3339 UTC text so the
-- repository layer decodes identically on both engines (see
-- docs/adr/0004-timestamp-and-identifier-storage.md).
--
-- Deletion and retention: accounts soft-delete via deleted_at so that audit
-- events keep resolving; hard deletion is an explicit operator action covered
-- by the account-deletion workflow (spec §22), which also expires backups.

CREATE TABLE accounts (
    id                UUID    PRIMARY KEY,
    status            TEXT    NOT NULL DEFAULT 'active',
    email             TEXT    NOT NULL,
    email_verified_at TEXT,
    age_state         TEXT    NOT NULL DEFAULT 'unknown',
    created_at        TEXT    NOT NULL,
    updated_at        TEXT    NOT NULL,
    version           BIGINT NOT NULL DEFAULT 1,
    deleted_at        TEXT
);

CREATE UNIQUE INDEX accounts_email_normalized ON accounts (lower(email));
CREATE INDEX accounts_status_created_at ON accounts (status, created_at);

CREATE TABLE password_credentials (
    account_id    UUID PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    password_hash TEXT NOT NULL,
    algorithm     TEXT NOT NULL DEFAULT 'argon2id',
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL
);

CREATE TABLE sessions (
    id              UUID PRIMARY KEY,
    token_hash      TEXT NOT NULL,
    account_id      UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    csrf_token_hash TEXT NOT NULL,
    user_agent      TEXT,
    created_at      TEXT NOT NULL,
    last_seen_at    TEXT NOT NULL,
    expires_at      TEXT NOT NULL,
    revoked_at      TEXT
);

CREATE UNIQUE INDEX sessions_token_hash ON sessions (token_hash);
CREATE INDEX sessions_account_expiry ON sessions (account_id, expires_at);

CREATE TABLE recovery_tokens (
    token_hash TEXT PRIMARY KEY,
    account_id UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    purpose    TEXT NOT NULL,
    created_at TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    used_at    TEXT
);

CREATE INDEX recovery_tokens_account_purpose ON recovery_tokens (account_id, purpose);

CREATE TABLE second_factors (
    account_id       UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    factor_type      TEXT NOT NULL,
    encrypted_secret TEXT NOT NULL,
    confirmed_at     TEXT,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL,
    PRIMARY KEY (account_id, factor_type)
);

CREATE TABLE pseuds (
    id              UUID    PRIMARY KEY,
    account_id      UUID    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    handle          TEXT    NOT NULL,
    display_name    TEXT    NOT NULL,
    bio             TEXT,
    discoverability TEXT    NOT NULL DEFAULT 'listed',
    created_at      TEXT    NOT NULL,
    updated_at      TEXT    NOT NULL,
    version         BIGINT NOT NULL DEFAULT 1,
    deleted_at      TEXT
);

CREATE UNIQUE INDEX pseuds_handle_normalized ON pseuds (lower(handle));
CREATE INDEX pseuds_account ON pseuds (account_id, created_at);

CREATE TABLE public_pseud_links (
    source_pseud_id UUID NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    target_pseud_id UUID NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    created_at      TEXT NOT NULL,
    PRIMARY KEY (source_pseud_id, target_pseud_id)
);

CREATE INDEX public_pseud_links_target ON public_pseud_links (target_pseud_id);

CREATE TABLE privacy_settings (
    id         UUID PRIMARY KEY,
    account_id UUID REFERENCES accounts (id) ON DELETE CASCADE,
    pseud_id   UUID REFERENCES pseuds (id) ON DELETE CASCADE,
    key        TEXT NOT NULL,
    value      TEXT NOT NULL,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    CHECK ((account_id IS NULL) <> (pseud_id IS NULL))
);

CREATE UNIQUE INDEX privacy_settings_account_key
    ON privacy_settings (account_id, key) WHERE account_id IS NOT NULL;
CREATE UNIQUE INDEX privacy_settings_pseud_key
    ON privacy_settings (pseud_id, key) WHERE pseud_id IS NOT NULL;

CREATE TABLE age_assessments (
    id               UUID PRIMARY KEY,
    account_id       UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    age_band         TEXT NOT NULL,
    assurance_method TEXT NOT NULL,
    policy_version   TEXT NOT NULL,
    assessed_at      TEXT NOT NULL,
    created_at       TEXT NOT NULL
);

CREATE INDEX age_assessments_account ON age_assessments (account_id, assessed_at);

CREATE TABLE guardian_authorizations (
    id                     UUID PRIMARY KEY,
    account_id             UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    status                 TEXT NOT NULL DEFAULT 'pending',
    verification_reference TEXT,
    created_at             TEXT NOT NULL,
    granted_at             TEXT,
    expires_at             TEXT
);

CREATE INDEX guardian_authorizations_account ON guardian_authorizations (account_id, status);

CREATE TABLE blocks (
    source_pseud_id UUID NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    target_pseud_id UUID NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    created_at      TEXT NOT NULL,
    PRIMARY KEY (source_pseud_id, target_pseud_id)
);

CREATE INDEX blocks_target ON blocks (target_pseud_id);

CREATE TABLE mutes (
    id          UUID NOT NULL PRIMARY KEY,
    pseud_id    UUID NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    target_type TEXT NOT NULL,
    target_id   TEXT NOT NULL,
    created_at  TEXT NOT NULL
);

CREATE UNIQUE INDEX mutes_unique ON mutes (pseud_id, target_type, target_id);

CREATE TABLE api_tokens (
    id           UUID PRIMARY KEY,
    account_id   UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    name         TEXT NOT NULL,
    token_hash   TEXT NOT NULL,
    scopes       TEXT NOT NULL DEFAULT '',
    created_at   TEXT NOT NULL,
    last_used_at TEXT,
    expires_at   TEXT,
    revoked_at   TEXT
);

CREATE UNIQUE INDEX api_tokens_hash ON api_tokens (token_hash);
CREATE INDEX api_tokens_account ON api_tokens (account_id, created_at);
