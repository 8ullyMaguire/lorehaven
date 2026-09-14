-- M15 — Economy: credits, fair queues, bounties, billing

CREATE TABLE credit_transactions (
    id TEXT PRIMARY KEY,
    type TEXT NOT NULL,
    idempotency_key TEXT NOT NULL,
    reference TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE UNIQUE INDEX idx_credit_txn_idempotency ON credit_transactions(idempotency_key);

CREATE TABLE credit_entries (
    transaction_id TEXT NOT NULL,
    account TEXT NOT NULL,
    bucket TEXT NOT NULL,
    amount_bp INTEGER NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_credit_entries_account ON credit_entries(account, created_at);

CREATE TABLE credit_holds (
    id TEXT PRIMARY KEY,
    account TEXT NOT NULL,
    amount INTEGER NOT NULL,
    job_id TEXT NOT NULL,
    expires_at TEXT NOT NULL,
    released_at TEXT,
    captured_at TEXT
);
CREATE INDEX idx_credit_holds_job ON credit_holds(job_id);

CREATE TABLE queue_slots (
    job_id TEXT PRIMARY KEY,
    priority_class TEXT NOT NULL,
    position INTEGER NOT NULL,
    enqueued_at TEXT NOT NULL
);

CREATE TABLE bounties (
    id TEXT PRIMARY KEY,
    job_kind TEXT NOT NULL,
    terms TEXT NOT NULL,
    escrow_transaction TEXT NOT NULL,
    state TEXT NOT NULL,
    claimant TEXT,
    created_by TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE subscriptions (
    account TEXT NOT NULL,
    tier TEXT NOT NULL,
    state TEXT NOT NULL,
    period_end TEXT NOT NULL,
    provider TEXT,
    external_ref TEXT,
    PRIMARY KEY (account, tier)
);

CREATE TABLE usage_counters (
    account TEXT NOT NULL,
    action TEXT NOT NULL,
    day TEXT NOT NULL,
    count INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (account, action, day)
);
