-- M15 settlement tables (spec §20.10.9)
-- Pool B distributions and per-period summaries.
--
-- Convention note (per 63f22db): TEXT timestamps, INTEGER counters, JSON as
-- TEXT — identical column shapes to the SQLite dialect. No TIMESTAMPTZ/JSONB.

-- §20.10.9 — monthly Pool B distributions to eligible authors.
CREATE TABLE IF NOT EXISTS pool_b_distributions (
    id                          UUID PRIMARY KEY,
    period_start                TEXT NOT NULL,
    period_end                  TEXT NOT NULL,
    author_account_id           UUID NOT NULL REFERENCES accounts (id) ON DELETE RESTRICT,
    amount_minor                BIGINT NOT NULL,
    currency                    TEXT NOT NULL DEFAULT 'EUR',
    quality_score_bp            BIGINT NOT NULL DEFAULT 0,
    attributed_reading_time_seconds BIGINT NOT NULL DEFAULT 0,
    ai_multiplier_bp            BIGINT NOT NULL DEFAULT 10000,
    idempotency_key             TEXT,
    created_at                  TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_pool_b_dist_author ON pool_b_distributions(author_account_id, period_start);
CREATE UNIQUE INDEX IF NOT EXISTS idx_pool_b_dist_idem
    ON pool_b_distributions(idempotency_key) WHERE idempotency_key IS NOT NULL;

-- §20.10.9 — per-period settlement summary (one row per period).
CREATE TABLE IF NOT EXISTS monetization_period_summaries (
    id                          UUID PRIMARY KEY,
    period_start                TEXT NOT NULL,
    period_end                  TEXT NOT NULL,
    pool_a_total_minor          BIGINT NOT NULL DEFAULT 0,
    pool_b_total_minor          BIGINT NOT NULL DEFAULT 0,
    active_earner_median_minor  BIGINT NOT NULL DEFAULT 0,
    cap_value_minor             BIGINT NOT NULL DEFAULT 0,
    authors_in_pool_a           BIGINT NOT NULL DEFAULT 0,
    authors_in_pool_b           BIGINT NOT NULL DEFAULT 0,
    authors_capped              BIGINT NOT NULL DEFAULT 0,
    processor_fee_min_minor     BIGINT NOT NULL DEFAULT 0,
    processor_fee_max_minor     BIGINT NOT NULL DEFAULT 0,
    created_at                  TEXT NOT NULL,
    UNIQUE (period_start, period_end)
);
CREATE INDEX IF NOT EXISTS idx_period_summaries_period ON monetization_period_summaries(period_start, period_end);
