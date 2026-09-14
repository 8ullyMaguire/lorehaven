-- M19 — Administration, statistics, abuse defence, privacy, operations

CREATE TABLE admin_actions (
    id TEXT PRIMARY KEY,
    actor TEXT NOT NULL,
    action TEXT NOT NULL,
    subject_type TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    document TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE INDEX idx_admin_actions_actor ON admin_actions(actor, created_at);

CREATE TABLE feature_flags (
    key TEXT PRIMARY KEY,
    state TEXT NOT NULL,
    rollout_bp INTEGER NOT NULL DEFAULT 0,
    note TEXT NOT NULL,
    updated_by TEXT NOT NULL,
    updated_at TEXT NOT NULL
);

CREATE TABLE announcements (
    id TEXT PRIMARY KEY,
    body TEXT NOT NULL,
    level TEXT NOT NULL,
    starts_at TEXT NOT NULL,
    ends_at TEXT,
    created_by TEXT NOT NULL
);

CREATE TABLE stat_snapshots (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    period TEXT NOT NULL,
    document TEXT NOT NULL,
    computed_at TEXT NOT NULL,
    UNIQUE (kind, period)
);

CREATE TABLE privacy_requests (
    id TEXT PRIMARY KEY,
    account TEXT NOT NULL,
    kind TEXT NOT NULL,
    state TEXT NOT NULL,
    requested_at TEXT NOT NULL,
    completed_at TEXT,
    result_ref TEXT
);
CREATE INDEX idx_privacy_requests_account ON privacy_requests(account, state);

CREATE TABLE abuse_counters (
    key TEXT NOT NULL,
    window TEXT NOT NULL,
    count INTEGER NOT NULL DEFAULT 0,
    blocked_until TEXT,
    PRIMARY KEY (key, window)
);

CREATE TABLE ip_policy (
    ip TEXT PRIMARY KEY,
    state TEXT NOT NULL,
    reason TEXT,
    set_by TEXT NOT NULL,
    set_at TEXT NOT NULL
);

CREATE TABLE signup_controls (
    id TEXT PRIMARY KEY CHECK (id = 'singleton'),
    mode TEXT NOT NULL,
    challenge INTEGER NOT NULL DEFAULT 0,
    updated_by TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
