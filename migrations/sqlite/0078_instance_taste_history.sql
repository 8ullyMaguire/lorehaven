-- Migration 0078 — history for the instance's taste knobs (gaps review F1/F2).
--
-- SQLite twin of migrations/postgres/0078_instance_taste_history.sql. Same
-- columns and the same "the row is the state *before* the write" contract, so a
-- rollback is a copy rather than a direction flag.

CREATE TABLE instance_taste_settings_history (
    history_id            TEXT    PRIMARY KEY,
    replaced_version      INTEGER,
    gravity_strength      INTEGER NOT NULL,
    signal_weight_mode    TEXT    NOT NULL,
    admin_weight          INTEGER NOT NULL,
    diversity_injection_percent INTEGER NOT NULL,
    changed_by            TEXT,
    changed_at            TEXT    NOT NULL,
    rolled_back_from      TEXT
);

CREATE INDEX instance_taste_settings_history_changed_at ON instance_taste_settings_history (changed_at);
CREATE INDEX instance_taste_settings_history_changed_by ON instance_taste_settings_history (changed_by);
