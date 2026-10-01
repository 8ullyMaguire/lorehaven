-- Migration 0102 — a work's coordinates: the four prose measures §49.3 says the
-- ranker scores against (M45-14).
--
-- Dialect: PostgreSQL.
--
-- Mirrors migrations/sqlite/0102_work_coordinates.sql statement for statement, in
-- the same order, with the same columns. The parity test
-- (the_two_dialects_declare_the_same_columns_and_indexes) compares declared column
-- SETS and index column lists, and it is blind to column *types* -- see the note on
-- `flagged_by` in 0099. So the two files are written to match by hand, and every
-- type difference below is deliberate and is the one place they are allowed to
-- differ.
--
-- The three differences from the SQLite file, each required by the engine:
--
--   1. `work_id` is UUID here and TEXT on SQLite. It follows `works.id`
--      (0003: `id UUID PRIMARY KEY`), and rule 1 of this repo's dialect notes
--      applies: an id's type is decided by its FOREIGN KEY, never by the
--      neighbouring table's spelling. 0099's `tasting_samples.work_id` is the
--      precedent.
--   2. The four measures are DOUBLE PRECISION, not REAL. REAL is single-precision
--      float (about 7 significant decimal digits); §49.8 requires coordinates to
--      "reproduce byte for byte", and a value rounded through a 4-byte float does
--      not round-trip the same text the same way on both engines. The domain layer
--      already rounds to six decimals; REAL would quietly discard digits beyond
--      that and make the two engines disagree in their low bits.
--   3. The counts are BIGINT, not INTEGER. PostgreSQL maps INTEGER to INT4 (a
--      32-bit signed range) while the store reads i64 -- rule 4 of the dialect
--      notes, and the reason `show_public_ratings` and `version` in 0003 are BIGINT
--      on this side only. A `word_count` over INT4 would be fine for any real work,
--      and would still be a decode error in the projection.
CREATE TABLE work_coordinates (
    work_id                   UUID    PRIMARY KEY
                                         REFERENCES works (id) ON DELETE CASCADE,

    -- §49.3's four measures, each normalised to 0.0..=1.0 so it can be compared
    -- directly against a reader's `TasteDimension::admin_target` weight.
    --
    -- Nullable on both dialects, deliberately and for the same reason on both:
    -- §49.3 requires that an unmeasurable coordinate be ABSENT and not zero --
    -- "A zero would mean 'uniformly flat prose' and would rank against
    -- short-but-sharp works." PostgreSQL *could* express the measured case as NOT
    -- NULL columns in a separate table from the unmeasured case; the column is
    -- nullable on both so the two schemas mean the same thing, which is the
    -- property the store and the parity tests depend on.
    sentence_length_variance  DOUBLE PRECISION,
    dialogue_ratio            DOUBLE PRECISION,
    vocabulary_richness       DOUBLE PRECISION,
    chapter_length_spread     DOUBLE PRECISION,

    -- 'no_text' | 'too_short' | NULL, with the mutual-exclusion CHECK below rather
    -- than NOT NULL: a measured work has no reason, and a constraint that forces
    -- NULL to mean "measured" inverts the common case.
    unmeasurable_reason       TEXT,

    word_count                BIGINT,
    sentence_count            BIGINT,

    -- `works.version` is BIGINT here and INTEGER on SQLite, so this column follows
    -- it. Copied rather than joined so the row keeps describing the text as it was
    -- when measured.
    text_version              BIGINT  NOT NULL,

    -- TEXT, matching `works.created_at` (0003) and `tasting_samples.created_at`
    -- (0099), which are TEXT on BOTH engines. Not TIMESTAMPTZ -- checked, not
    -- assumed; an earlier draft of 0100 asserted a TIMESTAMPTZ asymmetry that does
    -- not exist, and reading the sibling migration is what caught it.
    computed_at               TEXT    NOT NULL,

    -- CHECK constraints, identical in meaning to the SQLite file. PostgreSQL
    -- enforces all of them; SQLite enforces its own CHECKs too, so neither engine
    -- is the only line of defence here -- the parity is what matters, and both
    -- files carry the same seven.

    -- Exactly one of "measured" and "unmeasurable" holds. Note this permits a
    -- MEASURED work with chapter_length_spread NULL: a single-chapter work is
    -- measured and has no distribution to report, and §49.3's absent-is-not-a-zero
    -- rule applies at that level too.
    CHECK (
        (sentence_length_variance IS NULL AND dialogue_ratio IS NULL
             AND vocabulary_richness IS NULL AND chapter_length_spread IS NULL
             AND unmeasurable_reason IS NOT NULL)
     OR (sentence_length_variance IS NOT NULL AND dialogue_ratio IS NOT NULL
             AND vocabulary_richness IS NOT NULL
             AND unmeasurable_reason IS NULL)
    ),

    CHECK (
        word_count IS NULL OR word_count > 0
    ),
    CHECK (
        sentence_count IS NULL OR sentence_count >= 0
    ),

    -- Every measure inside the scale a reader's weight lives on. Clamped in the
    -- domain layer; a constraint is what runs on the production database.
    CHECK (
        (sentence_length_variance IS NULL OR (sentence_length_variance >= 0.0 AND sentence_length_variance <= 1.0))
    ),
    CHECK (
        (dialogue_ratio IS NULL OR (dialogue_ratio >= 0.0 AND dialogue_ratio <= 1.0))
    ),
    CHECK (
        (vocabulary_richness IS NULL OR (vocabulary_richness >= 0.0 AND vocabulary_richness <= 1.0))
    ),
    CHECK (
        (chapter_length_spread IS NULL OR (chapter_length_spread >= 0.0 AND chapter_length_spread <= 1.0))
    ),

    CHECK (
        unmeasurable_reason IS NULL
     OR unmeasurable_reason IN ('no_text', 'too_short')
    )
);

-- Same shape as the SQLite file. PostgreSQL supports partial indexes natively, so
-- no dialect workaround is needed -- which is worth stating because the 0100 index
-- needed a column carried specifically to make its predicate expressible.
CREATE INDEX work_coordinates_measured
    ON work_coordinates (work_id)
    WHERE unmeasurable_reason IS NULL;
