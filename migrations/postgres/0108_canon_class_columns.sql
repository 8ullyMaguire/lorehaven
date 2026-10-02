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
--   * `canon_dependent` says WHICH WAY, never NULL. §50.2 wants a class and not a
--     score, so there is no floating-point verdict here.
--   * `unmeasurable_reason` is set when and only when no class was reached.
--   * the three measures behind the verdict are stored too, so a stored flag can
--     be audited and a threshold change can be re-derived rather than trusted.
--
-- The measures are columns and not merely a consequence of the class because
-- §50.3 requires the class be reproducible from the text, which means someone
-- has to be able to recompute it and compare.
--
-- ── Where this differs from the SQLite dialect, and why ───────────────────────
--
-- Four invariants. SQLite needs ten triggers because it cannot add a table-level
-- CHECK to an existing table, and PostgreSQL states each one as an ordinary CHECK in
-- the same `ALTER`. The rules are identical, which is what makes the two engines
-- agree about which rows are legal.
--
--   1. A reason and a verdict are mutually exclusive: a row claiming both is
--      asserting a classification and its own absence.
--   2. A class is never stored without the measures that produced it. An
--      unverifiable flag is worse than an absent one, because it looks answered.
--   3. `density` is a share, so not negative and not above one. The domain layer
--      clamps and 0102 notes that SQLite's dynamic typing makes the domain clamp
--      the only protection the default test engine has. Here the range is a
--      constraint on both engines rather than a habit in one.
--   4. `unexplained_names` lies within `0..=word_count`. The upper bound is not
--      paranoia: it is exactly the corruption a bad fixture produces, and a
--      density above 1.0 would silently misclassify the work.

ALTER TABLE canon_agnostic_works
    ADD COLUMN canon_dependent BOOLEAN NOT NULL DEFAULT FALSE;

ALTER TABLE canon_agnostic_works ADD COLUMN unmeasurable_reason TEXT;
ALTER TABLE canon_agnostic_works ADD COLUMN unexplained_names INTEGER;
ALTER TABLE canon_agnostic_works ADD COLUMN word_count        INTEGER;
ALTER TABLE canon_agnostic_works ADD COLUMN density           DOUBLE PRECISION;

-- ── Backfill, and why this value ─────────────────────────────────────────────
--
-- `ADD CONSTRAINT` validates against existing rows, so a deployment that already
-- flagged works under 0106 cannot apply this migration without a backfill --
-- verified on 15.5: adding these two CHECKs to a table holding two pre-0108 rows
-- fails outright.
--
-- A legacy row says only "this work was once called canon-agnostic". It does not
-- carry the measures that produced that verdict, and §50.3 requires the class be
-- recomputable from the text -- so the only honest state for it is
-- UNCLASSIFIED, not a class. Backfilling `canon_dependent = false` would read as
-- "measured, and canon-agnostic", which is a measurement nobody made.
--
-- So: a reason, no measures. `too_short` is the reason work_coordinates already
-- uses for "there was not enough text", which is close enough to be accurate
-- here -- these rows predate the measurement, so there is nothing to audit --
-- and the alternative is inventing a reason string for a state the schema does
-- not otherwise name. What matters to a reader is that the row says
-- "unclassified" rather than asserting a class.
--
-- The work is re-measured on the next ingest, which is what makes this safe: the
-- flag is a cache of a deterministic computation, not a source of truth.

UPDATE canon_agnostic_works
   SET unmeasurable_reason = 'unclassified: 0108 backfill, never measured'
 WHERE unmeasurable_reason IS NULL
   AND unexplained_names IS NULL;

ALTER TABLE canon_agnostic_works
    ADD CONSTRAINT canon_class_reason_ck CHECK (
        -- Neither `=` nor a bare `IS DISTINCT FROM`. In three-valued logic
        -- `false = NULL` is NULL, and a CHECK passes on NULL, so `=` would reject
        -- every LEGAL row: a classified work has a NULL reason and a non-NULL
        -- unexplained_names, which is exactly the pair this constraint exists to
        -- allow. Verified on 15.5 -- the `=` form rejected the legal row
        -- (false, NULL, 3, 1000, 0.003).
        --
        -- And not `IS DISTINCT FROM` either: PostgreSQL parses the `NOT` as
        -- applying to the whole `(a IS DISTINCT FROM b)`, which inverts the
        -- intent -- `pg_get_constraintdef` came back as `NOT (a IS DISTINCT FROM
        -- b)`, so it demanded the two NULL-ness flags DIFFER, which rejects the
        -- legal row and accepts `reason+measures`. Verified: both wrong.
        --
        -- The pairing, exactly: a reason is set when and only when the measures
        -- are ABSENT. So the left side asks "is there a reason" and the right side
        -- asks "are the measures absent", and the two must agree.
        --
        -- Getting the second `IS NULL` wrong inverts the whole constraint. The
        -- first version read `(unmeasurable_reason IS NOT NULL) = (unexplained_names
        -- IS NOT NULL)`, which is exactly backwards: it rejects every legal row
        -- (a classified work has no reason but does have measures) and accepts
        -- `reason+measures`. Verified on 15.5 -- both directions wrong before,
        -- both right after.
        (unmeasurable_reason IS NOT NULL) = (unexplained_names IS NULL)
    );

ALTER TABLE canon_agnostic_works
    ADD CONSTRAINT canon_class_needs_measures_ck CHECK (
        unmeasurable_reason IS NOT NULL
        OR (unexplained_names IS NOT NULL AND word_count IS NOT NULL
            AND density IS NOT NULL)
    );

ALTER TABLE canon_agnostic_works
    ADD CONSTRAINT canon_class_density_range_ck CHECK (
        density IS NULL OR (density >= 0.0 AND density <= 1.0)
    );

ALTER TABLE canon_agnostic_works
    ADD CONSTRAINT canon_class_counts_sane_ck CHECK (
        unexplained_names IS NULL
        OR (unexplained_names >= 0 AND word_count > 0
            AND unexplained_names <= word_count)
    );