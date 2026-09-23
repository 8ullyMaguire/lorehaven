-- M17 Phase 2 (Health Layer) — admin taste profile, work vectors, quiz, probes.
--
-- The admin taste profile is the instance's taste centroid (spec §0.4): one row
-- per dimension with the admin's ideal position (0.0-1.0) and influence weight.
CREATE TABLE IF NOT EXISTS admin_taste_profile (
    dimension_key TEXT PRIMARY KEY,
    label         TEXT NOT NULL,
    admin_target  DOUBLE PRECISION NOT NULL DEFAULT 0.5 CHECK (admin_target BETWEEN 0.0 AND 1.0),
    weight        DOUBLE PRECISION NOT NULL DEFAULT 1.0 CHECK (weight >= 0.0),
    updated_at    TEXT NOT NULL
);

-- Cached per-work taste vectors (spec §16.17). Derived from work tags: a work
-- scores on a dimension when its tags reference that dimension key.
CREATE TABLE IF NOT EXISTS work_taste_vectors (
    work_id     UUID PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    vector      JSONB NOT NULL DEFAULT '[]'::jsonb,  -- JSON array of f64, ordered by dimension key sort
    computed_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS work_taste_vectors_computed ON work_taste_vectors (computed_at);

-- Onboarding quiz answers (spec §0.4.2). One row per (account, work) the user
-- was shown; `picked` records whether they selected it.
CREATE TABLE IF NOT EXISTS quiz_answers (
    id          UUID PRIMARY KEY,
    account_id  UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    work_id     UUID NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    picked      INTEGER NOT NULL DEFAULT 0,
    answered_at TEXT NOT NULL,
    UNIQUE (account_id, work_id)
);
CREATE INDEX IF NOT EXISTS quiz_answers_account ON quiz_answers (account_id);

-- Taste probe engagement (spec §16.19). Records how a user engaged with a
-- probe work so the profile can expand (positive) or reinforce the boundary
-- (negative).
CREATE TABLE IF NOT EXISTS taste_probes (
    id           UUID PRIMARY KEY,
    account_id   UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    work_id      UUID NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    -- positive | negative | neutral
    engagement   TEXT NOT NULL CHECK (engagement IN ('positive', 'negative', 'neutral')),
    engaged_at   TEXT NOT NULL,
    UNIQUE (account_id, work_id)
);
CREATE INDEX IF NOT EXISTS taste_probes_account ON taste_probes (account_id, engaged_at);

-- Admin-curated quiz work pool (spec §0.4.2). Position controls display order.
CREATE TABLE IF NOT EXISTS quiz_works (
    work_id  UUID PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    position INTEGER NOT NULL DEFAULT 0,
    set_at   TEXT NOT NULL
);
