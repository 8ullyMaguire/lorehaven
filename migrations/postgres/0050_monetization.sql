-- Migration 0050 — monetization: pricing, entitlements, earnings, payouts, AI
-- declarations, payment events (spec §20.9), redistribution pools (spec §20.10).
--
-- Dialect: PostgreSQL.

-- Work pricing (spec §20.9.2). One row per priced work; DELETE disables.
CREATE TABLE work_pricing (
    work_id          UUID     NOT NULL PRIMARY KEY,
    model            TEXT     NOT NULL,              -- tips | early_access | purchase | patronage
    price_minor      INTEGER  NOT NULL,              -- in currency minor units
    currency         TEXT     NOT NULL DEFAULT 'EUR',
    public_at_offset_secs INTEGER,                   -- early_access: seconds until free
    enabled          BOOLEAN  NOT NULL DEFAULT TRUE,
    version          INTEGER  NOT NULL DEFAULT 1,    -- bumped on every change
    created_at       TEXT     NOT NULL,
    updated_at       TEXT     NOT NULL
);

CREATE INDEX work_pricing_enabled ON work_pricing (enabled);

-- Durable entitlements (spec §20.9.3): purchase/patronage bound to the account.
CREATE TABLE work_entitlements (
    id                UUID    NOT NULL PRIMARY KEY,
    account_id        UUID    NOT NULL,
    work_id           UUID    NOT NULL,
    kind              TEXT    NOT NULL,              -- purchase | patronage
    source_payment_id UUID,
    granted_at        TEXT    NOT NULL,
    expires_at        TEXT,                          -- NULL = permanent
    UNIQUE (account_id, work_id, kind)
);

CREATE INDEX work_entitlements_account ON work_entitlements (account_id);
CREATE INDEX work_entitlements_work ON work_entitlements (work_id);

-- Author earnings ledger (spec §20.9.3): money, separate from credits.
CREATE TABLE author_earnings_ledger (
    id               UUID    NOT NULL PRIMARY KEY,
    author_account   UUID    NOT NULL,
    amount_minor     INTEGER NOT NULL,               -- signed
    currency         TEXT    NOT NULL DEFAULT 'EUR',
    kind             TEXT    NOT NULL,               -- pool_a | pool_b | tip | payout | adjustment
    pool             TEXT    NOT NULL DEFAULT 'a',   -- a | b
    payment_id       UUID,
    idempotency_key  TEXT    NOT NULL UNIQUE,
    created_at       TEXT    NOT NULL
);

CREATE INDEX author_earnings_author ON author_earnings_ledger (author_account);
CREATE INDEX author_earnings_pool ON author_earnings_ledger (pool);

-- Payouts (spec §20.9.3): author-initiated, processor-referenced.
CREATE TABLE payouts (
    id                   UUID    NOT NULL PRIMARY KEY,
    author_account       UUID    NOT NULL,
    amount_minor         INTEGER NOT NULL,
    currency             TEXT    NOT NULL DEFAULT 'EUR',
    processor_reference  TEXT,
    status               TEXT    NOT NULL DEFAULT 'pending',  -- pending | paid | failed
    initiated_at         TEXT    NOT NULL,
    completed_at         TEXT
);

CREATE INDEX payouts_author ON payouts (author_account);

-- Monetization assertions (spec §20.9.1): versioned author responsibility statements.
CREATE TABLE monetization_assertions (
    id               UUID    NOT NULL PRIMARY KEY,
    work_id          UUID    NOT NULL,
    assertion_kind   TEXT    NOT NULL,               -- original | rights-held
    policy_version   INTEGER NOT NULL,
    accepted_at      TEXT    NOT NULL,
    revoked_at       TEXT
);

CREATE INDEX monetization_assertions_work ON monetization_assertions (work_id);

-- AI content declaration (spec §20.9.4): versioned, per work.
CREATE TABLE work_ai_declarations (
    work_id      UUID    NOT NULL PRIMARY KEY,
    declaration  TEXT    NOT NULL,                   -- none | assisted | co-written | generated
    declared_at  TEXT    NOT NULL,
    revised_at   TEXT
);

-- Payment events (spec §20.9.5): every monetary transaction with its processor fee.
CREATE TABLE payment_events (
    id                  UUID    NOT NULL PRIMARY KEY,
    kind                TEXT    NOT NULL,            -- tip | purchase | subscription | payout
    account_id          UUID,                        -- payer, NULL for system flows
    work_id             UUID,
    author_account      UUID,
    amount_minor        INTEGER NOT NULL,            -- gross
    processor_fee_minor INTEGER NOT NULL DEFAULT 0,
    currency            TEXT    NOT NULL DEFAULT 'EUR',
    net_minor           INTEGER NOT NULL,            -- gross - fee
    created_at          TEXT    NOT NULL
);

CREATE INDEX payment_events_author ON payment_events (author_account);
CREATE INDEX payment_events_created ON payment_events (created_at);
