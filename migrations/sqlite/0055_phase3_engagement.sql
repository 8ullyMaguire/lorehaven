-- M18 Phase 3 (Engagement Layer) — streaks, lifecycle, taste notifications.
--
-- Streaks track daily login presence. Flat milestone bonuses (7d, 30d) are
-- one-time credits that reward the habit without diluting the taste signal.
CREATE TABLE IF NOT EXISTS streaks (
    account_id          TEXT PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    current_streak      INTEGER NOT NULL DEFAULT 0,
    longest_streak      INTEGER NOT NULL DEFAULT 0,
    last_login_at       TEXT,
    streak_freezes_used INTEGER NOT NULL DEFAULT 0,
    updated_at          TEXT NOT NULL
);

-- One-time streak milestone awards (spec §9.7.1). Tracks which milestones have
-- already fired so they never double-pay.
CREATE TABLE IF NOT EXISTS streak_milestones (
    id          TEXT PRIMARY KEY,
    account_id  TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    milestone   TEXT NOT NULL CHECK (milestone IN ('7d', '30d')),
    awarded_at  TEXT NOT NULL,
    UNIQUE (account_id, milestone)
);

-- Lifecycle events (spec §9.8): completion bonus and resurrection reward.
-- Tracks which works have already triggered each event type.
CREATE TABLE IF NOT EXISTS lifecycle_events (
    id          TEXT PRIMARY KEY,
    work_id     TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    event_type  TEXT NOT NULL CHECK (event_type IN ('completion', 'resurrection')),
    triggered_at TEXT NOT NULL,
    UNIQUE (work_id, event_type)
);

-- Taste-weighted notification queue (spec §9.9). When a work is published,
-- aligned users are queued for notification. The worker drains this table.
CREATE TABLE IF NOT EXISTS taste_notification_queue (
    id          TEXT PRIMARY KEY,
    work_id     TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    account_id  TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    taste_score REAL NOT NULL,
    queued_at   TEXT NOT NULL,
    sent_at     TEXT,
    UNIQUE (work_id, account_id)
);
CREATE INDEX IF NOT EXISTS taste_notification_queue_queued ON taste_notification_queue (queued_at, sent_at);
CREATE INDEX IF NOT EXISTS taste_notification_queue_account ON taste_notification_queue (account_id, sent_at);
