-- 0122: `chapters` becomes `units`, and `chapter_revisions` becomes `revisions`
-- (M4-02).
--
-- Spec §5 is normative: a **unit** is a part of a work — a chapter, a track,
-- an episode — and a **revision** is an append-only version of a unit's text.
-- `chapters` and `chapter_revisions` are lorehaven's words, and the three
-- projects' words for the same thing must not leak into the code (R-2).
--
-- THE RENAME DOES NOT REWRITE THE DATA
--
-- Every row keeps its identity, its order and its text. A rename is a change
-- of vocabulary and not of content, and a migration that rewrote content would
-- be a migration with a data-loss story.
--
-- THE TABLES FIRST, THEN THE COLUMNS
--
-- The order matters on SQLite: `ALTER TABLE ... RENAME TO` rewrites the
-- foreign keys that point at the renamed table, and a column rename afterwards
-- rewrites the column inside the constraint. Renaming the columns first would
-- leave the constraints pointing at a table that is about to change its name.
--
-- THE INDEXES ARE REBUILT, NOT RENAMED
--
-- SQLite has no `ALTER INDEX ... RENAME TO`, so the index is dropped and
-- recreated under its new name. The *definition* is unchanged: an index that
-- changed shape would be a migration that quietly made a query faster, and a
-- query that got faster for no reason is a query nobody can explain later.
--
-- The public URL keeps its shape — `/api/v1/works/{id}/chapters` — because a
-- URL is the most stable public surface an instance has, and the *model* is
-- what this migration renames. The response field `chapters` becomes `units`,
-- which is the breaking change, and it is recorded in the changelog.

-- ── the tables ────────────────────────────────────────────────────────────

ALTER TABLE chapter_revisions RENAME TO revisions;
ALTER TABLE chapters RENAME TO units;
ALTER TABLE import_chapters RENAME TO import_units;

-- ── the columns ───────────────────────────────────────────────────────────

-- The revision's unit. A revision belongs to a unit and to nothing else.
ALTER TABLE revisions RENAME COLUMN chapter_id TO unit_id;

-- The publication event's unit: the event is about a unit when it is about
-- one, and about the work as a whole when it is not.
ALTER TABLE publication_events RENAME COLUMN chapter_id TO unit_id;

-- The reader's bookmarked unit.
ALTER TABLE bookmarks RENAME COLUMN chapter_id TO unit_id;

-- The reader's progress through a unit.
ALTER TABLE reading_progress RENAME COLUMN chapter_id TO unit_id;

-- The bounty aimed at a unit.
ALTER TABLE targeted_bounties RENAME COLUMN chapter_id TO unit_id;

-- The topic's link to the unit it discusses.
ALTER TABLE topic_work_links RENAME COLUMN chapter_id TO unit_id;

-- The translation's unit: **the source's** unit, which is the source's own
-- identifier and not the archive's.
ALTER TABLE translation_units RENAME COLUMN chapter_id TO unit_id;

-- The media reference's unit.
ALTER TABLE work_media_references RENAME COLUMN chapter_id TO unit_id;

-- The passage's unit — the pair the passage is about.
ALTER TABLE work_passages RENAME COLUMN chapter_id TO unit_id;

-- The warning's unit.
ALTER TABLE work_warnings RENAME COLUMN chapter_id TO unit_id;

-- The comment's anchor: the unit a comment is anchored to.
ALTER TABLE comments RENAME COLUMN anchor_chapter_id TO anchor_unit_id;

-- The imported unit's key: the source's own identifier for the part.
ALTER TABLE import_units RENAME COLUMN source_chapter_key TO source_unit_key;

-- The imported unit that was materialised into a work.
ALTER TABLE import_units RENAME COLUMN chapter_id TO unit_id;

-- The reader's body copy: which unit it came from.
ALTER TABLE reader_body_copies RENAME COLUMN chapter_key TO unit_key;

-- The reader's progress: the unit they were last reading.
ALTER TABLE reader_work_progress RENAME COLUMN last_chapter TO last_unit;

-- The schedule's window: the units it covers.
ALTER TABLE topic_schedules RENAME COLUMN chapter_start TO unit_start;
ALTER TABLE topic_schedules RENAME COLUMN chapter_end TO unit_end;

-- The spoiler scope: a unit rather than a "chapter".
ALTER TABLE forum_topics RENAME COLUMN spoiler_scope_chapter TO spoiler_scope_unit;

-- The coordinates' length spread: the spread of the work's unit lengths.
ALTER TABLE work_coordinates RENAME COLUMN chapter_length_spread TO unit_length_spread;

-- ── the indexes ───────────────────────────────────────────────────────────

-- The indexes are dropped and recreated here rather than renamed, so that the
-- two dialects DECLARE THE SAME INDEX NAMES IN THE SAME MIGRATION. The schema
-- parity test reads the `CREATE INDEX` statements of a migration and compares
-- the two dialects, and a PostgreSQL `ALTER INDEX ... RENAME TO` is invisible
-- to it -- which would make the parity test report a difference that is not
-- one.
DROP INDEX chapters_work_order;
CREATE INDEX units_work_order ON units (work_id, order_key);

DROP INDEX chapters_revision;
CREATE INDEX units_revision ON units (current_revision_id);

DROP INDEX chapter_revisions_chapter;
CREATE INDEX revisions_unit ON revisions (unit_id, revision_number DESC);

DROP INDEX import_chapters_job;
CREATE INDEX import_units_job ON import_units (import_job_id, ordinal);

DROP INDEX import_chapters_item;
CREATE INDEX import_units_item ON import_units (library_item_id, ordinal);

DROP INDEX IF EXISTS idx_work_media_references_chapter;
CREATE INDEX IF NOT EXISTS idx_work_media_references_unit ON work_media_references(unit_id);
