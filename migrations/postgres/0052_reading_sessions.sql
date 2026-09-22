-- M15 reading sessions — time-on-page for subscription revenue attribution
-- (spec §20.6.1). Append-only; aggregated at settlement.
--
-- Convention note (per 63f22db): TEXT timestamps, INTEGER counters.

CREATE TABLE IF NOT EXISTS reading_sessions (
    id              UUID PRIMARY KEY,
    account_id      UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    work_id         UUID NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    seconds         BIGINT NOT NULL DEFAULT 0,
    started_at      TEXT NOT NULL,
    ended_at        TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_reading_sessions_work ON reading_sessions(work_id, started_at);
CREATE INDEX IF NOT EXISTS idx_reading_sessions_account ON reading_sessions(account_id, started_at);
