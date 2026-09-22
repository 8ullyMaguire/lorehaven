-- 0060_media_resilience: media availability guarantee system (spec §32.7.1-4).
--
-- Dialect: SQLite.
--
-- New tables for the media reference graph: media_references, availability_links,
-- work_media_references, curator_rewards, link_health_checks.

CREATE TABLE IF NOT EXISTS media_references (
    id                          TEXT PRIMARY KEY,
    perceptual_hash             TEXT,
    content_hash                TEXT NOT NULL,
    media_kind                  TEXT NOT NULL DEFAULT 'image',
    first_seen_at               TEXT NOT NULL,
    width                       INTEGER,
    height                      INTEGER,
    duration_seconds            INTEGER,
    format                      TEXT,
    file_size_bytes             INTEGER,
    content_notes               TEXT NOT NULL DEFAULT '[]',
    curator_verified            INTEGER NOT NULL DEFAULT 0,
    created_at                  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at                  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_media_references_content_hash ON media_references(content_hash);
CREATE INDEX IF NOT EXISTS idx_media_references_perceptual_hash ON media_references(perceptual_hash);
CREATE INDEX IF NOT EXISTS idx_media_references_media_kind ON media_references(media_kind);

CREATE TABLE IF NOT EXISTS availability_links (
    id                          TEXT PRIMARY KEY,
    media_reference_id          TEXT NOT NULL,
    url                         TEXT NOT NULL,
    provider                    TEXT NOT NULL DEFAULT 'other',
    status                      TEXT NOT NULL DEFAULT 'pending_verification',
    last_checked_at             TEXT NOT NULL DEFAULT (datetime('now')),
    last_healthy_at             TEXT,
    consecutive_failures        INTEGER NOT NULL DEFAULT 0,
    added_by                    TEXT,
    verified_by                 TEXT NOT NULL DEFAULT '[]',
    reported_broken_by          TEXT NOT NULL DEFAULT '[]',
    priority                    INTEGER NOT NULL DEFAULT 0,
    failure_details             TEXT,
    created_at                  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at                  TEXT NOT NULL DEFAULT (datetime('now')),
    UNIQUE(media_reference_id, url)
);

CREATE INDEX IF NOT EXISTS idx_availability_links_reference ON availability_links(media_reference_id);
CREATE INDEX IF NOT EXISTS idx_availability_links_status ON availability_links(status);
CREATE INDEX IF NOT EXISTS idx_availability_links_priority ON availability_links(priority DESC);
CREATE INDEX IF NOT EXISTS idx_availability_links_provider ON availability_links(provider);
CREATE INDEX IF NOT EXISTS idx_availability_links_media_ref ON availability_links(media_reference_id, status);

CREATE TABLE IF NOT EXISTS work_media_references (
    id                          TEXT PRIMARY KEY,
    work_id                     TEXT NOT NULL,
    chapter_id                  TEXT,
    media_reference_id          TEXT NOT NULL,
    context                     TEXT NOT NULL DEFAULT 'reference',
    display_url                 TEXT NOT NULL,
    author_note                 TEXT,
    inserted_at                 TEXT NOT NULL DEFAULT (datetime('now')),
    deleted_at                  TEXT
);

CREATE INDEX IF NOT EXISTS idx_work_media_references_work ON work_media_references(work_id);
CREATE INDEX IF NOT EXISTS idx_work_media_references_chapter ON work_media_references(chapter_id);
CREATE INDEX IF NOT EXISTS idx_work_media_references_media_ref ON work_media_references(media_reference_id);
CREATE INDEX IF NOT EXISTS idx_work_media_references_work_context ON work_media_references(work_id, context);

CREATE TABLE IF NOT EXISTS curator_rewards (
    id                          TEXT PRIMARY KEY,
    account_id                  TEXT NOT NULL,
    action                      TEXT NOT NULL,
    media_reference_id          TEXT,
    availability_link_id        TEXT,
    amount                      INTEGER NOT NULL,
    created_at                  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_curator_rewards_account ON curator_rewards(account_id);
CREATE INDEX IF NOT EXISTS idx_curator_rewards_created ON curator_rewards(created_at);

CREATE TABLE IF NOT EXISTS link_health_checks (
    id                          TEXT PRIMARY KEY,
    availability_link_id        TEXT NOT NULL,
    status                      TEXT NOT NULL,
    response_time_ms            INTEGER,
    content_type                TEXT,
    http_status                 INTEGER,
    failure_reason             TEXT,
    created_at                  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_link_health_checks_link ON link_health_checks(availability_link_id);
CREATE INDEX IF NOT EXISTS idx_link_health_checks_created ON link_health_checks(created_at);

CREATE TABLE IF NOT EXISTS curator_standing_bounties (
    id                          TEXT PRIMARY KEY,
    name                        TEXT NOT NULL,
    provider                    TEXT,
    healthy_links_below         INTEGER,
    work_admin_rating_min       INTEGER,
    has_archive_link            INTEGER,
    reward                      INTEGER NOT NULL,
    enabled                     INTEGER NOT NULL DEFAULT 1,
    created_at                  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at                  TEXT NOT NULL DEFAULT (datetime('now'))
);
