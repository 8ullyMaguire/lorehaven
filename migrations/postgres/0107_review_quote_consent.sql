-- Migration 0107 — quote consent for rec notes (spec §49.4, M45-35).
--
-- Dialect: PostgreSQL.
--
-- Same column, same default, same constraint, same index predicate as the SQLite
-- arm, so `the_two_dialects_declare_the_same_columns_and_indexes` passes.
--
-- Two dialect notes:
--
-- * `BOOLEAN` where SQLite uses `INTEGER` with a CHECK. The parity test compares
--   declared column SETS and index column lists and is blind to types, so this
--   difference is safe -- and it is the right difference, because SQLite has no
--   boolean type and `INTEGER NOT NULL DEFAULT 0 CHECK (allow_quote IN (0,1))` is
--   how that dialect spells it.
-- * `DROP INDEX IF EXISTS` is not used, because this index is new in this
--   migration and so cannot already exist. A `DROP` would be a no-op that hides
--   the real question: if the name IS taken, the migration fails loudly instead,
--   which is the outcome wanted.
--
-- The consent belongs to the READER who wrote the note, not to the author of the
-- work -- the two are different people and only one of them can consent to their
-- own words. It defaults to FALSE: a note written before this migration, or by a
-- client that does not know the field, is NOT quotable. Defaulting to TRUE would
-- publish the notes of every reader who has ever written one, retroactively and
-- silently, which is exactly the consent failure §49.4 exists to prevent.

ALTER TABLE review ADD COLUMN allow_quote BOOLEAN NOT NULL DEFAULT FALSE
    CHECK (allow_quote IN (TRUE, FALSE));

-- The predicate is the whole index: an eligible note must be public, published,
-- undeleted AND consented, and only those can be surfaced. The rating lives in
-- `rating` as a separate row by the same pseud, so the join happens over
-- (pseud_id, work_id) and the consent predicate stays in the index rather than
-- behind a join.
CREATE INDEX IF NOT EXISTS review_quotable_public
    ON review (work_id, published_at)
    WHERE allow_quote AND is_public
          AND published_at IS NOT NULL AND deleted_at IS NULL;
