-- Migration 0107 — quote consent for rec notes (spec §49.4, M45-35).
--
-- Dialect: SQLite.
--
-- §49.4's second clause: "Only notes the author has allowed to be quoted are
-- eligible. A rec note is a reader's words about someone else's work; surfacing
-- it unquoted is a consent question, not a display one."
--
-- The consent belongs to the READER who wrote the note, not to the author of the
-- work -- the two are different people and only one of them can consent to their
-- own words. The column therefore lives on `review` and defaults to 0: a note
-- written before this migration, or by a client that does not know the field, is
-- NOT quotable. Defaulting to 1 would publish the notes of every reader who has
-- ever written one, which is exactly the consent failure the clause exists to
-- prevent, and it would do so retroactively and silently.

ALTER TABLE review ADD COLUMN allow_quote INTEGER NOT NULL DEFAULT 0
    CHECK (allow_quote IN (0, 1));

-- §49.4's first and third clauses, and the index that makes "the top-rated
-- quotable note" a bounded lookup rather than a scan over every note ever
-- written. The predicate is the whole index: an eligible note must be public,
-- published, undeleted AND consented, and only those can be surfaced.
--
-- The rating lives in `rating` as a separate row by the same pseud, so the index
-- is on review alone and the join to `rating` happens over the (pseud_id,
-- work_id) pair. That keeps the consent predicate in the index rather than
-- behind a join, so a note that is not eligible is never even counted.
CREATE INDEX IF NOT EXISTS review_quotable_public
    ON review (work_id, published_at)
    WHERE allow_quote = 1 AND is_public = 1
          AND published_at IS NOT NULL AND deleted_at IS NULL;

-- §49.4's fourth clause is enforced in the store, not the schema: rec blurbs are
-- not a ranking input, so nothing here feeds the ranker. There is deliberately no
-- column on any ranking table -- the separation is structural.
