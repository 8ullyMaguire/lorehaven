-- 0063_advanced_mirroring: local mirror storage & IPFS tracking (spec §32.7.6).
--
-- Dialect: SQLite.

CREATE TABLE IF NOT EXISTS local_mirrors (
    id                          TEXT PRIMARY KEY,
    media_reference_id          TEXT NOT NULL,
    storage_path                TEXT NOT NULL,
    original_url                TEXT NOT NULL,
    file_size_bytes             INTEGER NOT NULL,
    content_type                TEXT NOT NULL,
    checksum_sha256             TEXT NOT NULL,
    mirrored_at                 TEXT NOT NULL DEFAULT (datetime('now')),
    mirrored_by                 TEXT NOT NULL,
    status                      TEXT NOT NULL DEFAULT 'active',
    last_verified_at            TEXT,
    expires_at                  TEXT,
    UNIQUE (media_reference_id, checksum_sha256)
);

CREATE INDEX IF NOT EXISTS idx_local_mirrors_ref ON local_mirrors(media_reference_id);
CREATE INDEX IF NOT EXISTS idx_local_mirrors_status ON local_mirrors(status);

CREATE TABLE IF NOT EXISTS ipfs_pins (
    id                          TEXT PRIMARY KEY,
    media_reference_id          TEXT NOT NULL,
    cid                         TEXT NOT NULL UNIQUE,
    pin_service                 TEXT NOT NULL,
    status                      TEXT NOT NULL DEFAULT 'pinned',
    file_size_bytes             INTEGER NOT NULL,
    pinned_at                   TEXT NOT NULL DEFAULT (datetime('now')),
    last_replicated_at          TEXT
);

CREATE INDEX IF NOT EXISTS idx_ipfs_pins_ref ON ipfs_pins(media_reference_id);
CREATE INDEX IF NOT EXISTS idx_ipfs_pins_cid ON ipfs_pins(cid);

CREATE TABLE IF NOT EXISTS federated_mirrors (
    id                          TEXT PRIMARY KEY,
    media_reference_id          TEXT NOT NULL,
    instance_url                TEXT NOT NULL,
    mirror_url                  TEXT NOT NULL,
    status                      TEXT NOT NULL DEFAULT 'available',
    last_seen_at                TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE (media_reference_id, instance_url)
);

CREATE INDEX IF NOT EXISTS idx_federated_mirrors_ref ON federated_mirrors(media_reference_id);

CREATE TABLE IF NOT EXISTS dmca_takedowns (
    id                          TEXT PRIMARY KEY,
    local_mirror_id             TEXT NOT NULL,
    claimant_name               TEXT NOT NULL,
    claimant_email              TEXT NOT NULL,
    original_work_description   TEXT NOT NULL,
    complaint_text              TEXT NOT NULL,
    status                      TEXT NOT NULL DEFAULT 'pending',
    resolved_at                 TEXT,
    resolved_by                 TEXT,
    created_at                  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_dmca_takedowns_mirror ON dmca_takedowns(local_mirror_id);
