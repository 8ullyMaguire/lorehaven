-- M32-07e (P1 repair): repair a foreign key that migration 0035 rewrote into a
-- table that no longer exists.
--
-- 0035 rebuilt `export_jobs` as `export_jobs_new` -> copy -> rename, and SQLite
-- rewrites the *referencing* foreign keys when it renames a table. So the
-- `device_deliveries.export_job_id` constraint created back in 0008 was silently
-- repointed at `export_jobs_old` and then had its target dropped. Every insert
-- into `device_deliveries` on a freshly migrated instance fails with
-- `no such table: main.export_jobs_old`, which is the exact error my own
-- milestone skill documents for this migration shape.
--
-- It shipped green because nothing in the codebase inserts to this table, so
-- the suite never reached the broken constraint. The schema was wrong and
-- unexercised, not working.
--
-- Nothing references `device_deliveries`, so the rebuild cannot cascade into a
-- second table. The shape is the CREATE-new -> copy -> DROP-old -> RENAME-new
-- form, so the only name rewritten is the one being fixed.
--
-- Dialect: SQLite.
CREATE TABLE device_deliveries_fixed (
    id            TEXT    PRIMARY KEY,
    -- Nullable rather than cascading: the diagnostic outlives the export.
    export_job_id TEXT    REFERENCES export_jobs (id) ON DELETE SET NULL,
    account_id    TEXT    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
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

INSERT INTO device_deliveries_fixed
    (id, export_job_id, account_id, target, address, state, last_error,
     created_at, delivered_at)
SELECT id, export_job_id, account_id, target, address, state, last_error,
       created_at, delivered_at
FROM device_deliveries;

DROP TABLE device_deliveries;
ALTER TABLE device_deliveries_fixed RENAME TO device_deliveries;

CREATE INDEX IF NOT EXISTS idx_device_deliveries_account
    ON device_deliveries (account_id);
CREATE INDEX IF NOT EXISTS idx_device_deliveries_export_job
    ON device_deliveries (export_job_id);
