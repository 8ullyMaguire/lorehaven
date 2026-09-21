-- M39 — Resource Directory (spec §39): community-curated ranked lists
-- of external resources and internal references.

CREATE TABLE directory_entries (
    id              TEXT PRIMARY KEY,          -- uuid
    list_id         TEXT NOT NULL,             -- directory_lists.id
    kind            TEXT NOT NULL,             -- 'external' | 'work' | 'author' | 'tag' | 'fandom'
    category        TEXT NOT NULL DEFAULT '',  -- for external entries (§39.2), '' for internal
    title           TEXT NOT NULL,
    url             TEXT NOT NULL DEFAULT '',  -- external only
    description     TEXT NOT NULL DEFAULT '',
    ref_id          TEXT,                      -- internal only: work/pseud/tag/fandom id
    tags_json       TEXT NOT NULL DEFAULT '[]',-- sorted unique lowercase tags
    submitted_by    TEXT NOT NULL,             -- account id
    approved_by     TEXT,                      -- NULL = pending
    removed_at      TEXT,                      -- NULL = live
    score           REAL NOT NULL DEFAULT 0,   -- denormalised weighted sum
    created_at      TEXT NOT NULL,             -- RFC 3339
    updated_at      TEXT NOT NULL
);
CREATE INDEX idx_directory_entries_list ON directory_entries (list_id, score DESC, created_at);
CREATE INDEX idx_directory_entries_submitter ON directory_entries (submitted_by, created_at DESC);
CREATE INDEX idx_directory_entries_pending ON directory_entries (approved_by) WHERE approved_by IS NULL;

CREATE TABLE directory_lists (
    id              TEXT PRIMARY KEY,          -- uuid
    slug            TEXT NOT NULL UNIQUE,      -- url-safe
    title           TEXT NOT NULL,
    description     TEXT NOT NULL DEFAULT '',
    kind            TEXT NOT NULL,             -- 'external' | 'works' | 'authors' | 'mixed'
    is_instance_list BOOLEAN NOT NULL DEFAULT FALSE,  -- shown on landing page
    position        INTEGER NOT NULL DEFAULT 0,   -- instance-list display order
    created_by      TEXT NOT NULL,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);

CREATE TABLE directory_votes (
    entry_id        TEXT NOT NULL,
    account_id      TEXT NOT NULL,
    vote_value      INTEGER NOT NULL CHECK (vote_value IN (-1, 1)),
    weight          REAL NOT NULL DEFAULT 1,   -- weight at vote time (recomputed on trust/taste change)
    voted_at        TEXT NOT NULL,
    PRIMARY KEY (entry_id, account_id)
);
CREATE INDEX idx_directory_votes_entry ON directory_votes (entry_id);
