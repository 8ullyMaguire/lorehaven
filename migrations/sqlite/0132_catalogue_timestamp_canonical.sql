-- 0132 — the catalogue's timestamps become one shape, and the database is what keeps it.
--
-- The PostgreSQL twin carries the reasoning (four shapes, 91,471 of them produced by an importer
-- that is not in this workspace, and the function call on the column costing the index). This file
-- is kept in step statement for statement so the parity test can compare the two.
--
-- SQLite cannot add a CHECK to an existing table without rebuilding it, so the same rule is a pair
-- of triggers per column. `GLOB` rather than `LIKE`: `LIKE` has no character classes, and `GLOB`
-- lets the trigger reject the row without calling a function on it first.

-- `strftime('%Y-%m-%dT%H:%M:%S', x)` reads all four shapes and normalises to UTC (SQLite's date
-- functions treat a zone-less value as UTC, which is the same rule M63-02 applies at read time), so
-- the backfill is one expression rather than a CASE over shapes.
UPDATE library_items
   SET source_updated_at = strftime('%Y-%m-%dT%H:%M:%SZ', source_updated_at)
 WHERE source_updated_at IS NOT NULL
   AND source_updated_at NOT GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z';

UPDATE library_items
   SET last_synced_at = strftime('%Y-%m-%dT%H:%M:%SZ', last_synced_at)
 WHERE last_synced_at IS NOT NULL
   AND last_synced_at NOT GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z';

UPDATE library_items
   SET created_at = strftime('%Y-%m-%dT%H:%M:%SZ', created_at)
 WHERE created_at IS NOT NULL
   AND created_at NOT GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z';

CREATE TRIGGER library_items_canonical_source_updated_at_insert
BEFORE INSERT ON library_items
WHEN NEW.source_updated_at IS NOT NULL
 AND NEW.source_updated_at NOT GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z'
BEGIN
    SELECT RAISE(ABORT, 'source_updated_at must be canonical UTC (…Z)');
END;

CREATE TRIGGER library_items_canonical_source_updated_at_update
BEFORE UPDATE OF source_updated_at ON library_items
WHEN NEW.source_updated_at IS NOT NULL
 AND NEW.source_updated_at NOT GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z'
BEGIN
    SELECT RAISE(ABORT, 'source_updated_at must be canonical UTC (…Z)');
END;

CREATE TRIGGER library_items_canonical_last_synced_at_insert
BEFORE INSERT ON library_items
WHEN NEW.last_synced_at IS NOT NULL
 AND NEW.last_synced_at NOT GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z'
BEGIN
    SELECT RAISE(ABORT, 'last_synced_at must be canonical UTC (…Z)');
END;

CREATE TRIGGER library_items_canonical_last_synced_at_update
BEFORE UPDATE OF last_synced_at ON library_items
WHEN NEW.last_synced_at IS NOT NULL
 AND NEW.last_synced_at NOT GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z'
BEGIN
    SELECT RAISE(ABORT, 'last_synced_at must be canonical UTC (…Z)');
END;

CREATE TRIGGER library_items_canonical_created_at_insert
BEFORE INSERT ON library_items
WHEN NEW.created_at IS NOT NULL
 AND NEW.created_at NOT GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z'
BEGIN
    SELECT RAISE(ABORT, 'created_at must be canonical UTC (…Z)');
END;

CREATE TRIGGER library_items_canonical_created_at_update
BEFORE UPDATE OF created_at ON library_items
WHEN NEW.created_at IS NOT NULL
 AND NEW.created_at NOT GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]Z'
BEGIN
    SELECT RAISE(ABORT, 'created_at must be canonical UTC (…Z)');
END;

CREATE INDEX IF NOT EXISTS idx_library_items_source_updated_at
    ON library_items (source_updated_at);
