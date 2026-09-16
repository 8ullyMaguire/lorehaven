-- M23 §30.2 — media files for a work (downloadable artifacts).
-- Split out of the round-4 edit of 0024: applied migrations are immutable
-- (sqlx pins checksums per version), so new tables get a new migration.

CREATE TABLE IF NOT EXISTS media_files (
    id              TEXT PRIMARY KEY,
    work_id         TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    edition_kind    TEXT NOT NULL,              -- prose | poetry | anthology | translation | narration
    url             TEXT,
    size_bytes      INTEGER,
    mime_type       TEXT,
    checksum        TEXT,
    created_at      TEXT NOT NULL,
    updated_at      TEXT NOT NULL,
    version         INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_media_files_work ON media_files(work_id);
