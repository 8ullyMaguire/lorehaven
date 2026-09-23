-- M45: Roadmap consensus — Elo-ranked feature board (spec §44, ADR 0023).
--
-- Cards are seeded from docs/requirements.csv and user suggestions.
-- Elo is the community's (arena votes); stage is the operator's.
-- Cards are never deleted; rejected ones keep their history.

CREATE TABLE IF NOT EXISTS roadmap_cards (
    id              TEXT PRIMARY KEY,            -- UUID as text
    title           TEXT NOT NULL,
    category        TEXT NOT NULL DEFAULT 'general',
    stage           TEXT NOT NULL DEFAULT 'idea'
                    CHECK (stage IN ('idea','up_next','in_progress','finished',
                                     'shipped','medium_term','long_term','rejected')),
    elo_rating      REAL NOT NULL DEFAULT 1500.0,
    matches_played  INTEGER NOT NULL DEFAULT 0,
    times_best      INTEGER NOT NULL DEFAULT 0,
    times_worst     INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_roadmap_cards_stage ON roadmap_cards (stage);

CREATE TABLE IF NOT EXISTS roadmap_suggestions (
    id          TEXT PRIMARY KEY,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    raw_text    TEXT NOT NULL,
    card_id     TEXT REFERENCES roadmap_cards(id) ON DELETE SET NULL,
    created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_roadmap_suggestions_card ON roadmap_suggestions (card_id);

CREATE TABLE IF NOT EXISTS roadmap_ballots (
    id           TEXT PRIMARY KEY,
    card_ids     TEXT NOT NULL,   -- JSON array of 4 card ids
    served_elo   TEXT NOT NULL,   -- JSON map card_id -> pre-match Elo
    account_id   TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    created_at   TEXT NOT NULL,
    voted_at     TEXT
);
CREATE INDEX IF NOT EXISTS idx_roadmap_ballots_account ON roadmap_ballots (account_id);

CREATE TABLE IF NOT EXISTS roadmap_moves (
    id          TEXT PRIMARY KEY,
    card_id     TEXT NOT NULL REFERENCES roadmap_cards(id) ON DELETE CASCADE,
    from_stage  TEXT NOT NULL,
    to_stage    TEXT NOT NULL,
    reason      TEXT NOT NULL,
    moved_by    TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    created_at  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_roadmap_moves_card ON roadmap_moves (card_id, created_at DESC);
