-- 0028_orphan_succession: work orphaning and succession (spec §24.8, §32.3).
--
-- Dialect: PostgreSQL.
-- Timestamps are TIMESTAMPTZ; ids are UUID; booleans are BOOLEAN.

CREATE TABLE IF NOT EXISTS work_orphans (
    work_id                 UUID PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    orphaned_at             TIMESTAMPTZ NOT NULL,
    reason                  TEXT NOT NULL,    -- relinquished | pseud_deleted | account_deleted
    former_owner_pseud_id   UUID NOT NULL,
    succession_kind         TEXT,             -- new_work | existing_work
    successor_work_id       UUID REFERENCES works (id) ON DELETE SET NULL,
    created_at              TIMESTAMPTZ NOT NULL,
    updated_at              TIMESTAMPTZ NOT NULL,
    version                 BIGINT NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_work_orphans_successor ON work_orphans(successor_work_id);

ALTER TABLE works ADD COLUMN orphaned BOOLEAN NOT NULL DEFAULT FALSE;
