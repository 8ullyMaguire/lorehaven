-- M43 — per-pseud browse sort preference (spec §43.4 stickiness).
--
-- Dialect: SQLite.

CREATE TABLE reader_sort_preferences (
    pseud_id        TEXT NOT NULL REFERENCES pseuds(id) ON DELETE CASCADE,
    surface         TEXT NOT NULL,   -- which surface this preference is for (e.g. 'discover', 'people', 'library')
    sort_value      TEXT NOT NULL,   -- the chosen sort value (§43.2 vocabulary)
    updated_at      TEXT NOT NULL,
    PRIMARY KEY (pseud_id, surface)
);
CREATE INDEX idx_sort_prefs_pseud ON reader_sort_preferences (pseud_id);
