-- 0125: the entity floor (M6-02).
--
-- Spec §6.4: "A canonical entity is created only when its tag clears a
-- **work-count floor**; below the floor the assertion is still stored, still
-- searchable and still countable, but **it is not a node in the tree**."
--
-- WHY THE FLOOR IS NOT OPTIONAL
--
-- 97 % of the freeform namespace is singletons: 587,258 of 634,190 names are
-- on exactly one work. Without a floor, "browse the taxonomy" is 634,231
-- rows of one-off user phrases, and every count on the page is a count of
-- one. The default floor is **5 works**, configurable per kind, and it is the
-- same distinction the AO3 recommender already drew: *"catalog size says an
-- entity exists; co-bookmarker evidence says someone in the comparison set
-- has read it."*
--
-- THE STATE IS A COLUMN AND NOT A JOIN
--
-- `node_state` is `browsable` | `assertion_only`. The alternative -- a
-- subquery counting the works -- would make the browse's own filter depend
-- on a count over the largest table in the schema, and the count changes
-- with every tag written.
--
-- The default is `browsable`, and that is deliberate: **every entity that
-- exists today was created by a human** through the taxonomy UI, and the
-- established vocabulary is not a set of singletons. Defaulting to
-- `assertion_only` would empty the browse of the whole existing corpus the
-- moment this migration ran -- and it would be invisible.
--
-- An entity created **by a signal** is `assertion_only` until a recount
-- says otherwise, and that is set by the code rather than by the default:
-- a name nobody has curated and nobody has attached to five works is an
-- assertion, not a node.
--
-- `work_count` is the floor's input and it is **denormalised on purpose**:
-- it is the number the floor is tested against, and a number that is
-- computed on every browse is a number that is wrong the moment a work is
-- tagged. The recount is a bin's job and it is idempotent.
--
-- Dialect: SQLite.
ALTER TABLE canonical_entities ADD COLUMN node_state TEXT NOT NULL DEFAULT 'browsable';

-- The floor's input. Zero until a recount says otherwise, and a recount is
-- the honest way to fill it: a default of zero would put every entity below
-- every floor.
ALTER TABLE canonical_entities ADD COLUMN node_count INTEGER NOT NULL DEFAULT 0;

-- The browse's filter, served from the index rather than from a count over
-- the largest table in the schema.
CREATE INDEX canonical_entities_node_state ON canonical_entities (kind, node_state, canonical);

-- The recount reads the work counts by node, so the index has to answer
-- "which nodes carry this many works" without a scan of `work_tags`.
CREATE INDEX work_tags_node_work ON work_tags (node_id, work_id);
