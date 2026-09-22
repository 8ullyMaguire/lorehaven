-- M15 settlement tables (spec §20.10.9) — SQLite dialect.
--
-- Pool B distributions and per-period summaries.

CREATE TABLE IF NOT EXISTS pool_b_distributions (
    id                          TEXT    NOT NULL PRIMARY KEY,
    period_start                TEXT    NOT NULL,
    period_end                  TEXT    NOT NULL,
    author_account_id           TEXT    NOT NULL REFERENCES accounts (id) ON DELETE RESTRICT,
    amount_minor                INTEGER NOT NULL,
    currency                    TEXT    NOT NULL DEFAULT 'EUR',
    quality_score_bp            INTEGER NOT NULL DEFAULT 0,
    attributed_reading_time_seconds INTEGER NOT NULL DEFAULT 0,
    ai_multiplier_bp            INTEGER NOT NULL DEFAULT 10000,
    idempotency_key             TEXT,
    created_at                  TEXT    NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_pool_b_dist_author ON pool_b_distributions(author_account_id, period_start);
CREATE UNIQUE INDEX IF NOT EXISTS idx_pool_b_dist_idem
    ON pool_b_distributions(idempotency_key) WHERE idempotency_key IS NOT NULL;

CREATE TABLE IF NOT EXISTS monetization_period_summaries (
    id                          TEXT    NOT NULL PRIMARY KEY,
    period_start                TEXT    NOT NULL,
    period_end                  TEXT    NOT NULL,
    pool_a_total_minor          INTEGER NOT NULL DEFAULT 0,
    pool_b_total_minor          INTEGER NOT NULL DEFAULT 0,
    active_earner_median_minor  INTEGER NOT NULL DEFAULT 0,
    cap_value_minor             INTEGER NOT NULL DEFAULT 0,
    authors_in_pool_a           INTEGER NOT NULL DEFAULT 0,
    authors_in_pool_b           INTEGER NOT NULL DEFAULT 0,
    authors_capped              INTEGER NOT NULL DEFAULT 0,
    processor_fee_min_minor     INTEGER NOT NULL DEFAULT 0,
    processor_fee_max_minor     INTEGER NOT NULL DEFAULT 0,
    created_at                  TEXT    NOT NULL,
    UNIQUE (period_start, period_end)
);
CREATE INDEX IF NOT EXISTS idx_period_summaries_period ON monetization_period_summaries(period_start, period_end);
