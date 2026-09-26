-- Migration 0077 — the instance's taste *knobs*, stored (spec §0.4, amended by
-- docs/spec-amendments/taste-gravitational-system.md).
--
-- `PUT /operator/taste-profile` answered `{"status": "updated"}` and persisted
-- nothing, so an operator's change to the instance's taste model was accepted,
-- reported as done, and lost on the next request. The config file cannot be the
-- home for it either: a running instance does not rewrite its own configuration,
-- and the API has no business editing a file it may not have permission to write.
--
-- What is NOT here, and why: the dimensions. `admin_taste_profile` (migration
-- 0054) has held `dimension_key, label, admin_target, weight` since M17, and
-- `taste_health::save_admin_taste_profile` writes it. This table holds only the
-- scalar knobs that config also carries and no table did — gravity strength,
-- signal weight mode, admin weight, diversity injection — so there is one home
-- for the dimensions and one for the knobs, rather than two competing copies of
-- the same concept.
--
-- One row: an instance has one taste model. `id = 1` says so in the schema
-- rather than leaving it to convention.

CREATE TABLE instance_taste_settings (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    gravity_strength       INTEGER NOT NULL DEFAULT 0,
    signal_weight_mode     TEXT    NOT NULL DEFAULT 'taste_weighted',
    admin_weight           INTEGER NOT NULL DEFAULT 1,
    diversity_injection_percent INTEGER NOT NULL DEFAULT 10,
    -- Who made the last change, and when. §19's audit rule applies to the
    -- instance's taste model as much as to anything else an operator touches.
    updated_by      UUID,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    version         INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX instance_taste_settings_updated_by ON instance_taste_settings (updated_by);
