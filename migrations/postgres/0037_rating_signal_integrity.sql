CREATE TABLE IF NOT EXISTS rating_anomaly_events (
    id              UUID PRIMARY KEY,
    work_id         UUID REFERENCES works(id) ON DELETE CASCADE,
    cohort_id       UUID,
    kind            TEXT NOT NULL CHECK (kind IN ('burst', 'cohort_outlier', 'profile_outlier')),
    severity        INTEGER NOT NULL DEFAULT 1,
    detail          JSONB NOT NULL DEFAULT '{}',
    detected_at     TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    cleared_at      TIMESTAMPTZ,
    cleared_by      UUID REFERENCES accounts(id) ON DELETE SET NULL
);

CREATE INDEX IF NOT EXISTS rating_anomaly_work ON rating_anomaly_events(work_id);
CREATE INDEX IF NOT EXISTS rating_anomaly_detected ON rating_anomaly_events(detected_at);

ALTER TABLE works ADD COLUMN contested INTEGER NOT NULL DEFAULT 0;
ALTER TABLE works ADD COLUMN contested_at TIMESTAMPTZ;
ALTER TABLE works ADD COLUMN contested_reason TEXT;