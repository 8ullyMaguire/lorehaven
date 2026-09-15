-- M21 skeleton — tables for the 2026-09-14 spec revision.
-- Works monetization (spec §20.9), gifts (§18.10), subscriptions to content
-- (§23.3), saved-search alerts (§14.2), author `ai_training` assertions (§24.14).
--
-- Convention note (per 63f22db): TEXT timestamps, INTEGER counters, JSON as
-- TEXT — identical column shapes to the SQLite dialect. No TIMESTAMPTZ/JSONB.
--
-- Retention: entitlements are durable (survive pseud switching, never
-- retroactively removed); earnings and payouts are permanent money records,
-- never cascaded; content subscriptions and alerts are private reader state
-- and cascade with the account; gifts are public attribution and are
-- anonymised, not deleted, when a pseud leaves.

-- §20.9.2 — how a priced work is sold.
CREATE TABLE IF NOT EXISTS work_pricing (
    id               UUID PRIMARY KEY,
    work_id          UUID NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    model            TEXT NOT NULL,             -- tips | early_access | purchase | patronage
    price_minor      BIGINT NOT NULL DEFAULT 0,
    currency         TEXT NOT NULL DEFAULT 'EUR',
    public_at_offset TEXT,
    enabled          BOOLEAN NOT NULL DEFAULT TRUE,
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL,
    version          BIGINT NOT NULL DEFAULT 1,
    UNIQUE (work_id)
);
CREATE INDEX IF NOT EXISTS idx_work_pricing_work ON work_pricing(work_id);

-- §20.9.3 — durable access record; bound to the ACCOUNT so pseud switching
-- cannot lose a purchase.
CREATE TABLE IF NOT EXISTS work_entitlements (
    id               UUID PRIMARY KEY,
    account_id       UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    work_id          UUID NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    kind             TEXT NOT NULL,             -- purchase | patronage | early_access | gift
    source_payment_id TEXT,
    granted_at       TEXT NOT NULL,
    expires_at       TEXT,
    UNIQUE (account_id, work_id, kind)
);

-- §20.9.3 — author earnings are MONEY, a ledger append-only like credits.
CREATE TABLE IF NOT EXISTS author_earnings_ledger (
    id               UUID PRIMARY KEY,
    author_account_id UUID REFERENCES accounts (id) ON DELETE RESTRICT,
    amount_minor    BIGINT NOT NULL,
    currency        TEXT NOT NULL,
    kind            TEXT NOT NULL,              -- tip | sale | patronage_payout | platform_fee
    payment_id      TEXT,
    idempotency_key TEXT,
    created_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_author_earnings_author ON author_earnings_ledger(author_account_id, created_at);
CREATE UNIQUE INDEX IF NOT EXISTS idx_author_earnings_idem
    ON author_earnings_ledger(idempotency_key) WHERE idempotency_key IS NOT NULL;

-- §20.9.3 — payouts leave through the payment processor's flow.
CREATE TABLE IF NOT EXISTS payouts (
    id               UUID PRIMARY KEY,
    author_account_id  UUID NOT NULL REFERENCES accounts (id) ON DELETE RESTRICT,
    amount_minor       BIGINT NOT NULL,
    currency           TEXT NOT NULL,
    processor_reference TEXT,
    status             TEXT NOT NULL,           -- initiated | paid | failed | reversed
    initiated_at       TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_payouts_author ON payouts(author_account_id, initiated_at);

-- §20.9.1 — rights assertion records, re-demanded on price changes.
CREATE TABLE IF NOT EXISTS monetization_assertions (
    id               UUID PRIMARY KEY,
    work_id        UUID NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    assertion_kind TEXT NOT NULL,               -- original | rights-held
    policy_version TEXT NOT NULL,
    accepted_at    TEXT NOT NULL,
    revoked_at     TEXT
);

-- §18.10 — gifts and dedications: public attribution, one row per gift.
CREATE TABLE IF NOT EXISTS work_gifts (
    id               UUID PRIMARY KEY,
    work_id                 UUID NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    recipient_pseud_id      UUID REFERENCES pseuds (id) ON DELETE SET NULL,
    gift_note               TEXT,
    challenge_fulfillment_id TEXT,
    created_at              TEXT NOT NULL,
    declined_at             TEXT,
    UNIQUE (work_id, recipient_pseud_id)
);
CREATE INDEX IF NOT EXISTS idx_work_gifts_recipient ON work_gifts(recipient_pseud_id);

-- §23.3 — subscriptions to content (distinct from billing `subscriptions`
-- which stays in 0017). Per-pseud, private, pausable.
CREATE TABLE IF NOT EXISTS content_subscriptions (
    id               UUID PRIMARY KEY,
    subscriber_pseud_id UUID NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    subject_type   TEXT NOT NULL,               -- work | series | collection | fandom | author
    subject_id     TEXT NOT NULL,
    state          TEXT NOT NULL DEFAULT 'active',  -- active | paused
    created_at     TEXT NOT NULL,
    UNIQUE (subscriber_pseud_id, subject_type, subject_id)
);
CREATE INDEX IF NOT EXISTS idx_content_sub_subject ON content_subscriptions(subject_type, subject_id);

-- §14.2 — saved-search alerts: scheduled runs of a saved view.
CREATE TABLE IF NOT EXISTS search_alerts (
    id               UUID PRIMARY KEY,
    saved_search_id  UUID NOT NULL REFERENCES saved_views (id) ON DELETE CASCADE,
    owner_pseud_id   UUID NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    frequency        TEXT NOT NULL DEFAULT 'daily',       -- daily | weekly | monthly
    last_run_at      TEXT,
    created_at       TEXT NOT NULL,
    UNIQUE (owner_pseud_id, saved_search_id)
);

-- §24.14 — the author's stated AI-training preference, per work.
ALTER TABLE works ADD COLUMN ai_training TEXT NOT NULL DEFAULT 'unset';

-- §24.14 — per-author AI-training preference assertion (opt-in, not work-level).
CREATE TABLE IF NOT EXISTS author_ai_training (
    pseud_id   TEXT PRIMARY KEY,
    opt_in     BIGINT NOT NULL DEFAULT 0,      -- 0 = opt-out, 1 = opt-in
    updated_at TEXT NOT NULL
);
