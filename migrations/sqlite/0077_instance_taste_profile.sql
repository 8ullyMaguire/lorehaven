-- Migration 0077 — the instance Taste Profile, stored rather than faked (spec §0.4,
-- amended by docs/spec-amendments/taste-gravitational-system.md).
--
-- SQLite twin of migrations/postgres/0077_instance_taste_profile.sql. Same
-- columns and the same single-row constraint; JSONB becomes TEXT because that
-- is what SQLite has, and the id is INTEGER rather than a sentinel UUID.

CREATE TABLE instance_taste_profile (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    dimensions      TEXT    NOT NULL DEFAULT '[]',
    exemplars       TEXT    NOT NULL DEFAULT '[]',
    anti_examples   TEXT    NOT NULL DEFAULT '[]',
    gravity_strength       INTEGER NOT NULL DEFAULT 0,
    signal_weight_mode     TEXT    NOT NULL DEFAULT 'balanced',
    admin_weight           INTEGER NOT NULL DEFAULT 0,
    diversity_injection_percent INTEGER NOT NULL DEFAULT 0,
    updated_by      TEXT,
    updated_at      TEXT    NOT NULL,
    version         INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX instance_taste_profile_updated_by ON instance_taste_profile (updated_by);
