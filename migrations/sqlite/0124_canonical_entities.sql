-- 0124: the taxonomy becomes the canonical entities (M6-01).
--
-- Spec §5: the canonical term is **canonical entity**. Spec §6.4 names the
-- columns. And the specification's own vocabulary table names the table
-- `canonical_entities` / `taxonomy_nodes` -- either spelling, because the
-- instance is not supposed to have two vocabularies.
--
-- IT HAS TWO.
--
-- `taxonomy_nodes` (0011) is the live vocabulary: what the tag browser reads,
-- what `search_nodes` searches, and what `work_tags` references. And
-- `canonical_entities` (0081) is the exchange's **own** table, carrying the
-- curator's account and the canonical form a curator set.
--
-- Migration 0082 saw the duplication and could not fix it: it put
-- `review_status` and `signal_count` on the taxonomy's node, and the exchange
-- kept writing to its own table. So the counts a signal reinforced never
-- reached the node the review queue reads, and the two tables described one
-- state in two spellings of two shapes.
--
-- So this migration **merges them** rather than renaming one and leaving the
-- other:
--
--   1. the columns of both tables are on one table;
--   2. the exchange's rows are folded into the taxonomy's, and their signal
--      counts **add** -- a name the exchange has seen forty times and the
--      taxonomy knows about is a name with forty signals;
--   3. the exchange's table is dropped;
--   4. the taxonomy's table takes the name.
--
-- THE DATA IS NOT REWRITTEN
--
-- A rename is a change of vocabulary and not of content, and a migration that
-- rewrote content would be a migration with a data-loss story. The rows keep
-- their identities, their spellings and their counts.
--
-- THE COLUMNS
--
-- `slug` and `parent_id` are new and empty. `canonical_name` is the
-- **curator-set** canonical form, which is the one value the exchange's table
-- carried that the taxonomy's did not; the display name stays in `canonical`,
-- because that is what every reader-facing surface already reads.
--
-- `work_count` is the floor's input (spec §6.4) and starts at zero: a
-- recount is a bin's job, and a default of zero is honest about a number
-- nobody has counted yet.
--
-- Dialect: SQLite.
--
-- ── the columns of both tables are on one table ────────────────────────────

ALTER TABLE taxonomy_nodes ADD COLUMN updated_at TEXT;
ALTER TABLE taxonomy_nodes ADD COLUMN curated_by TEXT;
ALTER TABLE taxonomy_nodes ADD COLUMN curated_at TEXT;
-- The curator-set canonical form, carried over from the exchange's table.
ALTER TABLE taxonomy_nodes ADD COLUMN canonical_name TEXT;
-- A URL-safe name, derived from the canonical form. Empty until the browse
-- needs one; the column is here so the shape is settled before the data is.
ALTER TABLE taxonomy_nodes ADD COLUMN slug TEXT;
-- The entity this one sits under, for the tree.
ALTER TABLE taxonomy_nodes ADD COLUMN parent_id TEXT REFERENCES taxonomy_nodes(id) ON DELETE SET NULL;
-- The floor's input (spec §6.4). Zero until a recount says otherwise.
ALTER TABLE taxonomy_nodes ADD COLUMN work_count INTEGER NOT NULL DEFAULT 0;

-- ── the exchange's rows fold into the taxonomy's ───────────────────────────
--
-- `INSERT OR IGNORE` skips the (kind, norm) pairs the taxonomy already
-- knows, and the unique index on (kind, norm) is what makes it skip them.
--
-- **The count is zero here, not the exchange's.** The exchange's count is
-- added by the UPDATE below, and a row inserted with it would be counted
-- twice -- the counts *add*, and adding the exchange's count to a row that
-- already carries it is a doubling.
--
-- The id is minted here because the exchange's table had no id: it was keyed
-- by (kind, norm) alone. The bytes are a UUID, formatted from SQLite's
-- randomblob, with the version and variant bits set so it is a well-formed
-- UUID and not merely 32 hex characters.
INSERT OR IGNORE INTO taxonomy_nodes
    (id, kind, canonical, norm, created_at, updated_at, review_status, signal_count,
     canonical_name, curated_by, curated_at)
SELECT
    lower(substr(h, 1, 8)) || '-' || substr(h, 9, 4) || '-4' || substr(h, 14, 3)
        || '-8' || substr(h, 18, 3) || '-' || substr(h, 21, 12),
    ce.kind, ce.canonical, ce.norm, ce.created_at, ce.updated_at,
    ce.review_status, 0, ce.canonical, ce.curated_by, ce.curated_at
FROM canonical_entities ce
CROSS JOIN (SELECT lower(hex(randomblob(16))) AS h) AS r;

-- ── the counts add ────────────────────────────────────────────────────────
--
-- A name the exchange has seen forty times and the taxonomy knows about is a
-- name with forty signals, and the review queue's ordering is that count.
-- Adding rather than replacing is the only reading that does not lose the
-- signals one of the two tables already recorded.
UPDATE taxonomy_nodes
   SET signal_count = signal_count + (
       SELECT COALESCE(SUM(ce.signal_count), 0)
         FROM canonical_entities ce
        WHERE ce.kind = taxonomy_nodes.kind AND ce.norm = taxonomy_nodes.norm);

