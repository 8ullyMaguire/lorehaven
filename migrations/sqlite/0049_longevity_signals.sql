-- Migration 0049 — longevity signals: half-life scoring and interaction warmth
-- (spec §41).
--
-- Dialect: SQLite.
-- Timestamps are RFC 3339 UTC text; identifiers are canonical UUID text.
--
-- Design notes:
--
--  * `half_life_bp` is INTEGER basis points, never REAL (ADR 0004). A work
--    published less than `half_life_min_age_days` ago has NULL half-life;
--    the job only scores works old enough to have a first window.
--
--  * `interaction_warmth` is a private reader-author relationship. The row
--    is per (reader account, author account) pair — never per work, never
--    per-reader visible. The `tier` is derived from `warmth_bp` and the
--    configured thresholds; the aggregate view counts authors' tiers.

-- Half-life score on the work row. NULL = not yet scored (too young).
ALTER TABLE works ADD COLUMN half_life_bp INTEGER;

-- Interaction warmth between a reader and an author (spec §41.2).
CREATE TABLE interaction_warmth (
    account_id      TEXT    NOT NULL,  -- the reader
    author_account  TEXT    NOT NULL,  -- the author
    warmth_bp       INTEGER NOT NULL DEFAULT 0,
    tier            TEXT    NOT NULL DEFAULT 'lurk',
    updated_at      TEXT    NOT NULL,
    PRIMARY KEY (account_id, author_account)
);

CREATE INDEX interaction_warmth_author
    ON interaction_warmth (author_account, tier);
