-- M18 — Public API, bots, feeds, push, federation, AI providers
-- Note: api_tokens table already exists from migration 0001_identity

CREATE TABLE IF NOT EXISTS bot_registrations (
    id TEXT PRIMARY KEY,
    token_id TEXT NOT NULL,
    owner TEXT NOT NULL,
    contact TEXT NOT NULL,
    user_agent TEXT NOT NULL,
    state TEXT NOT NULL,
    registered_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS feed_handles (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    subject TEXT NOT NULL,
    handle TEXT NOT NULL UNIQUE,
    created_at TEXT NOT NULL
);

CREATE TABLE IF NOT EXISTS push_subscriptions (
    id TEXT PRIMARY KEY,
    account TEXT NOT NULL,
    endpoint TEXT NOT NULL,
    keys TEXT NOT NULL,
    device_name TEXT,
    created_at TEXT NOT NULL,
    revoked_at TEXT
);
CREATE INDEX idx_push_subscriptions_account ON push_subscriptions(account);

CREATE TABLE IF NOT EXISTS federation_peers (
    id TEXT PRIMARY KEY,
    host TEXT NOT NULL,
    direction TEXT NOT NULL,
    reason TEXT,
    set_by TEXT NOT NULL,
    set_at TEXT NOT NULL,
    UNIQUE (host)
);

CREATE TABLE IF NOT EXISTS federation_inbound (
    id TEXT PRIMARY KEY,
    peer_host TEXT NOT NULL,
    object_type TEXT NOT NULL,
    object_id TEXT NOT NULL,
    received_at TEXT NOT NULL,
    state TEXT NOT NULL,
    note TEXT
);
CREATE INDEX idx_federation_inbound_peer ON federation_inbound(peer_host, received_at);

CREATE TABLE IF NOT EXISTS ai_consents (
    id TEXT PRIMARY KEY,
    work_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    consent TEXT NOT NULL,
    granted_by TEXT NOT NULL,
    created_at TEXT NOT NULL,
    revoked_at TEXT,
    UNIQUE (work_id, provider)
);

CREATE TABLE IF NOT EXISTS ai_requests (
    id TEXT PRIMARY KEY,
    work_id TEXT NOT NULL,
    provider TEXT NOT NULL,
    account TEXT,
    purpose TEXT NOT NULL,
    charged_transaction TEXT,
    requested_at TEXT NOT NULL,
    served_at TEXT
);
CREATE INDEX idx_ai_requests_work ON ai_requests(work_id, requested_at);
