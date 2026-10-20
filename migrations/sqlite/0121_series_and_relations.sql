-- 0121: series, series entries and work relations (M4-04).
--
-- Spec §6.2, §11.5. A **series** is an ordered set of works; a **work
-- relation** is an explicit statement that one work stands in a named
-- relation to another.
--
-- NOT THE SAME THING AS `work_relationships`
--
-- The schema already has a `work_relationships` table, and the two names differ
-- by one letter:
--
--   * `work_relationships` is a TAXONOMIC relationship — "these two characters
--     are enemies-to-lovers" — and it lives in the taxonomy family.
--   * `work_relations` is a CONTENT relationship — "this is the next one in the
--     series" — and it lives in the content family.
--
-- They are different facts about different things, and the specification lists
-- both (the content list and the taxonomy list). The name is the
-- specification's, and the difference is called out here because a reader of
-- the schema will otherwise assume one of them is a typo for the other.
--
-- THE RELATION VOCABULARY IS CLOSED
--
-- Five relations, and the CHECK is the same list the domain's enum holds. A
-- relation outside it is not a relation the instance can explain, and a
-- free-text relation would make "next in series" and "the next one, probably"
-- indistinguishable — which is exactly the distinction the ladder's first rung
-- depends on.
--
-- `next_in_series` is the one the ladder's first rung reads: **the explicit
-- relation** (spec §11.5). It is explicit rather than inferred from a series'
-- position because a series' order is the author's and a relation is a
-- statement about two works that may not be in a series at all.
--
-- THE SERIES' ORDER IS THE AUTHOR'S
--
-- `series_entries.position` is the order, and there is no other order. A
-- "smart" ordering would have to be derived from a signal, and no signal here
-- could support one — inventing an order would silently contradict the author.

CREATE TABLE series (
    id            TEXT    PRIMARY KEY,
    -- The stable handle, in the URL.
    slug          TEXT    NOT NULL UNIQUE,
    title         TEXT    NOT NULL,
    description   TEXT    NOT NULL DEFAULT '',
    -- Who curates it. `ON DELETE SET NULL`, not CASCADE: a series outlives the
    -- pseud that made it, because a deleted author's series is still a series
    -- and its works are still published.
    owner_pseud_id TEXT   REFERENCES pseuds (id) ON DELETE SET NULL,
    created_at    TEXT    NOT NULL,
    updated_at    TEXT    NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE series_entries (
    id         TEXT    PRIMARY KEY,
    series_id  TEXT    NOT NULL REFERENCES series (id) ON DELETE CASCADE,
    work_id    TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    -- The author's order. The only order there is.
    position   INTEGER NOT NULL,
    created_at TEXT    NOT NULL,
    updated_at TEXT    NOT NULL,
    version    INTEGER NOT NULL DEFAULT 1,
    -- A work is in a series once. A second entry would be a second ordering of
    -- the same work, and two orderings of one thing is one too many.
    UNIQUE (series_id, work_id)
);

CREATE INDEX series_entries_series_position ON series_entries (series_id, position);
CREATE INDEX series_entries_work ON series_entries (work_id);

CREATE TABLE work_relations (
    id            TEXT    PRIMARY KEY,
    from_work_id  TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    to_work_id    TEXT    NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    relation_type TEXT    NOT NULL
        CHECK (relation_type IN ('next_in_series', 'prequel', 'sequel',
                                 'side_story', 'translation_of')),
    created_at    TEXT    NOT NULL,
    updated_at    TEXT    NOT NULL,
    version       INTEGER NOT NULL DEFAULT 1,
    UNIQUE (from_work_id, to_work_id, relation_type)
);

CREATE INDEX work_relations_from ON work_relations (from_work_id);
CREATE INDEX work_relations_to ON work_relations (to_work_id);