-- ── a curated exchange row curates the node ───────────────────────────────
--
-- Reinforcing an unverified entity does not curate it, and curating does not
-- happen as a side effect of being agreed with. But a row the exchange
-- records as *curated* was curated by somebody, and that somebody's word is
-- the one that counts.
UPDATE taxonomy_nodes
   SET review_status = 'curated'
 WHERE EXISTS (
       SELECT 1 FROM canonical_entities ce
        WHERE ce.kind = taxonomy_nodes.kind AND ce.norm = taxonomy_nodes.norm
          AND ce.review_status = 'curated');

-- The curator's account and the moment, where the exchange had them.
UPDATE taxonomy_nodes
   SET curated_by = (SELECT ce.curated_by FROM canonical_entities ce
                      WHERE ce.kind = taxonomy_nodes.kind AND ce.norm = taxonomy_nodes.norm),
       curated_at = (SELECT ce.curated_at FROM canonical_entities ce
                      WHERE ce.kind = taxonomy_nodes.kind AND ce.norm = taxonomy_nodes.norm)
 WHERE EXISTS (
       SELECT 1 FROM canonical_entities ce
        WHERE ce.kind = taxonomy_nodes.kind AND ce.norm = taxonomy_nodes.norm
          AND ce.curated_by IS NOT NULL);

-- ── the exchange's table goes ──────────────────────────────────────────────
--
-- Its rows are in the taxonomy's now, and the two tables described one state
-- in two shapes, which is how a "curated" label ends up meaning "exists in
-- the new table" rather than "was reviewed".
DROP TABLE canonical_entities;

-- ── the rename ────────────────────────────────────────────────────────────
--
-- The table first, then the indexes: SQLite has no `ALTER INDEX ... RENAME
-- TO`, so an index is dropped and recreated under its new name. The
-- *definition* is unchanged -- an index that changed shape would be a
-- migration that quietly made a query faster, and a query that got faster for
-- no reason is a query nobody can explain later.
--
-- The foreign keys that point at the renamed table are rewritten by SQLite
-- itself, which is why the rename comes after the columns and not before.
ALTER TABLE taxonomy_nodes RENAME TO canonical_entities;

DROP INDEX taxonomy_nodes_kind_norm;
CREATE UNIQUE INDEX canonical_entities_kind_norm ON canonical_entities (kind, norm);
DROP INDEX taxonomy_nodes_norm;
CREATE INDEX canonical_entities_norm ON canonical_entities (norm);
DROP INDEX taxonomy_nodes_review;
CREATE INDEX canonical_entities_review ON canonical_entities (review_status);

-- The review queue's own ordering (spec §15.17: the name with forty distinct
-- signals is examined before the name with one). The index lived on the
-- exchange's own table, which this migration drops, so it is recreated here:
-- the queue reads the merged table now, and an index left behind on a
-- dropped table is an index that does not exist.
CREATE INDEX canonical_entities_queue ON canonical_entities (review_status, signal_count DESC);

-- ── the tables around it take the entity vocabulary ───────────────────────
--
-- The aliases, the edges, the closure and the scope are all *about* entities,
-- and the specification's vocabulary is `entity_alias` (spec §6.4) with the
-- closure the `+children` expansion is a single join against.
--
-- **After the parent's rename**, because SQLite rewrites the foreign keys
-- that point at a renamed table -- so the children follow the parent to its new
-- name for free, and renaming them first would leave them pointing at a table
-- that is about to change its name.
ALTER TABLE taxonomy_aliases RENAME TO entity_aliases;
ALTER TABLE taxonomy_node_scope RENAME TO entity_scope;
ALTER TABLE taxonomy_edges RENAME TO entity_edges;
ALTER TABLE taxonomy_closure RENAME TO entity_closure;

-- The indexes are dropped and recreated under their new names, so the two
-- dialects **declare the same index names in the same migration**. The
-- *definitions* are unchanged -- an index that changed shape would be a
-- migration that quietly made a query faster, and a query that got faster for
-- no reason is a query nobody can explain later.
DROP INDEX taxonomy_nodes_status_idx;
CREATE INDEX canonical_entities_status_idx ON canonical_entities (status);
DROP INDEX taxonomy_nodes_merged_into_idx;
CREATE INDEX canonical_entities_merged_into_idx ON canonical_entities (merged_into);

DROP INDEX taxonomy_aliases_norm;
CREATE INDEX entity_aliases_norm ON entity_aliases (norm);
DROP INDEX taxonomy_aliases_node;
CREATE INDEX entity_aliases_node ON entity_aliases (node_id);

DROP INDEX taxonomy_node_scope_fandom_idx;
CREATE INDEX entity_scope_fandom_idx ON entity_scope (fandom_node_id);

DROP INDEX taxonomy_edges_src_idx;
CREATE INDEX entity_edges_src_idx ON entity_edges (src_id);
DROP INDEX taxonomy_edges_rel_idx;
CREATE INDEX entity_edges_rel_idx ON entity_edges (rel);

DROP INDEX taxonomy_closure_ancestor_idx;
CREATE INDEX entity_closure_ancestor_idx ON entity_closure (ancestor_id, rel);
DROP INDEX taxonomy_closure_descendant_idx;
CREATE INDEX entity_closure_descendant_idx ON entity_closure (descendant_id);
