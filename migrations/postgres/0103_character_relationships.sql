-- Migration 0103 — the character/relationship substrate that §15.3's
-- `ExistsCharacter` and `ExistsRelationship` need (M46-01).
--
-- Dialect: PostgreSQL.
--
-- Mirrors migrations/sqlite/0103_character_relationships.sql statement for
-- statement, in the same order, with the same columns and the same CHECK
-- constraints. The parity test compares declared column SETS and index column
-- lists and is blind to column *types*, so every difference below is deliberate
-- and is the complete list of them.
--
-- The four differences, all forced by the engine:
--
--   1. `work_id` is UUID here and TEXT on SQLite, because `works.id` is UUID
--      (0003). This is the ONLY column in this migration that changes type.
--
--   2. `taxonomy_nodes.id` is TEXT on **both** dialects — 0011 declares it TEXT in
--      both arms — so `character_node_id`, `attribute_node_id` and
--      `ship_node_id` are TEXT here too, with no cast on either engine. This is
--      the opposite of the usual pattern and worth stating because the instinct
--      is wrong: an id that looks like a UUID usually is one, and here it is not.
--
--   3. `is_pov` is BIGINT, not INTEGER. PostgreSQL maps INTEGER to INT4 (32-bit)
--      while the store reads i64, which is a decode error even when the bind was
--      correct. 0003 does the same for `works.show_public_ratings`.
--
--   4. `confidence` is DOUBLE PRECISION, not REAL, so a confidence round-trips
--      identically on both engines.
--
-- Three things are *better* here, and deliberately not emulated on SQLite:
--
--   * The `work_tags` CHECKs are real CHECK constraints. SQLite cannot ALTER a
--     table to add one, so the SQLite file uses triggers instead — the same rules,
--     enforced differently, because the alternative was rebuilding `work_tags`.
--   * A composite FOREIGN KEY needs no trigger, and neither does anything else
--     here: every CHECK in the SQLite file has a direct PostgreSQL equivalent.
--   * `RAISE(ABORT, ...)` has no counterpart, so the SQLite triggers are the only
--     construct in this migration with no PostgreSQL twin.

CREATE TABLE work_characters (
    -- UUID here, TEXT on SQLite: follows `works.id` (0003).
    work_id            UUID    NOT NULL REFERENCES works (id) ON DELETE CASCADE,

    -- TEXT on both dialects -- `taxonomy_nodes.id` is TEXT on both (0011).
    character_node_id  TEXT    NOT NULL REFERENCES taxonomy_nodes (id) ON DELETE CASCADE,

    -- §15.1: `prominence = protagonist`.
    -- protagonist | supporting | cameo | mentioned.
    -- BIGINT rather than INTEGER: PostgreSQL's INTEGER is INT4, and the store
    -- reads i64.
    prominence         TEXT    NOT NULL DEFAULT 'supporting',
    is_pov             BIGINT  NOT NULL DEFAULT 0,

    added_at           TEXT    NOT NULL,

    PRIMARY KEY (work_id, character_node_id),

    CONSTRAINT work_characters_prominence_ck
        CHECK (prominence IN ('protagonist', 'supporting', 'cameo', 'mentioned')),
    CONSTRAINT work_characters_is_pov_ck
        CHECK (is_pov IN (0, 1))
);

CREATE INDEX work_characters_character
    ON work_characters (character_node_id, work_id);

-- §15.1: `attributes = [vampire, BAMF]`.
--
-- The character is part of the key, which is what makes §15.3's correlation rule
-- enforceable in the schema rather than only in the compiler: joining from
-- `work_characters` on (work_id, character_node_id) cannot reach another
-- character's attributes, because another character's attributes have a different
-- key.
CREATE TABLE work_character_attributes (
    work_id            UUID    NOT NULL,
    character_node_id  TEXT    NOT NULL,
    attribute_node_id  TEXT    NOT NULL REFERENCES taxonomy_nodes (id) ON DELETE CASCADE,
    added_at           TEXT    NOT NULL,

    PRIMARY KEY (work_id, character_node_id, attribute_node_id),

    -- Composite FK rather than two single-column FKs: this makes "the attribute
    -- belongs to a character who is actually in this work" true in the database.
    -- With two independent FKs a row could name a character not present in the
    -- work at all, and no query would notice.
    FOREIGN KEY (work_id, character_node_id)
        REFERENCES work_characters (work_id, character_node_id) ON DELETE CASCADE
);

