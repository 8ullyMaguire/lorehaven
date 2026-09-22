-- 0062_author_media_tools: media health dashboard & preferences (spec §32.7.8).
--
-- Dialect: PostgreSQL.

CREATE TABLE IF NOT EXISTS author_media_preferences (
    account_id                  TEXT PRIMARY KEY,
    auto_submit_to_archive      BOOLEAN NOT NULL DEFAULT TRUE,
    prefer_curator_verified      BOOLEAN NOT NULL DEFAULT TRUE,
    broken_link_notifications   TEXT NOT NULL DEFAULT 'digest_weekly',
    allow_curator_edits          BOOLEAN NOT NULL DEFAULT TRUE,
    minimum_healthy_links       INTEGER NOT NULL DEFAULT 3,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Author-funded targeted bounties: spend credits to post bounties for
-- specific works (e.g., "50 credits to whoever finds a working mirror
-- for the moodboard in chapter 7").
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
    claimed_at                  TIMESTAMPTZ,
    created_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at                  TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_targeted_bounties_work ON targeted_bounties(work_id);
CREATE INDEX IF NOT EXISTS idx_targeted_bounties_account ON targeted_bounties(account_id);
CREATE INDEX IF NOT EXISTS idx_targeted_bounties_status ON targeted_bounties(status);
