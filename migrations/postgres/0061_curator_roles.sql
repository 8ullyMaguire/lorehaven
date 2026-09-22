-- 0061_curator_roles: media curator role (spec §32.7.5).
--
-- Dialect: PostgreSQL.
--
-- A distinct role from Vanguard. Curators maintain media availability:
-- add mirrors, verify links, rescue dead references. Opt-in for TL3+.

CREATE TABLE IF NOT EXISTS curator_roles (
    account_id      TEXT PRIMARY KEY,
    opted_in_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    opted_out_at    TIMESTAMPTZ,
    opted_in_by     TEXT NOT NULL DEFAULT 'self',
    UNIQUE (account_id)
);

CREATE INDEX IF NOT EXISTS idx_curator_roles_active ON curator_roles(account_id) WHERE opted_out_at IS NULL;

-- Link verification quorum: track which curators have verified a specific
-- availability_link. Requires 2 independent curators for perceptual matches
-- with distance 3-6.
CREATE TABLE IF NOT EXISTS link_verifications (
    id                      TEXT PRIMARY KEY,
    availability_link_id    TEXT NOT NULL,
    media_reference_id      TEXT NOT NULL,
    curator_id              TEXT NOT NULL,
    verification_type       TEXT NOT NULL,   -- 'exact_match' | 'perceptual_match' | 'reverify'
    confidence              REAL NOT NULL DEFAULT 1.0,  -- 0.0-1.0 (1.0 = exact, <1.0 = perceptual distance)
    created_at              TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (availability_link_id, curator_id)
);

CREATE INDEX IF NOT EXISTS idx_link_verifications_link ON link_verifications(availability_link_id);
CREATE INDEX IF NOT EXISTS idx_link_verifications_curator ON link_verifications(curator_id);
