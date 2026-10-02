-- §50.2: the canon-agnostic CLASS, made storable. M45-31.
--
-- 0106 created `canon_agnostic_works` and could only record "this work IS
-- canon-agnostic". That makes "classified as canon-dependent" and "never
-- classified" the same row: absent means nothing, so a reader cannot tell a
-- measured standalone work from an unmeasured one, and §49.3 says an absent
-- value is not a zero.
--
-- So the class is stored explicitly and absence is a third state rather than a
-- default. The shape mirrors `work_coordinates` (0102), which solved the same
-- problem for the same measures:
--
--   * `canon_dependent` says WHICH WAY, 0 | 1, never NULL. §50.2 wants a class
--     and not a score, so there is no floating-point verdict here.
--   * `unmeasurable_reason` is set when and only when no class was reached.
--   * the three measures behind the verdict are stored too, so a stored flag can
--     be audited and a threshold change can be re-derived rather than trusted.
--
-- The last point is why the measures are columns and not just a consequence of
-- the class: §50.3 requires the class be reproducible from the text, which means
-- someone has to be able to recompute it and compare.

ALTER TABLE canon_agnostic_works
    ADD COLUMN canon_dependent INTEGER NOT NULL DEFAULT 0
        CHECK (canon_dependent IN (0, 1));

ALTER TABLE canon_agnostic_works ADD COLUMN unmeasurable_reason TEXT;
ALTER TABLE canon_agnostic_works ADD COLUMN unexplained_names INTEGER;
ALTER TABLE canon_agnostic_works ADD COLUMN word_count        INTEGER;
ALTER TABLE canon_agnostic_works ADD COLUMN density           REAL;

-- ── Multi-column invariants, as triggers ─────────────────────────────────────
--
-- SQLite cannot add a table-level CHECK to an existing table, and the ALTER form
-- only accepts a column's own single-column CHECK. Verified on 3.53: the four
-- contradictions below are all accepted by a plain table with no trigger, so the
-- invariant has to be written out. 0104 uses the same idiom for the same reason;
-- the shape is
--
--     WHEN (A is true) <> (B is true)
--
-- which in SQLite's WHERE means "exactly one of these two holds".
--
-- 1. A reason and a verdict are mutually exclusive. A row claiming both is
--    asserting a classification and its own absence.

CREATE TRIGGER canon_class_reason_ck_insert
    BEFORE INSERT ON canon_agnostic_works
    FOR EACH ROW
    WHEN (NEW.unmeasurable_reason IS NOT NULL)
      <> (NEW.unexplained_names IS NULL)
    BEGIN
        SELECT RAISE(ABORT,
            'canon_agnostic_works: unmeasurable_reason and the measures are set '
            || 'together or not at all');
    END;

CREATE TRIGGER canon_class_reason_ck_update
    BEFORE UPDATE ON canon_agnostic_works
    FOR EACH ROW
    WHEN (NEW.unmeasurable_reason IS NOT NULL)
      <> (NEW.unexplained_names IS NULL)
    BEGIN
        SELECT RAISE(ABORT,
            'canon_agnostic_works: unmeasurable_reason and the measures are set '
            || 'together or not at all');
    END;

-- 2. A class is never stored without the measures that produced it. An
--    unverifiable flag is worse than an absent one, because it looks answered.
--    §49.3's whole point is that a number can be recomputed from the text; a
--    verdict with no density cannot be.

CREATE TRIGGER canon_class_needs_measures_insert
    BEFORE INSERT ON canon_agnostic_works
    FOR EACH ROW
    WHEN (NEW.unmeasurable_reason IS NULL)
      AND (NEW.unexplained_names IS NULL OR NEW.word_count IS NULL
           OR NEW.density IS NULL)
    BEGIN
        SELECT RAISE(ABORT,
            'canon_agnostic_works: a classified row carries the measures that '
            || 'produced it');
    END;

CREATE TRIGGER canon_class_needs_measures_update
    BEFORE UPDATE ON canon_agnostic_works
    FOR EACH ROW
    WHEN (NEW.unmeasurable_reason IS NULL)
      AND (NEW.unexplained_names IS NULL OR NEW.word_count IS NULL
           OR NEW.density IS NULL)
    BEGIN
        SELECT RAISE(ABORT,
            'canon_agnostic_works: a classified row carries the measures that '
            || 'produced it');
    END;

-- 3. `density` is a share, so it is a share: not negative, not above one. The
--    domain layer clamps to 0.0..=1.0 and 0102 says in a comment that SQLite's
--    dynamic typing makes the domain clamp the only protection the default test
--    engine has — so the range is checked here too.

CREATE TRIGGER canon_class_density_range_insert
    BEFORE INSERT ON canon_agnostic_works
    FOR EACH ROW
    WHEN NEW.density IS NOT NULL AND (NEW.density < 0.0 OR NEW.density > 1.0)
    BEGIN
        SELECT RAISE(ABORT, 'canon_agnostic_works.density must be within 0.0..=1.0');
    END;

CREATE TRIGGER canon_class_density_range_update
    BEFORE UPDATE ON canon_agnostic_works
    FOR EACH ROW
    WHEN NEW.density IS NOT NULL AND (NEW.density < 0.0 OR NEW.density > 1.0)
    BEGIN
        SELECT RAISE(ABORT, 'canon_agnostic_works.density must be within 0.0..=1.0');
    END;

-- 4. The counts must be counts. A negative name count or word count is not a
--    measurement, and a name count larger than the word count is arithmetically
--    impossible — which is exactly the corruption a bad fixture produces, so the
--    database refuses it rather than letting it reach a reader.

CREATE TRIGGER canon_class_counts_sane_insert
    BEFORE INSERT ON canon_agnostic_works
    FOR EACH ROW
    WHEN NEW.unexplained_names IS NOT NULL
     AND (NEW.unexplained_names < 0 OR NEW.word_count <= 0
          OR NEW.unexplained_names > NEW.word_count)
    BEGIN
        SELECT RAISE(ABORT,
            'canon_agnostic_works: unexplained_names must be within 0..word_count');
    END;

CREATE TRIGGER canon_class_counts_sane_update
    BEFORE UPDATE ON canon_agnostic_works
    FOR EACH ROW
    WHEN NEW.unexplained_names IS NOT NULL
     AND (NEW.unexplained_names < 0 OR NEW.word_count <= 0
          OR NEW.unexplained_names > NEW.word_count)
    BEGIN
        SELECT RAISE(ABORT,
            'canon_agnostic_works: unexplained_names must be within 0..word_count');
    END;