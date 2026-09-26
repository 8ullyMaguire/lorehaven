-- Migration 0077 — the instance's taste *knobs*, stored (spec §0.4, amended by
-- docs/spec-amendments/taste-gravitational-system.md).
--
-- SQLite twin of migrations/postgres/0077_instance_taste_settings.sql. The
-- dimensions are deliberately absent here: `admin_taste_profile` (migration
-- 0054) has owned them since M17, and a second copy would be a second truth.

CREATE TABLE instance_taste_settings (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    gravity_strength       INTEGER NOT NULL DEFAULT 0,
    signal_weight_mode     TEXT    NOT NULL DEFAULT 'taste_weighted',
    admin_weight           INTEGER NOT NULL DEFAULT 1,
    diversity_injection_percent INTEGER NOT NULL DEFAULT 10,
    updated_by      TEXT,
    updated_at      TEXT    NOT NULL,
    version         INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX instance_taste_settings_updated_by ON instance_taste_settings (updated_by);
