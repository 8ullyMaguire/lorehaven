-- M46: Work card aggregate metrics — views, complete reads, reactions, kudos,
-- bookmarks, collection adds, review count (spec §9.5, §9.4).
--
-- work_view_log          deduplicated view events: one row per work per viewer
--                       per hour, so repeated refreshes do not inflate views
--                       (spec §10 acceptance). viewer_hash is the account id
--                       when signed in, otherwise a salted hash of IP+UA.
-- work_kudos            one row per account per work — "count once per reader
--                       per target" (spec §9.4).
-- work_metric_aggregates materialized counters maintained incrementally by the
--                       application layer (avoids trigger dialect drift), one
--                       row per work, created lazily on first event.

CREATE TABLE IF NOT EXISTS work_view_log (
    work_id      TEXT NOT NULL,
    viewer_hash  TEXT NOT NULL,
    viewed_at    TEXT NOT NULL,
    is_automated INTEGER NOT NULL DEFAULT 0,
    PRIMARY KEY (work_id, viewer_hash, viewed_at)
);
CREATE INDEX IF NOT EXISTS idx_work_view_log_work ON work_view_log (work_id);

CREATE TABLE IF NOT EXISTS work_kudos (
    work_id    UUID NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    account_id UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    created_at TEXT NOT NULL,
    PRIMARY KEY (work_id, account_id)
);
CREATE INDEX IF NOT EXISTS idx_work_kudos_work ON work_kudos (work_id);

CREATE TABLE IF NOT EXISTS work_metric_aggregates (
    work_id         UUID PRIMARY KEY,
    views           INTEGER NOT NULL DEFAULT 0,
    complete_reads  INTEGER NOT NULL DEFAULT 0,
    reactions       INTEGER NOT NULL DEFAULT 0,
    kudos           INTEGER NOT NULL DEFAULT 0,
    bookmarks       INTEGER NOT NULL DEFAULT 0,
    collection_adds INTEGER NOT NULL DEFAULT 0,
    reviews         INTEGER NOT NULL DEFAULT 0,
    updated_at      TEXT NOT NULL,
    FOREIGN KEY (work_id) REFERENCES works(id) ON DELETE CASCADE
);