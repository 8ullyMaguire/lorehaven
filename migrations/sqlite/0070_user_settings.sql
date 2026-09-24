-- Migration 0070 — M47: User Configuration (spec §46)
--
-- Per-domain settings tables: search_settings, content_filters,
-- notification_routes. No JSONB mega-table; follows migration 0002 pattern.
--
-- Dialect: SQLite.
-- Timestamps are RFC 3339 UTC text; identifiers are canonical UUID text.

CREATE TABLE IF NOT EXISTS search_settings (
    id              TEXT    PRIMARY KEY,
    pseud_id        TEXT    NOT NULL REFERENCES pseuds(id) ON DELETE CASCADE,
    key             TEXT    NOT NULL,
    value           TEXT NOT NULL,
    created_at      TEXT    NOT NULL,
    updated_at      TEXT    NOT NULL,
    UNIQUE (pseud_id, key)
);
CREATE INDEX IF NOT EXISTS idx_search_settings_pseud ON search_settings (pseud_id);

CREATE TABLE IF NOT EXISTS content_filters (
    id              TEXT    PRIMARY KEY,
    pseud_id        TEXT    NOT NULL REFERENCES pseuds(id) ON DELETE CASCADE,
    filter_type     TEXT    NOT NULL CHECK (filter_type IN ('tag','fandom','warning')),
    value           TEXT    NOT NULL,
    created_at      TEXT    NOT NULL,
    updated_at      TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_content_filters_pseud ON content_filters (pseud_id);

CREATE TABLE IF NOT EXISTS notification_routes (
    id              TEXT    PRIMARY KEY,
    account_id      TEXT    NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    event_type      TEXT    NOT NULL,
    channel         TEXT    NOT NULL,
    enabled         BOOLEAN NOT NULL DEFAULT 1,
    created_at      TEXT    NOT NULL,
    updated_at      TEXT    NOT NULL,
    UNIQUE (account_id, event_type)
);
CREATE INDEX IF NOT EXISTS idx_notification_routes_account ON notification_routes (account_id);
