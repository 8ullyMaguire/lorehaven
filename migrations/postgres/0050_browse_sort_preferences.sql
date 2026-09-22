-- M43 — per-pseud browse sort preference (spec §43.4 stickiness).
--
-- Dialect: PostgreSQL.

CREATE TABLE reader_sort_preferences (
    pseud_id        UUID NOT NULL REFERENCES pseuds(id) ON DELETE CASCADE,
    surface         TEXT NOT NULL,
    sort_value      TEXT NOT NULL,
    updated_at      TIMESTAMPTZ NOT NULL,
    PRIMARY KEY (pseud_id, surface)
);
CREATE INDEX idx_sort_prefs_pseud ON reader_sort_preferences (pseud_id);
