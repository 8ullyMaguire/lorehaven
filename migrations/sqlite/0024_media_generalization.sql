-- M22 skeleton — generalized media entity model (spec §32, R1).
-- Creators (local pseuds and external records), distributors and
-- distribution edges, typed media collections and memberships, media
-- editions, media rights, quality signals, and works.format (§30.1).
--
-- Identifier family: content (ADR 0004, ADR 0019) — TEXT ids here, UUID in
-- the PostgreSQL twin; TEXT timestamps; INTEGER counters; no JSON columns.
--
-- Retention: creators, distributors and their edges are attribution — they
-- are anonymised, not deleted, when a pseud leaves; collections owned by an
-- account are private reader/curation state and cascade with the account;
-- editions and rights are durable publication history and never cascade
-- beyond their work; quality signals are recomputable and cascade with the
-- work.

-- §32.3.1 — who made it: a local pseud or an external creator record.
-- An external creator is NEVER merged into a local account by matching
-- names or handles (§11.11); verification is a quorum act (§19).
CREATE TABLE IF NOT EXISTS creators (
    id                TEXT PRIMARY KEY,
    kind              TEXT NOT NULL,             -- local_pseud | external
    pseud_id          TEXT REFERENCES pseuds (id) ON DELETE CASCADE,
    display_name      TEXT NOT NULL,
    source_key        TEXT,                      -- adapter key: ao3 | wattpad | ...
    source_creator_id TEXT,                      -- the platform's own identifier
    canonical_url     TEXT,
    verified_at       TEXT,                      -- quorum verification (§19)
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    version           INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_creators_pseud ON creators(pseud_id);
CREATE UNIQUE INDEX IF NOT EXISTS idx_creators_external
    ON creators(source_key, source_creator_id) WHERE source_key IS NOT NULL;

-- §32.3.2 — attribution edges: a work is made by one or more creators.
CREATE TABLE IF NOT EXISTS media_creators (
    id         TEXT PRIMARY KEY,
    work_id    TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    creator_id TEXT NOT NULL REFERENCES creators (id) ON DELETE CASCADE,
    role       TEXT NOT NULL,                    -- author | narrator | editor | translator | artist | other
    position   INTEGER NOT NULL DEFAULT 0,
    created_at TEXT NOT NULL,
    UNIQUE (work_id, creator_id, role)
);
CREATE INDEX IF NOT EXISTS idx_media_creators_creator ON media_creators(creator_id);

-- §32.3.3 — who made it available, and how.
CREATE TABLE IF NOT EXISTS distributors (
    id            TEXT PRIMARY KEY,
    name          TEXT NOT NULL,
    kind          TEXT NOT NULL,                 -- platform | publisher | archive | zine | self
    source_key    TEXT,
    canonical_url TEXT,
    created_at    TEXT NOT NULL,
    updated_at    TEXT NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_distributors_source
    ON distributors(source_key) WHERE source_key IS NOT NULL;

CREATE TABLE IF NOT EXISTS distributorships (
    id             TEXT PRIMARY KEY,
    work_id        TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    distributor_id TEXT NOT NULL REFERENCES distributors (id) ON DELETE CASCADE,
    role           TEXT NOT NULL,                -- published | hosted | mirrored | preserved | narrated | translated | reprinted
    detail_url     TEXT,
    created_at     TEXT NOT NULL,
    UNIQUE (work_id, distributor_id, role)
);
CREATE INDEX IF NOT EXISTS idx_distributorships_distributor ON distributorships(distributor_id);

-- §32.3.4 — series, anthologies, reading lists, archive collections,
-- challenge anthologies, preserved batches: one typed-membership model.
-- (M13 event collections keep their own tables; this model does not merge
-- them, it generalises the *media* side — see ADR 0019.)
CREATE TABLE IF NOT EXISTS media_collections (
    id                TEXT PRIMARY KEY,
    collection_kind   TEXT NOT NULL,             -- series | anthology | reading_list | archive_collection | challenge_anthology | preserved_batch
    owning_account_id TEXT REFERENCES accounts (id) ON DELETE CASCADE,
    title             TEXT NOT NULL,
    description       TEXT,
    visibility        TEXT NOT NULL DEFAULT 'public',  -- public | unlisted | restricted
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    version           INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_media_collections_owner ON media_collections(owning_account_id);

CREATE TABLE IF NOT EXISTS media_collection_items (
    id            TEXT PRIMARY KEY,
    collection_id TEXT NOT NULL REFERENCES media_collections (id) ON DELETE CASCADE,
    work_id       TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    position      INTEGER NOT NULL DEFAULT 0,
    note          TEXT,
    added_by_account_id TEXT REFERENCES accounts (id) ON DELETE SET NULL,
    added_at      TEXT NOT NULL,
    UNIQUE (collection_id, work_id)
);
CREATE INDEX IF NOT EXISTS idx_media_collection_items_work ON media_collection_items(work_id);

-- §32.3.5 — publication history. Revisions (M5) remain the editing
-- history; editions are the *publication* history and never overwrite.
CREATE TABLE IF NOT EXISTS media_editions (
    id                TEXT PRIMARY KEY,
    work_id           TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    edition_kind      TEXT NOT NULL,             -- draft | revised | anthology | translation | narration | printing
    label             TEXT,
    parent_edition_id TEXT REFERENCES media_editions (id) ON DELETE SET NULL,
    published_at      TEXT,
    created_at        TEXT NOT NULL,
    updated_at        TEXT NOT NULL,
    version           INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX IF NOT EXISTS idx_media_editions_work ON media_editions(work_id);

-- §32.3.6 — rights on the creative object, stated once per work.
-- ai_training stays on works (M21, §24.14): it is the author's AI
-- statement; this table is the licensing and lending statement.
CREATE TABLE IF NOT EXISTS media_rights (
    work_id          TEXT PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    license          TEXT NOT NULL DEFAULT 'unknown',
    rights_statement TEXT,
    lending_class    TEXT NOT NULL DEFAULT 'none',  -- none | lending | reference
    updated_at       TEXT NOT NULL,
    version          INTEGER NOT NULL DEFAULT 1
);

-- §32.3.7 — typed, sourced, recomputable quality signals. The composite
-- score is instance configuration (ADR 0019): filters are the product,
-- never a public leaderboard, and never purchasable (§0.3).
CREATE TABLE IF NOT EXISTS quality_signals (
    id          TEXT PRIMARY KEY,
    work_id     TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    signal_kind TEXT NOT NULL,                   -- editorial_review | quorum_distinction | completeness | maturity | reader_positivity
    value       INTEGER NOT NULL,
    weight      INTEGER NOT NULL DEFAULT 1,
    source      TEXT NOT NULL,
    computed_at TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_quality_signals_work ON quality_signals(work_id);
CREATE INDEX IF NOT EXISTS idx_quality_signals_kind ON quality_signals(signal_kind, value);

-- §30.1 / §32.3.8 — format is a first-class property of a work; every row
-- that existed before M22 is prose.
ALTER TABLE works ADD COLUMN format TEXT NOT NULL DEFAULT 'prose';
