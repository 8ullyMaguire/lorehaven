-- Migration 0102 — a work's coordinates: the four prose measures §49.3 says the
-- ranker scores against (M45-14).
--
-- Dialect: SQLite.
--
-- Why this is stored rather than computed on read
-- -------------------------------------------------
-- `crates/domain/src/coordinates.rs` computes the four measures as pure functions
-- of a work's chapter text, and §49.7's contract ("same text in, same
-- coordinates out, on both engines") is about that computation. Storing the
-- result is not a cache decision to be revisited; it is what makes the measures
-- usable at all:
--
--   * **Ranking reads coordinates per candidate work, not per ranked work.** A
--     browse page scores dozens of works to return one page of results. Computing
--     prose statistics over every candidate's full text on every request is not
--     a query, it is a reindex.
--   * **A dimension the ranker scores against must not move between requests.**
--     If coordinates were computed live, a work edited mid-session would change
--     its own coordinates, and a reader's ranking would shift under them without
--     any edit being visible. `version` and `chapter_revisions` make the text
--     versioned; this table records what that text measured *at*.
--   * **The two dialects must agree.** A live computation is a per-engine
--     implementation detail; a stored row is data both engines read identically,
--     which is the property the parity tests exist to protect.
--
-- So `text_version` records which revision the measures were computed from, and a
-- recompute is a backfill that can be run, audited, and rolled back — rather than
-- a thing that happens as a side effect of somebody reading a page.
--
-- One row per work
-- ----------------
-- Primary key on `work_id`, not a surrogate plus a unique constraint: a work has
-- exactly one set of coordinates and the work is the only thing that can be asked
-- for them by. `ON DELETE CASCADE` because coordinates are derived from text that
-- belongs to the work — when the work goes, so does everything computed from it,
-- and a surviving row would be a work_id pointing at nothing.
CREATE TABLE work_coordinates (
    work_id                   TEXT    PRIMARY KEY
                                         REFERENCES works (id) ON DELETE CASCADE,

    -- §49.3's four measures, each normalised to 0.0..=1.0 so it can be compared
    -- directly against a reader's `TasteDimension::admin_target` weight.
    --
    -- Nullable on both dialects, deliberately, because §49.3 requires that an
    -- unmeasurable coordinate be ABSENT and not zero: "A work too short to measure
    -- has no coordinates, and an absent coordinate is not a zero. A zero would mean
    -- 'uniformly flat prose' and would rank against short-but-sharp works."
    -- A zero would be a lie the ranker would believe.
    --
    -- `chapter_length_spread` is NULL for a single-chapter work for the same
    -- reason one level down: a one-chapter work has no length *distribution*, and
    -- 0.0 would assert "evenly sized chapters" about a work that has exactly one.
    -- The other three are non-null for any measured work, including
    -- `dialogue_ratio = 0.0` for a work with no dialogue at all — that is a fact
    -- about the prose, not an absence, and §49.3's rule is about absence.
    sentence_length_variance  REAL,
    dialogue_ratio            REAL,
    vocabulary_richness       REAL,
    chapter_length_spread     REAL,

    -- Why a work has no coordinates, when it has none.
    --
    -- 'no_text' | 'too_short' | NULL. NOT NULL is wrong here: a measured work has
    -- no reason, and forcing NULL to mean "measured" makes the common case the
    -- unremarkable one while the interesting case is spelled out. So the invariant
    -- is mutual exclusion rather than a NOT NULL: either the four measures are
    -- present and unmeasurable_reason is NULL, or the measures are NULL and the
    -- reason is set. Enforced by a CHECK below, because a row that claims both is
    -- a bug in a write path and a constraint catches it on the database that runs
    -- in production rather than only in the test suite.
    unmeasurable_reason       TEXT,

    -- The counts the measures were computed over, recorded so a stored coordinate
    -- can be checked against the text it came from without recomputing, and so a
    -- work that shrank below MIN_MEASURABLE_WORDS is detectable by comparison
    -- rather than only by re-running the computation.
    word_count                INTEGER,
    sentence_count            INTEGER,

    -- Which text this row measured.
    --
    -- `version` is `works.version` (INTEGER on SQLite, BIGINT on PostgreSQL — see
    -- the dialect file) and is copied rather than joined because the row must keep
    -- describing the text as it was *when measured*. Joining `works` would report
    -- today's version against yesterday's numbers, which is precisely the drift
    -- this table exists to prevent.
    text_version              INTEGER NOT NULL,

    -- When the coordinates were computed. TEXT on both dialects, matching
    -- `works.created_at` / `tasting_samples.created_at` (0003, 0099), which are
    -- TEXT on BOTH engines — not TIMESTAMPTZ on PostgreSQL. Read the sibling
    -- migration rather than assuming an asymmetry.
    computed_at               TEXT    NOT NULL,

    -- CHECK constraints
    -- ------------------
    -- SQLite accepts what PostgreSQL rejects, so these are the gate that keeps the
    -- two engines meaning the same thing. Without them a store bug could write a
    -- ratio of 1.7 and only PostgreSQL would ever notice.

    -- Exactly one of "measured" and "unmeasurable" holds.
    CHECK (
        (sentence_length_variance IS NULL AND dialogue_ratio IS NULL
             AND vocabulary_richness IS NULL AND chapter_length_spread IS NULL
             AND unmeasurable_reason IS NOT NULL)
     OR (sentence_length_variance IS NOT NULL AND dialogue_ratio IS NOT NULL
             AND vocabulary_richness IS NOT NULL
             AND unmeasurable_reason IS NULL)
    ),

    -- A measured work has word and sentence counts; an unmeasurable one has no
    -- measures to have counted. `too_short` works do have a word count, so this
    -- permits NULL for the no_text case only.
    CHECK (
        word_count IS NULL OR word_count > 0
    ),
    CHECK (
        sentence_count IS NULL OR sentence_count >= 0
    ),

    -- Every measure is inside the scale a reader's weight lives on. Clamped in the
    -- domain layer, but a constraint is the only check that runs on the database
    -- in production, and a coordinate of 1.7 cannot be compared against a weight
    -- at all — it silently ranks against everything.
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

    -- The only two reasons §49.3's computation can return. A CHECK rather than a
    -- table lookup because the enum is closed, two members long, and referenced
    -- from the domain layer as `Unmeasurable` — so a value outside this set means
    -- a write path and the domain layer disagree about what exists.
    CHECK (
        unmeasurable_reason IS NULL
     OR unmeasurable_reason IN ('no_text', 'too_short')
    )
);

-- Ranking looks coordinates up by work, so `work_id` is already the primary key
-- and this index exists for the other access shape: "every measured work with its
-- coordinates", which is what a backfill enumerates and what a diagnostic asks.
-- Partial, because an unmeasurable work has no coordinates to score and including
-- it would return rows that cannot be used.
CREATE INDEX work_coordinates_measured
    ON work_coordinates (work_id)
    WHERE unmeasurable_reason IS NULL;
