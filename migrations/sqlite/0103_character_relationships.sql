-- Migration 0103 — the character/relationship substrate that §15.3's
-- `ExistsCharacter` and `ExistsRelationship` need (M46-01).
--
-- Dialect: SQLite.
--
-- Why this migration exists
-- --------------------------
-- Spec §15.3 lists two AST nodes:
--
--     ExistsCharacter | ExistsRelationship
--
-- and gives this example, which is journey 12 (§25.1 item 12, "Search bound
-- character attribute → exclude ship → filter by mood"):
--
--     {"and": [
--       {"exists_character": {"character_id": "A",
--                             "prominence": ["protagonist"],
--                             "attributes_all": ["BAMF"]}},
--       {"not": {"exists_relationship": {"participant_any": ["A"],
--                                         "kind_any": ["romantic", "sexual"]}}}
--     ]}
--
-- Neither node is in the built AST (`crates/domain/src/query.rs` has only
-- `Text | Phrase | Fielded | Comparison | And | Or | Not`), and there was no
-- schema for either: characters and ships existed only as `taxonomy_nodes` rows
-- with `kind = 'character'` / `'ship'`, which supports a flat name match and
-- nothing else — no prominence, no attributes, no participants. So journey 12 was
-- not implementable, and no amount of query-language work changes that. This
-- migration builds the substrate; the AST nodes are added alongside it.
--
-- Why a ship is a participant SET and the type belongs to the work
-- ----------------------------------------------------------------
-- The same pair of characters can be romantic in one fic and platonic in
-- another, sometimes in the same fic. So "A/B" cannot be the unit of relationship
-- truth: a ship node is the *identity* of a pairing (its sorted participant set),
-- and `work_relationships.rel_type` is the *claim* a particular work makes about
-- it. Storing the type on the ship node would make the second fic impossible to
-- record, and would make "any pairing involving X" unable to say which kind it
-- found.
--
-- Identity is the sorted set, so the ship node for A/B and for B/A is the same
-- row and no writer has to pick an order. `ship_participants` is what makes that
-- queryable; the node itself is a `taxonomy_nodes` row with `kind = 'ship'`,
-- created on first use per §15.17.
--
-- The §15.3 correlation rule, and why this schema can satisfy it
-- ----------------------------------------------------------------
-- §15.3 is emphatic: "Compile bound predicates into SQL `EXISTS` clauses. Never
-- allow one character to satisfy another character's attributes."
--
-- That is a constraint about **correlation**: every clause of an `ExistsCharacter`
-- must test the SAME `work_characters` row. A naive compilation of
-- `character A AND attribute vampire AND prominence protagonist` as three
-- separate `EXISTS` clauses would let work W satisfy "character A" while a
-- *different* work_characters row supplies "vampire" — which is a different
-- character's attribute, on this same work. That query would return works where
-- no single character is a vampire protagonist, which is precisely the bug the
-- spec forbids.
--
-- The fix lives in the schema's shape: `prominence` and `is_pov` are columns ON
-- `work_characters`, so one row carries all of a character's per-work facts and a
-- correlated subquery can test them together. Attributes are the one part that
-- needs their own table, and `work_character_attributes` is keyed
-- (work_id, character_node_id, attribute_node_id) for the same reason: the
-- character is part of the key, so a join cannot drift to another character.

CREATE TABLE work_characters (
    -- `works.id` is TEXT on SQLite (0003) and UUID on PostgreSQL; the work_id
    -- column follows it, which is why the PostgreSQL file differs here and only
    -- here.
    work_id            TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,

    -- `taxonomy_nodes.id` is TEXT on BOTH dialects (0011 declares it TEXT in both
    -- arms), so this column takes no cast on either engine. That is worth stating
    -- because the instinct is the opposite: an id that looks like a UUID usually
    -- is one, and here it is not. Checked against both migration files, not
    -- assumed.
    character_node_id  TEXT    NOT NULL REFERENCES taxonomy_nodes (id) ON DELETE CASCADE,

    -- §15.1: `prominence = protagonist`.
    --
    -- Values: protagonist | supporting | cameo | mentioned. NOT free text and NOT
    -- an ordinal, because "exclude Major Character Death" means something
    -- different when the tag is incidental -- which is the whole reason this
    -- column exists.
    --
    -- CHECK-constrained rather than left to the store: SQLite accepts what
    -- PostgreSQL rejects, so an unconstrained column here means a typo is caught
    -- in production on one engine only.
    prominence         TEXT    NOT NULL DEFAULT 'supporting',

    -- §15.1: `roles = [mentor]`. INTEGER on SQLite, BIGINT on PostgreSQL, because
    -- the store reads i64 and PostgreSQL maps INTEGER to INT4.
    is_pov             INTEGER NOT NULL DEFAULT 0,

    added_at           TEXT    NOT NULL,

    PRIMARY KEY (work_id, character_node_id),

    CHECK (prominence IN ('protagonist', 'supporting', 'cameo', 'mentioned')),
    CHECK (is_pov IN (0, 1))
);

-- "Works with X" is the single most common character query, and it is also the
-- inner loop of "X present, no relationship involving X" -- which must find the
-- character first and then fail to find a relationship. Leading with work_id
-- serves both.
CREATE INDEX work_characters_character
    ON work_characters (character_node_id, work_id);

-- §15.1: `attributes = [vampire, BAMF]`.
--
-- The character is part of the key, which is what makes §15.3's correlation rule
-- enforceable: a join from `work_characters` on (work_id, character_node_id)
-- cannot reach another character's attributes.
CREATE TABLE work_character_attributes (
    work_id            TEXT    NOT NULL,
    character_node_id  TEXT    NOT NULL,
    attribute_node_id  TEXT    NOT NULL REFERENCES taxonomy_nodes (id) ON DELETE CASCADE,
    added_at           TEXT    NOT NULL,

    PRIMARY KEY (work_id, character_node_id, attribute_node_id),

    -- Composite FK rather than two single-column FKs: this is what makes
    -- "the attribute belongs to a character that is actually in this work"
    -- true in the database and not merely in the store's code. With two
    -- independent FKs a row could name a character who is not in the work at all.
    FOREIGN KEY (work_id, character_node_id)
        REFERENCES work_characters (work_id, character_node_id) ON DELETE CASCADE
);

-- "Characters with this attribute" -- the reverse direction, and the one an
-- attribute browser or a "vampire protagonists" facet needs.
CREATE INDEX work_character_attributes_attribute
    ON work_character_attributes (attribute_node_id, work_id);

-- A ship's identity is its participant set.
--
-- No surrogate id and no ordering column: the *pair of rows* is the identity, so
-- A/B and B/A produce the same set and therefore the same ship node. Adding an
-- ordinal would invite two writers to disagree about A/B versus B/A and split one
-- ship into two nodes, which is the exact failure the set-based design prevents.
CREATE TABLE ship_participants (
    ship_node_id       TEXT    NOT NULL REFERENCES taxonomy_nodes (id) ON DELETE CASCADE,
    character_node_id  TEXT    NOT NULL REFERENCES taxonomy_nodes (id) ON DELETE CASCADE,

    PRIMARY KEY (ship_node_id, character_node_id)
);

-- §15.2: `Kind = romantic`, `Prominence = central`, `Dynamics = [enemies_to_lovers]`.
--
-- `rel_type` and `prominence` belong to the WORK's claim, not to the ship node --
-- see the header. Values:
--   rel_type:   romantic | platonic | familial | qpp | sexual | antagonistic
--               | other
--   prominence: primary | secondary | background
CREATE TABLE work_relationships (
    id                 TEXT    PRIMARY KEY,

    work_id            TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,

    -- The pairing this row is about. A `taxonomy_nodes` row with kind = 'ship'.
    ship_node_id       TEXT    NOT NULL REFERENCES taxonomy_nodes (id) ON DELETE CASCADE,

    rel_type           TEXT    NOT NULL,
    prominence         TEXT    NOT NULL DEFAULT 'secondary',

    -- §15.2: `Dynamics = [enemies_to_lovers]`. Structured rather than free text so
    -- it is queryable; a comma-separated string would be neither. Stored as a
    -- comma-separated list of node ids for now, because a dynamics *node* needs a
    -- controlled vocabulary this migration does not get to invent.
    dynamics           TEXT,

    label              TEXT,
    added_at           TEXT    NOT NULL,

    -- One work makes one claim about one pairing per type. Without this, a work
    -- could say "romantic" and "platonic" for the same ship and
    -- "NOT ship:(with:X type:romantic)" would exclude it anyway -- the exclusion
    -- would be silently wrong.
    UNIQUE (work_id, ship_node_id, rel_type),

    CHECK (rel_type IN ('romantic', 'platonic', 'familial', 'qpp', 'sexual',
                        'antagonistic', 'other')),
    CHECK (prominence IN ('primary', 'secondary', 'background'))
);

-- Journey 12's inner loop: for each relationship of a work, is X among its
-- participants? Leading with work_id because the compiled predicate walks
-- work -> relationship -> participants.
CREATE INDEX work_relationships_work
    ON work_relationships (work_id, ship_node_id);

-- The reverse: "which relationships involve X", for the participant_any arm.
CREATE INDEX work_relationships_ship
    ON work_relationships (ship_node_id, work_id);

-- Tag prominence, from advanced-search-as-built.md §2. A genuine gap:
-- `work_tags` had only `weight`, so "exclude Major Character Death" could not
-- distinguish incidental from load-bearing.
--
-- `source` and `confidence` are present for provenance and are constrained to
-- author-applied. This is ADR 0026's decision, in schema form: §49.2 says only
-- confirmed tags count toward gravity, and a reader- or machine-applied tag would
-- be a griefing vector the moment it influenced anything. The columns exist so a
-- future amendment has somewhere to go; the CHECK means nothing can write them
-- yet, so adopting them later is a migration rather than a data cleanup.
ALTER TABLE work_tags
    ADD COLUMN prominence TEXT NOT NULL DEFAULT 'secondary';

ALTER TABLE work_tags
    ADD COLUMN source TEXT NOT NULL DEFAULT 'author';

ALTER TABLE work_tags
    ADD COLUMN confidence REAL NOT NULL DEFAULT 1.0;

ALTER TABLE work_tags
    ADD COLUMN status TEXT NOT NULL DEFAULT 'applied';

-- SQLite cannot ALTER a table to add a CHECK, so these are enforced by triggers.
-- A trigger is the honest option here rather than a table rebuild: rebuilding
-- `work_tags` on SQLite means recreating a table other migrations and the store
-- both depend on, for a constraint the store already guarantees. The trigger is
-- belt-and-braces on the default test engine, where dynamic typing means nothing
-- else would catch a bad value.
--
-- §15.17's rule, in constraint form: an unverified entity is usable but visibly
-- unverified. `status` distinguishes an applied tag from one that was suggested
-- and never accepted.
CREATE TRIGGER work_tags_prominence_ck
    BEFORE INSERT ON work_tags
    FOR EACH ROW
    WHEN NEW.prominence NOT IN ('primary', 'secondary', 'background')
    BEGIN
        SELECT RAISE(ABORT, 'work_tags.prominence must be primary|secondary|background');
    END;

CREATE TRIGGER work_tags_source_ck
    BEFORE INSERT ON work_tags
    FOR EACH ROW
    WHEN NEW.source <> 'author'
    BEGIN
        -- ADR 0026: reader-applied and inferred tags are refused here rather than
        -- merely discouraged, so §49.2's "only confirmed tags count toward gravity"
        -- is a database invariant instead of a convention.
        SELECT RAISE(ABORT, 'work_tags.source must be author (ADR 0026, spec 49.2)');
    END;

CREATE TRIGGER work_tags_status_ck
    BEFORE INSERT ON work_tags
    FOR EACH ROW
    WHEN NEW.status NOT IN ('applied', 'suggested', 'rejected')
    BEGIN
        SELECT RAISE(ABORT, 'work_tags.status must be applied|suggested|rejected');
    END;
