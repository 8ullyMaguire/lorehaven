-- §17.5 — Mention events.
--
-- A mention is created when a forum post or comment references @handle.
-- Mention creation checks visibility, blocks, unsolicited-contact
-- restrictions, rate limits, and minor-protective policy (spec §17.5).
--
-- `events` (plural) because a single post may mention several handles.

CREATE TABLE IF NOT EXISTS mention_events (
    id              TEXT PRIMARY KEY,
    source_type     TEXT NOT NULL,             -- 'forum_post' | 'comment'
    source_id       TEXT NOT NULL,
    mentioned_pseud UUID NOT NULL REFERENCES pseuds(id) ON DELETE CASCADE,
    mentioned_by    UUID NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    created_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_mention_events_pseud
    ON mention_events(mentioned_pseud, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_mention_events_source
    ON mention_events(source_type, source_id);
