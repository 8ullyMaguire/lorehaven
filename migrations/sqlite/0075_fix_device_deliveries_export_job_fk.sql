-- M32-07e (P1 repair): repair a foreign key that migration 0035 rewrote into a
-- table that no longer exists.
--
-- 0035 rebuilt `export_jobs` as a rename-and-recreate, and SQLite rewrites the
-- *referencing* foreign keys when it renames a table. So the
-- `device_deliveries.export_job_id` constraint created back in 0008 was silently
-- repointed at `export_jobs_old` and then had its target dropped. Every insert
-- into `device_deliveries` on a freshly migrated instance fails with
-- `no such table: main.export_jobs_old`, which is the exact error the
-- `building-a-broken-recovery` reference documents for this migration shape.
--
-- It shipped green because nothing in the codebase inserts into this table, so
-- the suite never reached the broken constraint. The schema was wrong and
-- unexercised, not working.
--
-- The shape is 0035's: rename the broken table out of the way, then CREATE under
-- the real name. That keeps the *declared* schema identical to PostgreSQL's 0075,
-- which matters because `the_two_dialects_declare_the_same_columns_and_indexes`
-- in `crates/db/src/migrate.rs` compares the table and index NAMES the two files
-- declare. A `CREATE TABLE device_deliveries_fixed` + `RENAME` rebuild declares a
-- name the PostgreSQL twin never mentions, so the parity test correctly fails on
-- it -- which is exactly what it did on the first attempt at this repair.
--
-- `PRAGMA foreign_keys=OFF` guards the window where the table is renamed away;
-- the pool sets foreign_keys=ON itself, so this only needs to cover the rebuild.
--
-- Nothing references `device_deliveries`, so dropping it cannot cascade into a
-- second table.
--
-- Dialect: SQLite.
PRAGMA foreign_keys=OFF;

ALTER TABLE device_deliveries RENAME TO device_deliveries_broken;

CREATE TABLE device_deliveries (
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

INSERT INTO device_deliveries
    (id, export_job_id, account_id, target, address, state, last_error,
     created_at, delivered_at)
SELECT id, export_job_id, account_id, target, address, state, last_error,
       created_at, delivered_at
FROM device_deliveries_broken;

DROP TABLE device_deliveries_broken;

PRAGMA foreign_keys=ON;

-- Index names and definitions match PostgreSQL's 0075, which restores 0008's
-- verbatim. Plain CREATE, not IF NOT EXISTS: the old indexes went away with the
-- renamed table, and the migration-parity test in crates/db/src/migrate.rs
-- parses the index name as the last whitespace token before ` ON`, so an
-- `IF NOT EXISTS` prefix makes it read the name as `NOT`. The whole statement also
-- has to sit on ONE line: the parser scans line by line looking for ` ON ` in
-- the same line as the CREATE, so a wrapped index body is invisible to it.
CREATE INDEX device_deliveries_account ON device_deliveries (account_id, created_at DESC);
-- The bounce list an operator reads: what failed, most recent first.
CREATE INDEX device_deliveries_state ON device_deliveries (state, created_at DESC);
