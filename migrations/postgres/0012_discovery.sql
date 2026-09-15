CREATE TABLE taste_profiles (
    account TEXT PRIMARY KEY,
    signals TEXT NOT NULL,
    computed_at TEXT NOT NULL
);

CREATE TABLE operator_affinities (
    work_id TEXT PRIMARY KEY,
    affinity_bp INTEGER NOT NULL,
    operator TEXT NOT NULL,
    rationale TEXT NOT NULL,
    set_at TEXT NOT NULL
);

CREATE TABLE recipes (
    id TEXT PRIMARY KEY,
    owner TEXT NOT NULL,
    name TEXT NOT NULL,
    document TEXT NOT NULL,
    is_public BIGINT NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL
);

CREATE TABLE dashboard_layouts (
    account TEXT PRIMARY KEY,
    slots TEXT NOT NULL,
    updated_at TEXT NOT NULL
);
