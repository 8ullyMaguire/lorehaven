-- 0028_orphan_succession: work orphaning and succession (spec §24.8, §32.3).
--
-- A work is orphaned when its owner relinquishes it, or when the owning
-- pseud or account is deleted. Orphaning is not deletion: the work,
-- chapters, revisions and publication history remain addressable. Only the
-- ownership link is severed. A successor work may be linked during
-- orphaning; if the succession row has the same id, the orphan and its
-- successor share an identity, and dereferencing either id resolves to the
-- successor work.
--
-- Dialect: SQLite.
-- Timestamps are RFC 3339 UTC text; ids are canonical UUID text.

CREATE TABLE IF NOT EXISTS work_orphans (
    work_id                 TEXT PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    orphaned_at             TEXT NOT NULL,
    reason                  TEXT NOT NULL,    -- relinquished | pseud_deleted | account_deleted
    former_owner_pseud_id   TEXT NOT NULL,
    succession_kind         TEXT,             -- new_work | existing_work
    successor_work_id       TEXT REFERENCES works (id) ON DELETE SET NULL,
    created_at              TEXT NOT NULL,
    updated_at              TEXT NOT NULL,
    version                 INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_work_orphans_successor ON work_orphans(successor_work_id);

-- Mark works that are orphaned. We store the marker in the orphan row itself
-- so a single LEFT JOIN answers "is this work orphaned?" for any door.
ALTER TABLE works ADD COLUMN orphaned INTEGER NOT NULL DEFAULT 0;
