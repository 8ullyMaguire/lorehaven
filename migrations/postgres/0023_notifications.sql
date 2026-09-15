-- §5.5 Integrations — the notifications inbox.
--
-- The SvelteKit shell has rendered Notifications.svelte against
-- GET /api/v1/notifications since Milestone 1; this table is the backend
-- catching up (M12 scope: notifications become real on the first surfaces
-- that produce them — forum replies, sales, gifts).
--
-- Convention note (per 63f22db): TEXT timestamps, INTEGER counters, JSON as
-- TEXT — identical column shapes to the SQLite dialect.
--
-- Retention: rows are private reader state and cascade with the account.
-- `read_at` NULL means unread; marking read is idempotent.

CREATE TABLE IF NOT EXISTS notifications (
    id          TEXT PRIMARY KEY,
    account_id  UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    kind        TEXT NOT NULL,             -- reply | sale | gift | mention | system
    title       TEXT NOT NULL,
    body        TEXT NOT NULL,
    work_id     UUID,
    read_at     TEXT,
    created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_notifications_account
    ON notifications(account_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_notifications_unread
    ON notifications(account_id) WHERE read_at IS NULL;
