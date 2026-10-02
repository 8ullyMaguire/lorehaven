-- Migration 0106 — reason-tagged kudos, line-level highlights, and the canon-blind
-- flag (spec §50.1, §50.2; M45-24, M45-31).
--
-- Dialect: SQLite.
--
-- Three tables, in dependency order. The reasons are the point of all three, so
-- each carries one and each refuses an empty one.
--
-- 1. `work_kudos` gains a reason rather than becoming a new table. A kudos that
--    is a bare count and a kudos that carries a reason are the same gesture, and
--    §50.1 says a bare kudos stays VALID -- so this is a nullable column with a
--    CHECK, not a NOT NULL one and not a second table. Splitting them would make
--    "kudos per work" a UNION and make the count slower for no gain.
--
-- 2. `work_highlights` is separate because it is not a kudos. §50.1: a highlight
--    is a reason about a SPAN, it counts ONCE toward the work's gravity however
--    many exist, and it dies with the work. The PK on (work_id, account_id) plus
--    the offset is what makes "counts once" a property of the schema rather than a
--    rule a counter has to remember.
--
-- 3. `canon_agnostic_works` records §50.2's class. It is a table and not a column
--    on `work_coordinates` because §50.5 refuses automatic detection: the flag is
--    a claim about whether a work is readable without its parent canon, computed
--    by the same deterministic measures, and it deserves its own row with its own
--    provenance (which text version decided it) rather than one more nullable
--    column in a table about prose statistics.
--
-- Every reason column is an enumerated CHECK rather than free text, for §50.3's
-- reason: a reason that cannot be aggregated is not a signal. Free text rides
-- alongside as an unaggregatable note and is never a reason on its own.

ALTER TABLE work_kudos ADD COLUMN reason TEXT
    CHECK (reason IS NULL OR reason IN (
        'prose', 'characters', 'pacing', 'trope_execution',
        'worldbuilding', 'not_for_me'
    ));

ALTER TABLE work_kudos ADD COLUMN note TEXT;

-- §50.1: (work_id, account_id, offset) is the key, so a reader has at most one
-- highlight per sentence. Fifty highlights of different sentences are fifty
-- signals ABOUT SPANS and one signal ABOUT THE WORK -- the count that reaches
-- gravity is one, and it is one here because the work count is a COUNT(DISTINCT
-- account_id) rather than a row count.
CREATE TABLE IF NOT EXISTS work_highlights (
    id            TEXT    PRIMARY KEY,
    work_id       TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    account_id    TEXT    NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    -- The span is identified by its character offset into the work's concatenated
    -- text, the same addressing §49.5's tasting samples use. An offset rather than
    -- quoted text because a quote goes stale on the first edit, and a highlight
    -- whose text no longer matches is a highlight about nothing.
    start_offset  INTEGER NOT NULL CHECK (start_offset >= 0),
    end_offset    INTEGER NOT NULL CHECK (end_offset > start_offset),
    reason        TEXT    NOT NULL
                          CHECK (reason IN (
                              'prose', 'characters', 'pacing',
                              'trope_execution', 'worldbuilding', 'not_for_me'
                          )),
    note          TEXT,
    created_at    TEXT    NOT NULL
);

CREATE INDEX idx_work_highlights_work ON work_highlights (work_id);
CREATE INDEX idx_work_highlights_account ON work_highlights (account_id);
CREATE UNIQUE INDEX idx_work_highlights_span ON work_highlights (work_id, account_id, start_offset, end_offset);

-- §50.2: a work's fandom is unknown to a reader, which is not the same as
-- unknown to the instance. The flag is per work and carries the version of the
-- text that decided it, so a re-measure can supersede it rather than disagree
-- with it.
CREATE TABLE IF NOT EXISTS canon_agnostic_works (
    work_id           TEXT    PRIMARY KEY
                              REFERENCES works (id) ON DELETE CASCADE,
    -- Which text version produced this answer. Same reasoning as
    -- `work_coordinates.version`: a flag whose input has moved is stale, and
    -- staleness that is invisible is worse than no flag at all.
    text_version      INTEGER NOT NULL,
    -- §50.2's class is computed from the deterministic measures, so it is
    -- reproducible. Stored rather than computed per request for the same reason
    -- coordinates are: §49.3's "computed once at ingest".
    declared_at       TEXT    NOT NULL
);
