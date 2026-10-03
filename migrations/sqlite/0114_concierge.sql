-- M45-22: the personal concierge (spec §54).
--
-- Two tables. `concierge_sessions` is one rendered queue per request, recording the
-- reader's *stated intent* and what the ranker returned; `wip_watches` is a reader
-- watching a work-in-progress.
--
-- The design constraint that decides both: **a session is a record of what was
-- rendered, never a ranking input.** §54.1 refuses to make the concierge a second
-- ranker, and the way to guarantee that is to store the work ids as the rendering
-- they were rather than as a signal anything downstream reads. Nothing here is
-- ever joined into §16's blend.

CREATE TABLE IF NOT EXISTS concierge_sessions (
    id                TEXT PRIMARY KEY,
    account_id        TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    -- A §15.8 mood key, stored as the reader wrote it rather than as a taxonomy id,
    -- because the taxonomy is community-extensible and a canonical key survives the id
    -- churn an extension causes. NULL means "no mood selector", which is the default
    -- queue and not an error (§54.6).
    mood              TEXT,
    -- Minutes the reader stated they had. NULL when they stated none, and then nothing
    -- is cut — which is a different fact from "cut nothing because nothing overflowed",
    -- and has to be representable.
    budget_minutes    INTEGER,
    -- What the queue returned, in order, as a JSON array of work ids. A rendering, not
    -- a feed: re-ranking it later would make it one, which §54.1 refuses.
    work_ids          TEXT NOT NULL DEFAULT '[]',
    -- Total estimated minutes of the returned items. §54.4 requires the queue to state
    -- its estimate rather than leave the reader to infer a budget from a length.
    estimated_minutes REAL,
    -- Index the budget bound at, or NULL when the whole blended queue fit. NULL here
    -- with a non-NULL budget means "nothing was cut", which is not the same fact as
    -- "cut at index 0" and cannot be stored in the same value.
    truncated_at      INTEGER,
    -- Whether the estimate used the reader's observed reading speed or the instance
    -- default (§54.4). Stored because a reader who disputes the cut needs to know
    -- which number was used to cut it.
    rate_source       TEXT NOT NULL DEFAULT 'default'
                      CHECK (rate_source IN ('observed', 'default')),
    created_at        TEXT NOT NULL
);

-- §11.15's `cache | aggregate` split makes this `cache`: it is a record of intention,
-- dropped on the instance's own schedule. Nothing derives from it after the window.
CREATE INDEX IF NOT EXISTS concierge_sessions_account
    ON concierge_sessions (account_id, created_at DESC);

-- A WIP watch (§54.5).
--
-- The UNIQUE is the guarantee that a reader cannot stack two watches on one work and
-- be notified twice. That is not a convenience constraint: `add_watch` uses
-- `ON CONFLICT (account_id, work_id) DO NOTHING`, so without the UNIQUE that clause
-- has nothing to name and a second watch would be a second row that also notifies.
CREATE TABLE IF NOT EXISTS wip_watches (
    id                TEXT PRIMARY KEY,
    account_id        TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    work_id           TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    -- Set when the notification was written. NULL is the pending state and the partial
    -- index below exists to serve exactly the "who is waiting on this work" query.
    notified_at       TEXT,
    created_at        TEXT NOT NULL,
    UNIQUE (account_id, work_id)
);

-- Partial on purpose: the completion transition asks for `notified_at IS NULL` and the
-- index should not carry the rows that have already been answered.
CREATE INDEX IF NOT EXISTS wip_watches_pending ON wip_watches (work_id) WHERE notified_at IS NULL;