CREATE INDEX work_character_attributes_attribute
    ON work_character_attributes (attribute_node_id, work_id);

-- A ship's identity is its participant SET: no surrogate id, no ordering column,
-- so A/B and B/A are the same node.
CREATE TABLE ship_participants (
    ship_node_id       TEXT    NOT NULL REFERENCES taxonomy_nodes (id) ON DELETE CASCADE,
    character_node_id  TEXT    NOT NULL REFERENCES taxonomy_nodes (id) ON DELETE CASCADE,

    PRIMARY KEY (ship_node_id, character_node_id)
);

-- §15.2: `Kind = romantic`, `Prominence = central`, `Dynamics = [...]`.
--
-- rel_type belongs to the WORK's claim, not to the ship node: the same pair can be
-- romantic in one fic and platonic in another, so a ship node cannot carry it.
CREATE TABLE work_relationships (
    id                 TEXT    PRIMARY KEY,
    work_id            UUID    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    ship_node_id       TEXT    NOT NULL REFERENCES taxonomy_nodes (id) ON DELETE CASCADE,
    rel_type           TEXT    NOT NULL,
    prominence         TEXT    NOT NULL DEFAULT 'secondary',
    dynamics           TEXT,
    label              TEXT,
    added_at           TEXT    NOT NULL,

    -- One claim per (work, pairing, type). Without it a work could say both
    -- "romantic" and "platonic" for one ship, and a `NOT ... type:romantic`
    -- exclusion would silently exclude it anyway.
    CONSTRAINT work_relationships_unique_claim
        UNIQUE (work_id, ship_node_id, rel_type),

    CONSTRAINT work_relationships_rel_type_ck
        CHECK (rel_type IN ('romantic', 'platonic', 'familial', 'qpp', 'sexual',
                            'antagonistic', 'other')),
    CONSTRAINT work_relationships_prominence_ck
        CHECK (prominence IN ('primary', 'secondary', 'background'))
);

CREATE INDEX work_relationships_work
    ON work_relationships (work_id, ship_node_id);

CREATE INDEX work_relationships_ship
    ON work_relationships (ship_node_id, work_id);

-- Tag prominence, source, confidence, status. Mirrors the SQLite file's ALTERs.
--
-- DOUBLE PRECISION for confidence so it round-trips identically on both engines.
ALTER TABLE work_tags
    ADD COLUMN prominence TEXT NOT NULL DEFAULT 'secondary';

ALTER TABLE work_tags
    ADD COLUMN source TEXT NOT NULL DEFAULT 'author';

ALTER TABLE work_tags
    ADD COLUMN confidence DOUBLE PRECISION NOT NULL DEFAULT 1.0;

ALTER TABLE work_tags
    ADD COLUMN status TEXT NOT NULL DEFAULT 'applied';

-- Real CHECKs here, where the SQLite file needs triggers. Same four rules, same
-- reasons -- and the `source` rule is ADR 0026's decision expressed where it
-- cannot be bypassed: §49.2's "only confirmed tags count toward gravity" is a
-- database invariant, not a convention every write path has to remember.
ALTER TABLE work_tags
    ADD CONSTRAINT work_tags_prominence_ck
    CHECK (prominence IN ('primary', 'secondary', 'background'));

ALTER TABLE work_tags
    ADD CONSTRAINT work_tags_source_ck
    CHECK (source = 'author');

ALTER TABLE work_tags
    ADD CONSTRAINT work_tags_status_ck
    CHECK (status IN ('applied', 'suggested', 'rejected'));
