-- 0061_curator_roles: media curator role (spec §32.7.5).
--
-- Dialect: SQLite.

CREATE TABLE IF NOT EXISTS curator_roles (
    account_id      TEXT PRIMARY KEY,
    opted_in_at     TEXT NOT NULL DEFAULT (datetime('now')),
    opted_out_at    TEXT,
    opted_in_by     TEXT NOT NULL DEFAULT 'self',
    UNIQUE (account_id)
);

CREATE INDEX IF NOT EXISTS idx_curator_roles_active ON curator_roles(account_id) WHERE opted_out_at IS NULL;

CREATE TABLE IF NOT EXISTS link_verifications (
    id                      TEXT PRIMARY KEY,
    availability_link_id    TEXT NOT NULL,
    media_reference_id      TEXT NOT NULL,
    curator_id              TEXT NOT NULL,
    verification_type       TEXT NOT NULL,
    confidence              REAL NOT NULL DEFAULT 1.0,
    created_at              TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (availability_link_id, curator_id)
);

CREATE INDEX IF NOT EXISTS idx_link_verifications_link ON link_verifications(availability_link_id);
CREATE INDEX IF NOT EXISTS idx_link_verifications_curator ON link_verifications(curator_id);
