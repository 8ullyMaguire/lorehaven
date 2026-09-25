-- M32-07e (P1 repair): restore the `device_deliveries` table that migration 0035
-- dropped and never recreated.
--
-- 0035 rebuilt `export_jobs` and, because PostgreSQL cannot ALTER a constraint
-- in place, it dropped the dependent tables first:
--
--     DROP TABLE IF EXISTS device_deliveries;
--
-- ...then recreated `download_grants` and `bulk_export_items`, but never
-- `device_deliveries`. So on PostgreSQL the table has been absent since 0035 and
-- any code path that delivered an export to a device had nothing to insert into.
--
-- This is a different defect from the SQLite one, fixed in the same batch: the
-- SQLite rename rewrote the foreign key to point at a dropped table, while PG
-- removed the table outright. Both are silent -- the SQLite failure is deferred
-- to the first insert, and the PG one is a missing relation -- and both shipped
-- green because no code inserts into this table, so the suite never reached it.
--
-- There is no data to copy: the table does not exist on this dialect. The shape
-- is 0008's definition verbatim, including its indexes, so the two dialects
-- agree on the schema from here on.
--
-- Dialect: PostgreSQL.
CREATE TABLE device_deliveries (
    id            UUID    PRIMARY KEY,
    -- Nullable rather than cascading: the diagnostic outlives the export.
    export_job_id UUID    REFERENCES export_jobs (id) ON DELETE SET NULL,
    account_id    UUID    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    -- Where it was sent. Named rather than inferred, because "sent to a Kindle"
    -- and "sent by email" fail differently and are supported separately.
    target        TEXT    NOT NULL CHECK (target IN ('kindle', 'email')),
    address       TEXT    NOT NULL,
    state         TEXT    NOT NULL DEFAULT 'queued'
                          CHECK (state IN ('queued', 'sent', 'failed')),
    last_error    TEXT,
    created_at    TEXT    NOT NULL,
    delivered_at  TEXT
);

CREATE INDEX device_deliveries_account ON device_deliveries (account_id, created_at DESC);
-- The bounce list an operator reads: what failed, most recent first.
CREATE INDEX device_deliveries_state ON device_deliveries (state, created_at DESC);
