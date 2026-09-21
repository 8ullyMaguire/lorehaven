-- M42 — Export CTAs (spec §42): curator marks recording whether a work
-- already carries its author's own CTA. Quorum of agreeing true-marks
-- exempts the work from the instance CTA.

CREATE TABLE cta_marks (
    -- The work being marked.
    work_id     TEXT NOT NULL,
    -- The curator (account id) who marked it.
    curator     TEXT NOT NULL,
    -- Whether the author's own CTA is present in the work.
    has_own_cta INTEGER NOT NULL CHECK (has_own_cta IN (0, 1)),
    -- When the mark was made, RFC 3339.
    marked_at   TEXT NOT NULL,
    PRIMARY KEY (work_id, curator)
);

CREATE INDEX idx_cta_marks_work ON cta_marks (work_id);
