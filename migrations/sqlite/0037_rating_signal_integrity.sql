CREATE TABLE IF NOT EXISTS rating_anomaly_events (
    id              TEXT PRIMARY KEY,
    work_id         TEXT REFERENCES works(id) ON DELETE CASCADE,
    cohort_id       TEXT,
    kind            TEXT NOT NULL CHECK (kind IN ('burst', 'cohort_outlier', 'profile_outlier')),
    severity        INTEGER NOT NULL DEFAULT 1,
    detail          TEXT NOT NULL DEFAULT '{}',
    detected_at     TEXT NOT NULL,
    cleared_at      TEXT,
    cleared_by      TEXT
);

CREATE INDEX IF NOT EXISTS rating_anomaly_work ON rating_anomaly_events(work_id);
CREATE INDEX IF NOT EXISTS rating_anomaly_detected ON rating_anomaly_events(detected_at);

ALTER TABLE works ADD COLUMN contested INTEGER NOT NULL DEFAULT 0;
ALTER TABLE works ADD COLUMN contested_at TEXT;
ALTER TABLE works ADD COLUMN contested_reason TEXT;