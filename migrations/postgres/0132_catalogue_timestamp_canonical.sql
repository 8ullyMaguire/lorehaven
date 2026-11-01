-- 0132 — the catalogue's timestamps become one shape, and the database is what keeps it.
--
-- M63-02 taught every window to read four different timestamp shapes correctly, at the cost of a
-- function call on the column: `2026-10-13` (91,471 rows, the out-of-tree XenForo importer),
-- `2026-10-27T01:04:45` with no zone at all (27,461), `…+00:00` (older writer, 0 of these in
-- source_updated_at but 66,875 in created_at), `…Z` (60) and
-- `…T15:18:27.394000` with no zone (3). With a function on the column PostgreSQL cannot use an
-- index, and `?updated=7d` measured 1.7s cold over 141,317 rows.
--
-- The shape is now a rule the database enforces instead of a habit each writer keeps. That matters
-- here more than usual: the bare dates are produced by an importer that is *not in this workspace*,
-- so a rule kept only in Rust would be bypassed by the one writer that broke the shape in the first
-- place. From this migration on, that importer fails loudly
-- (`violates check constraint "library_items_canonical_timestamps"`) instead of storing a value that
-- every window has to guess at.
--
-- The backfill applies the rule M63-02 already used at read time -- a value carrying `Z` or an
-- offset is an instant, anything else is UTC by definition -- so no row's instant moves, and
-- `?updated=@2026-10-27` returns the same rows before and after this migration. Only the rendering
-- changes.
--
-- The canonical form is second-precision with a `Z`, which is what the in-tree writer already
-- emits: `time`'s Rfc3339 formatter writes `Z` for UTC (verified: an `OffsetDateTime` in UTC
-- formats to `2026-11-01T12:00:00Z`). Choosing `+00:00` instead would have made the database
-- refuse its own writer's output, which is how this was caught — the first draft of this
-- migration did exactly that and 30-odd fixtures that mirror production failed.

-- The shape test, written out three times rather than factored into a function: a function would
-- have to be created and dropped here, and `IMMUTABLE` functions in a check are a footgun the next
-- migration would inherit.
UPDATE library_items
   SET source_updated_at = CASE
         WHEN source_updated_at ~ '^\d{4}-\d{2}-\d{2}$'
           THEN source_updated_at || 'T00:00:00Z'
         ELSE to_char(((CASE WHEN source_updated_at ~ '(Z|[+-][0-9]{2}:[0-9]{2})$'
                             THEN source_updated_at::timestamptz
                             ELSE source_updated_at::timestamp AT TIME ZONE 'UTC' END) AT TIME ZONE 'UTC'),
                      'YYYY-MM-DD"T"HH24:MI:SS"Z"')
       END
 WHERE source_updated_at IS NOT NULL
   AND source_updated_at !~ '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$';

UPDATE library_items
   SET last_synced_at = CASE
         WHEN last_synced_at ~ '^\d{4}-\d{2}-\d{2}$'
           THEN last_synced_at || 'T00:00:00Z'
         ELSE to_char(((CASE WHEN last_synced_at ~ '(Z|[+-][0-9]{2}:[0-9]{2})$'
                             THEN last_synced_at::timestamptz
                             ELSE last_synced_at::timestamp AT TIME ZONE 'UTC' END) AT TIME ZONE 'UTC'),
                      'YYYY-MM-DD"T"HH24:MI:SS"Z"')
       END
 WHERE last_synced_at IS NOT NULL
   AND last_synced_at !~ '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$';

UPDATE library_items
   SET created_at = CASE
         WHEN created_at ~ '^\d{4}-\d{2}-\d{2}$'
           THEN created_at || 'T00:00:00Z'
         ELSE to_char(((CASE WHEN created_at ~ '(Z|[+-][0-9]{2}:[0-9]{2})$'
                             THEN created_at::timestamptz
                             ELSE created_at::timestamp AT TIME ZONE 'UTC' END) AT TIME ZONE 'UTC'),
                      'YYYY-MM-DD"T"HH24:MI:SS"Z"')
       END
 WHERE created_at IS NOT NULL
   AND created_at !~ '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$';

-- NOT VALID then VALIDATE: the constraint is added without scanning the table under an exclusive
-- lock, then checked. Both statements are in this migration, so a failure to validate fails the
-- migration rather than leaving an unvalidated check behind.
ALTER TABLE library_items
    ADD CONSTRAINT library_items_canonical_timestamps CHECK (
         (source_updated_at IS NULL
          OR source_updated_at ~ '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$')
     AND (last_synced_at IS NULL
          OR last_synced_at ~ '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$')
     AND (created_at IS NULL
          OR created_at ~ '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$')
    ) NOT VALID;

ALTER TABLE library_items VALIDATE CONSTRAINT library_items_canonical_timestamps;

-- With one shape, the window is a range on a btree rather than a function call per row. This is
-- the index M63-02's `CASE` made unusable.
CREATE INDEX IF NOT EXISTS idx_library_items_source_updated_at
    ON library_items (source_updated_at);
