-- M16 — Marketplace, extensions, webhooks, gallery

CREATE TABLE listings (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,             -- paid_work | commission | ask
    owner TEXT NOT NULL,
    work_id TEXT,                   -- paid_work
    terms TEXT NOT NULL,            -- versioned JSON: price points, turnaround
    state TEXT NOT NULL,            -- draft|active|paused|closed
    created_at TEXT NOT NULL
);
CREATE INDEX idx_listings_kind_state ON listings(kind, state, created_at);

CREATE TABLE commissions (
    id TEXT PRIMARY KEY,
    listing_id TEXT NOT NULL,
    client TEXT NOT NULL,
    state TEXT NOT NULL,            -- quoted|accepted|in_progress|delivered|accepted_final|refunded|disputed
    quote_transaction TEXT,
    delivery_work TEXT,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE extension_manifests (
    id TEXT PRIMARY KEY,            -- slug
    version TEXT NOT NULL,
    document TEXT NOT NULL,         -- the manifest, versioned, signed hash
    submitted_by TEXT NOT NULL,
    state TEXT NOT NULL,            -- pending|approved|rejected|revoked
    created_at TEXT NOT NULL
);
CREATE INDEX idx_extension_manifests_id_version ON extension_manifests(id, version);

CREATE TABLE extension_grants (
    account TEXT NOT NULL,
    manifest_id TEXT NOT NULL,
    version TEXT NOT NULL,
    capabilities TEXT NOT NULL,     -- JSON: the granted capability list
    granted_at TEXT NOT NULL,
    revoked_at TEXT,
    PRIMARY KEY (account, manifest_id)
);

CREATE TABLE webhook_endpoints (
    id TEXT PRIMARY KEY,
    owner TEXT NOT NULL,
    url TEXT NOT NULL,
    secret TEXT NOT NULL,           -- HMAC key; store like a password
    events TEXT NOT NULL,           -- JSON list of subscribed event types
    active INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
);

CREATE TABLE webhook_deliveries (
    id TEXT PRIMARY KEY,
    endpoint_id TEXT NOT NULL,
    event_id TEXT NOT NULL,
    payload TEXT NOT NULL,          -- bounded; the bound is configuration
    signature TEXT NOT NULL,
    attempted_at TEXT NOT NULL,
    status TEXT NOT NULL,           -- ok|retrying|failed
    attempts INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX idx_webhook_deliveries_endpoint ON webhook_deliveries(endpoint_id, attempted_at);

CREATE TABLE gallery_items (
    id TEXT PRIMARY KEY,
    work_id TEXT NOT NULL,
    owner TEXT NOT NULL,
    media_type TEXT NOT NULL,
    storage_key TEXT NOT NULL,      -- M5 object storage; presigned only
    alt_text TEXT NOT NULL,
    sanitized_document TEXT NOT NULL,  -- the sanitiser's gallery output
    created_at TEXT NOT NULL
);
CREATE INDEX idx_gallery_items_work ON gallery_items(work_id);
