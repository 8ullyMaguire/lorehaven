-- Migration 0100 — the selector's uncertainty on the sample it drew
-- (spec §49.5, M45-19).
--
-- Dialect: PostgreSQL.
--
-- Mirrors migrations/sqlite/0100_tasting_uncertainty.sql statement for
-- statement, in the same order, with the same columns. The parity test
-- (the_two_dialects_declare_the_same_columns_and_indexes) compares declared
-- column SETS and index column lists, and it is blind to `ADD COLUMN` types --
-- see the note on `flagged_by` in 0099 and the test that exists to cover that
-- gap. So the two files must be written to match by hand, and the store carries
-- the casts.

-- The selector's uncertainty at the instant it drew this sample.
--
-- Nullable on both dialects, deliberately and for the same reason on both: every
-- pre-0100 row was drawn by a code path that did not record one, and there is no
-- honest default. Inventing a value (0.5 — the cold-start reading) would
-- manufacture a number for rows whose real uncertainty nobody knows, and an
-- evaluation across them would then be confidently wrong. `NULL` says exactly
-- what is true: not recorded.
--
-- PostgreSQL *could* add this as NOT NULL with a default, and SQLite could not.
-- The column is nullable on both so the two schemas mean the same thing, which
-- is the property the code depends on; the store reads `Option<f64>` and treats
-- `NULL` as "not recorded" rather than leaning on a constraint that only one
-- engine would enforce.
ALTER TABLE tasting_samples
    ADD COLUMN uncertainty_at_draw DOUBLE PRECISION;

-- Carries the "one open sample per (work, reader)" rule that the UNIQUE index
-- below needs to be expressible. A subquery in the index predicate is not a
-- permitted form here either — PostgreSQL requires the predicate to be an
-- expression over the row's own columns — so the fact is held as a column and
-- written by the response path in the same statement that inserts the response.
ALTER TABLE tasting_samples
    ADD COLUMN answered_at TEXT;

-- Partial, for the same reason as the SQLite file: a reader may re-sample a work
-- they have already answered, and must be able to.
--
-- Type note, checked rather than assumed: `tasting_samples.created_at` is TEXT
-- in 0099 on **both** dialects (`created_at TEXT NOT NULL`), not TIMESTAMPTZ on
-- PostgreSQL. `answered_at` therefore follows the column it sits beside and is
-- TEXT on both, so the store binds the same RFC 3339 string on either engine and
-- no cast is needed. An earlier draft of this file asserted a TIMESTAMPTZ
-- asymmetry that does not exist; reading the sibling migration is what caught it,
-- and it is recorded here because the next author will wonder why a timestamp
-- column is text.
CREATE UNIQUE INDEX IF NOT EXISTS idx_tasting_samples_one_open_per_work
    ON tasting_samples (account_id, work_id)
    WHERE answered_at IS NULL;
