-- Migration 0077 — the instance Taste Profile, stored rather than faked (spec §0.4,
-- amended by docs/spec-amendments/taste-gravitational-system.md).
--
-- `PUT /discovery/admin/taste-profile` answered `{"status": "updated"}` and
-- persisted nothing, so an operator's change to the instance's taste model was
-- accepted, reported as done, and lost on the next request. The config file
-- cannot be the home for it either: a running instance does not rewrite its own
-- configuration, and an edit to the file is not something the API can do.
--
-- One row per instance. The dimensions are a JSONB document because the shape is
-- defined by the amendment rather than by this schema: `dimensions` is
-- `[{key, label, admin_target, weight}]` and the instance chooses which axes
-- matter, so a fixed column list would be a hardcoded taxonomy the amendment
-- explicitly rules out.

CREATE TABLE instance_taste_profile (
    id              INTEGER PRIMARY KEY CHECK (id = 1),
    -- [{key, label, admin_target, weight}] -- see the amendment's §0.4. The
    -- domain validates the entries; the column stores what it is given.
    dimensions      JSONB   NOT NULL DEFAULT '[]'::jsonb,
    -- Works the operator likes, as negative/positive calibration anchors.
    exemplars       JSONB   NOT NULL DEFAULT '[]'::jsonb,
    -- Works the operator explicitly dislikes.
    anti_examples   JSONB   NOT NULL DEFAULT '[]'::jsonb,
    gravity_strength       INTEGER NOT NULL DEFAULT 0,
    signal_weight_mode     TEXT    NOT NULL DEFAULT 'balanced',
    admin_weight           INTEGER NOT NULL DEFAULT 0,
    diversity_injection_percent INTEGER NOT NULL DEFAULT 0,
    -- Who made the last change, and when. §19's audit rule applies to the
    -- instance's taste model as much as to anything else an operator touches.
    updated_by      UUID,
    updated_at      TIMESTAMPTZ NOT NULL DEFAULT now(),
    version         INTEGER NOT NULL DEFAULT 1
);

CREATE INDEX instance_taste_profile_updated_by ON instance_taste_profile (updated_by);
