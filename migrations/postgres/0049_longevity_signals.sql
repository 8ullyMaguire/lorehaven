-- Migration 0049 — longevity signals: half-life scoring and interaction warmth
-- (spec §41).
--
-- Dialect: PostgreSQL.

-- Half-life score on the work row. NULL = not yet scored (too young).
ALTER TABLE works ADD COLUMN half_life_bp INTEGER;

-- Interaction warmth between a reader and an author (spec §41.2).
CREATE TABLE interaction_warmth (
    account_id      UUID    NOT NULL,
    author_account  UUID    NOT NULL,
    warmth_bp       INTEGER NOT NULL DEFAULT 0,
    tier            TEXT    NOT NULL DEFAULT 'lurk',
    updated_at      TEXT    NOT NULL,
    PRIMARY KEY (account_id, author_account)
);

CREATE INDEX interaction_warmth_author
    ON interaction_warmth (author_account, tier);
