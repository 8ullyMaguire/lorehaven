-- 0062_author_media_tools: media health dashboard & preferences (spec §32.7.8).
--
-- Dialect: SQLite.

CREATE TABLE IF NOT EXISTS author_media_preferences (
    account_id                  TEXT PRIMARY KEY,
    auto_submit_to_archive      INTEGER NOT NULL DEFAULT 1,
    prefer_curator_verified      INTEGER NOT NULL DEFAULT 1,
    broken_link_notifications   TEXT NOT NULL DEFAULT 'digest_weekly',
    allow_curator_edits          INTEGER NOT NULL DEFAULT 1,
    minimum_healthy_links       INTEGER NOT NULL DEFAULT 3,
    created_at                  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at                  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE IF NOT EXISTS targeted_bounties (
    id                          TEXT PRIMARY KEY,
    work_id                     TEXT NOT NULL,
    chapter_id                  TEXT,
    media_reference_id          TEXT,
    account_id                  TEXT NOT NULL,
    reward                      INTEGER NOT NULL,
    status                      TEXT NOT NULL DEFAULT 'open',
    description                 TEXT,
    claimed_by                  TEXT,
    claimed_at                  TEXT,
    created_at                  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at                  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_targeted_bounties_work ON targeted_bounties(work_id);
CREATE INDEX IF NOT EXISTS idx_targeted_bounties_account ON targeted_bounties(account_id);
CREATE INDEX IF NOT EXISTS idx_targeted_bounties_status ON targeted_bounties(status);
