-- Migration 0056 — monetization: AI declarations and payment events
-- (spec §20.9.4, §20.9.5).
--
-- Dialect: PostgreSQL.
--
-- Note: work_pricing, work_entitlements, author_earnings_ledger, payouts, and
-- monetization_assertions already exist (created in migration 0022).

CREATE TABLE IF NOT EXISTS work_ai_declarations (
    work_id      UUID    NOT NULL PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    declaration  TEXT    NOT NULL,                   -- none | assisted | co-written | generated
    declared_at  TEXT    NOT NULL,
    revised_at   TEXT
);

CREATE TABLE IF NOT EXISTS payment_events (
    id                  UUID    NOT NULL PRIMARY KEY,
    kind                TEXT    NOT NULL,            -- tip | purchase | subscription | payout
    account_id          UUID    REFERENCES accounts (id) ON DELETE SET NULL,
    work_id             UUID    REFERENCES works (id) ON DELETE SET NULL,
    author_account      UUID    REFERENCES accounts (id) ON DELETE SET NULL,
    amount_minor        INTEGER NOT NULL,            -- gross
    processor_fee_minor INTEGER NOT NULL DEFAULT 0,
    currency            TEXT    NOT NULL DEFAULT 'EUR',
    net_minor           INTEGER NOT NULL,            -- gross - fee
    created_at          TEXT    NOT NULL
);

CREATE INDEX IF NOT EXISTS payment_events_author ON payment_events (author_account);
CREATE INDEX IF NOT EXISTS payment_events_created ON payment_events (created_at);
