-- 0127: the character's roles and the relationship's dynamics are sets, and a
-- relationship is the entity's kind (M6-05).
--
-- Spec §6.4: a **character_assertion** is "prominence + roles + attributes",
-- and a **relationship_assertion** is "participants (a *set*, >2 supported) +
-- kind + prominence + dynamics".
--
-- WHAT THE ESTATE ALREADY HAD, AND WHAT IT DID NOT
--
-- 0103 built `work_characters` (prominence + is_pov), `work_character_attributes`
-- (keyed with its character, so §15.3's correlation rule is structural),
-- `ship_participants` (a set with no ordinal, so A/B and B/A are one pairing)
-- and `work_relationships`. So the *assertions* were there. Two of the facets
-- were not:
--
--   * **roles** had no table at all -- §6.4 names them on the assertion;
--   * **dynamics** were a comma-separated string of node ids in
--     `work_relationships.dynamics`, and 0103 said so itself: *"Stored as a
--     comma-separated list of node ids for now, because a dynamics node needs a
--     controlled vocabulary this migration does not get to invent."*
--
-- A list in a string is not a set: it cannot be joined, it cannot be indexed,
-- membership is a `LIKE` on a delimiter, and a row with `a,b` and a row with
-- `b,a` are different strings describing the same claim. So dynamics become
-- rows, and the column goes.
--
-- WHERE THE VOCABULARY LIVES
--
-- **Not in this migration, and not in a `CHECK`.** A dynamic ("dominance",
-- "submission", "slow burn") is a *named thing*, and this instance has exactly
-- one table that owns names: `canonical_entities` (M6-01). So a role and a
-- dynamic are entity references, the same shape as an attribute, and the
-- vocabulary is curated in the taxonomy rather than compiled into a migration
-- that a curator cannot amend. That is also what keeps the three facets one
-- kind of claim: each is "this work says this named thing about this character
-- (or this pairing)".
--
-- A DYNAMIC IS NOT FREEZING THE RELATIONSHIP
--
-- `work_relationship_dynamics` references the **relationship row**, not the
-- (work, ship) pair: a work can claim "romantic" and "platonic" about the same
-- pairing (§15.2's rule, enforced by `work_relationships`' UNIQUE), and each of
-- those claims has its own dynamics. Keying on the pairing would put
-- `enemies_to_lovers` on both.
--
-- AND THE KIND'S WORD IS THE SPECIFICATION'S
--
-- §6.4's entity kinds are `freeform · fandom · character · relationship ·
-- author · rating · warning · mood · category · theme`. The estate spelled the
-- pairing's kind `ship`, so the kind, the participant table and the column that
-- points at the node all move to `relationship`. **The query field stays
-- `ship:`** -- §11.2 names that field, and a field name is a contract with the
-- clients that already send it, exactly as the URL is (`/chapters` kept its
-- shape when the model became `units`, M4-02).
--
-- THE DATA IS NOT LOST
--
-- The old dynamics are split into rows before the column is dropped, so an
-- instance that had `enemies_to_lovers,slow_burn` on a relationship keeps both,
-- as two rows. The split is the only thing in this file that differs between
-- the two engines' SQL: SQLite has `instr`, PostgreSQL has `strpos`, and each
-- twin says what it means.
--
-- Dialect: SQLite.

-- ---------------------------------------------------------------------------
-- 1. The kind is the specification's word.
-- ---------------------------------------------------------------------------

UPDATE canonical_entities SET kind = 'relationship' WHERE kind = 'ship';

-- ---------------------------------------------------------------------------
-- 2. The participant set follows the kind.
-- ---------------------------------------------------------------------------

ALTER TABLE ship_participants RENAME TO relationship_participants;

-- Both columns are TEXT on both engines (0011), so this rename is a rename and
-- not a retype.
ALTER TABLE relationship_participants
    RENAME COLUMN ship_node_id TO relationship_node_id;

ALTER TABLE work_relationships
    RENAME COLUMN ship_node_id TO relationship_node_id;

-- The reverse index is named after the column it leads with; a index called
-- `work_relationships_ship` on `relationship_node_id` is a name that is now
-- wrong. SQLite has no `ALTER INDEX`, so it is dropped and recreated.
DROP INDEX work_relationships_ship;
CREATE INDEX work_relationships_relationship
    ON work_relationships (relationship_node_id, work_id);

-- ---------------------------------------------------------------------------
-- 3. The character's roles (§6.4: prominence + roles + attributes).
-- ---------------------------------------------------------------------------

CREATE TABLE work_character_roles (
    work_id            TEXT NOT NULL,

    -- The character is part of the key, and the composite foreign key below is
    -- what makes "the role belongs to a character that is actually in this
    -- work" true in the database rather than only in the store. Same rule as
    -- `work_character_attributes` in 0103: with two single-column keys a row
    -- could name a character who is not in the work, and every query would
    -- still return a plausible answer.
    character_node_id  TEXT NOT NULL,
    role_node_id       TEXT NOT NULL REFERENCES canonical_entities (id) ON DELETE CASCADE,
    added_at           TEXT NOT NULL,

    PRIMARY KEY (work_id, character_node_id, role_node_id),

    FOREIGN KEY (work_id, character_node_id)
        REFERENCES work_characters (work_id, character_node_id) ON DELETE CASCADE
);

-- "Characters who are mentors" -- the reverse direction, and the one a facet
-- needs. The primary key already serves (work, character).
CREATE INDEX work_character_roles_role
    ON work_character_roles (role_node_id, work_id);

-- ---------------------------------------------------------------------------
-- 4. The relationship's dynamics, as a set.
-- ---------------------------------------------------------------------------

CREATE TABLE work_relationship_dynamics (
    -- The claim, not the pairing: a work may hold `romantic` and `platonic`
    -- claims about one pairing, and each has its own dynamics.
    relationship_id    TEXT NOT NULL REFERENCES work_relationships (id) ON DELETE CASCADE,
    dynamic_node_id    TEXT NOT NULL REFERENCES canonical_entities (id) ON DELETE CASCADE,
    added_at           TEXT NOT NULL,

    PRIMARY KEY (relationship_id, dynamic_node_id)
);

CREATE INDEX work_relationship_dynamics_dynamic
    ON work_relationship_dynamics (dynamic_node_id, relationship_id);

-- The backfill, before the column goes. The list was comma-separated with no
-- spaces promised by anything, so each element is trimmed and empty elements
-- are dropped: `a,,b` is two dynamics, not three.
INSERT INTO work_relationship_dynamics (relationship_id, dynamic_node_id, added_at)
WITH RECURSIVE split (id, rest, added_at) AS (
    SELECT id, dynamics || ',', added_at
      FROM work_relationships
     WHERE dynamics IS NOT NULL AND TRIM(dynamics) <> ''
    UNION ALL
    SELECT id, substr(rest, instr(rest, ',') + 1), added_at
      FROM split
     WHERE instr(rest, ',') > 0
)
SELECT id, TRIM(substr(rest, 1, instr(rest, ',') - 1)), added_at
  FROM split
 WHERE TRIM(substr(rest, 1, instr(rest, ',') - 1)) <> '';

ALTER TABLE work_relationships DROP COLUMN dynamics;
