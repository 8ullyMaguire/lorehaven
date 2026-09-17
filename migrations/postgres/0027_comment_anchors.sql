-- 0027_comment_anchors: anchored comments (spec §12, §30.4; M24).
-- Anchors a comment to a specific position: paragraph offset for text,
-- timestamp for media. NULL anchor_kind means a whole-work/media comment.

ALTER TABLE comments ADD COLUMN IF NOT EXISTS anchor_kind TEXT;
ALTER TABLE comments ADD COLUMN IF NOT EXISTS anchor_value TEXT;
ALTER TABLE comments ADD COLUMN IF NOT EXISTS anchor_chapter_id UUID REFERENCES chapters(id) ON DELETE SET NULL;

CREATE INDEX IF NOT EXISTS comments_anchor ON comments (subject_type, subject_id, anchor_kind, anchor_value) WHERE anchor_kind IS NOT NULL;
