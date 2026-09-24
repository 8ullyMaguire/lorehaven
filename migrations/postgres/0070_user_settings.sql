-- Migration 0070 — M47: User Configuration (spec §46)
--
-- Per-domain settings tables for search_settings, content_filters, and
-- notification_routes. No JSONB mega-table; follows migration 0002 pattern.
--
-- Dialect: PostgreSQL.
-- Timestamps are RFC 3339 UTC text; identifiers are UUID text.

-- Search defaults per pseud: pre-populate every search surface.
CREATE TABLE IF NOT EXISTS search_settings (
    id              UUID PRIMARY KEY,
    pseud_id        UUID NOT NULL REFERENCES pseuds(id) ON DELETE CASCADE,
    key             TEXT NOT NULL,
    value           JSONB NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL,
    UNIQUE (pseud_id, key)
);
CREATE INDEX IF NOT EXISTS idx_search_settings_pseud ON search_settings (pseud_id);

-- Content filters per pseud: blocked tags/fandoms/warnings, server-enforced.
CREATE TABLE IF NOT EXISTS content_filters (
    id              UUID PRIMARY KEY,
    pseud_id        UUID NOT NULL REFERENCES pseuds(id) ON DELETE CASCADE,
    filter_type     TEXT NOT NULL CHECK (filter_type IN ('tag','fandom','warning')),
    value           TEXT NOT NULL,
    created_at      TIMESTAMPTZ NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_content_filters_pseud ON content_filters (pseud_id);

-- Notification routes per account: per-event channel routing (spec §17.5).
CREATE TABLE IF NOT EXISTS notification_routes (
    id              UUID PRIMARY KEY,
    account_id      UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    event_type      TEXT NOT NULL,
    channel         TEXT NOT NULL,
    enabled         BOOLEAN NOT NULL DEFAULT true,
    created_at      TIMESTAMPTZ NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL,
    UNIQUE (account_id, event_type)
);
CREATE INDEX IF NOT EXISTS idx_notification_routes_account ON notification_routes (account_id);
