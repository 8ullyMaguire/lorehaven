-- M46-03 step 1: the taxonomy graph the advanced-search plan builds on.
--
-- The PostgreSQL half of `migrations/sqlite/0104_taxonomy_graph.sql`, which is
-- authoritative on the reasoning. Read that file first; this one records only what
-- differs between the dialects, and the two differences that matter are:
--
--   1. `ALTER TABLE ... ADD CONSTRAINT` is native here, and unlike SQLite the
--      constraint is enforced against EXISTING rows, not only new ones. The CHECKs
--      below are therefore a backfill gate, which is why they are stated over the
--      whole table rather than added per column.
--   2. `merged_into` is TEXT, not the plan's INTEGER. `taxonomy_nodes.id` is TEXT
--      here (migrations/postgres/0011_taxonomy.sql) and PostgreSQL will not coerce
--      integer to text inside a foreign key, so the plan's DDL as written cannot be
--      applied at all.
--
-- `scripts/probe-taxonomy-graph-pg.sh` applies this file to a real PostgreSQL 15 and
-- provokes every constraint, because "it applied" is not evidence that it bites.

ALTER TABLE taxonomy_nodes ADD COLUMN status TEXT NOT NULL DEFAULT 'active';

-- §15.17's curation state ('unverified' | 'curated', added by 0082) and this
-- node's lifecycle state ('active' | 'pending' | 'merged' | 'deprecated') are
-- different axes. Overloading one column would make "curated but merged"
-- inexpressible, so they stay separate.
ALTER TABLE taxonomy_nodes
    ADD CONSTRAINT taxonomy_nodes_status_ck
    CHECK (status IN ('active', 'pending', 'merged', 'deprecated'));

-- A self-reference so a merge cannot point at a node that does not exist, and so a
-- node merged into itself (a cycle of length one) is impossible. `ON DELETE SET
-- NULL` interacts with the merge CHECK below: deleting a merge target leaves the
-- source with status='merged' and merged_into=NULL, which that CHECK refuses. That
-- is deliberate -- a merged node with nothing to point at is not a merged node, and
-- silently promoting it back to 'active' would resurrect a name a curator retired.
ALTER TABLE taxonomy_nodes
    ADD COLUMN merged_into TEXT REFERENCES taxonomy_nodes(id) ON DELETE SET NULL;

ALTER TABLE taxonomy_nodes
    ADD CONSTRAINT taxonomy_nodes_merge_ck
    CHECK ((status = 'merged') = (merged_into IS NOT NULL));

CREATE INDEX taxonomy_nodes_status_idx ON taxonomy_nodes (status);
CREATE INDEX taxonomy_nodes_merged_into_idx ON taxonomy_nodes (merged_into);

-- Fandom scoping, many-to-many so a crossover belongs to several fandoms and a
-- shared-universe character to all of them.
CREATE TABLE taxonomy_node_scope (
    node_id        TEXT NOT NULL REFERENCES taxonomy_nodes(id) ON DELETE CASCADE,
    fandom_node_id TEXT NOT NULL REFERENCES taxonomy_nodes(id) ON DELETE CASCADE,
    created_at     TEXT NOT NULL,
    PRIMARY KEY (node_id, fandom_node_id),
    -- A node scoped to itself is a mistake, and it would make every scoped
    -- expansion include the node from its own scope row.
    CONSTRAINT taxonomy_node_scope_not_self_ck CHECK (node_id <> fandom_node_id)
);
-- "Everything in the Buffy fandom" is the query, and it must not scan the table.
CREATE INDEX taxonomy_node_scope_fandom_idx ON taxonomy_node_scope (fandom_node_id);

-- Graph edges. `rel` is CHECKed because an unrecognised relation would be stored
-- and then ignored by the expansion query -- the silent-wrong-answer failure the
-- plan warns about for bad curator edges.
CREATE TABLE taxonomy_edges (
    src_id     TEXT NOT NULL REFERENCES taxonomy_nodes(id) ON DELETE CASCADE,
    dst_id     TEXT NOT NULL REFERENCES taxonomy_nodes(id) ON DELETE CASCADE,
    rel        TEXT NOT NULL,
    created_by TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY (src_id, dst_id, rel),
    CONSTRAINT taxonomy_edges_not_self_ck CHECK (src_id <> dst_id),
    CONSTRAINT taxonomy_edges_rel_ck
        CHECK (rel IN ('parent', 'implies', 'related', 'adjacent_fandom'))
);
-- Expansion walks outward from a node, so the index leads with src_id. The primary
-- key already covers the exact-match case; this serves "every edge from here", which
-- is what expansion actually asks.
CREATE INDEX taxonomy_edges_src_idx ON taxonomy_edges (src_id);
-- `implies` is off by default in search, so the toggle reads only these rows. A
-- partial index would be smaller, but the SQLite half cannot express the same one,
-- and this table is small enough that the honest choice is a full index.
CREATE INDEX taxonomy_edges_rel_idx ON taxonomy_edges (rel);

-- Materialised closure, so `tag:"Fake Dating"+children` is a join rather than a
-- recursive CTE or a huge IN list. A recursive CTE differs in shape and cost between
-- the two engines; this is one ordinary join that behaves identically on both.
--
-- depth is the SHORTEST path, so a node reachable by two routes keeps the smaller
-- one and expansion never walks a longer path than it must. A node is its own
-- ancestor at depth 0, without which `+children` would exclude the node itself.
CREATE TABLE taxonomy_closure (
    ancestor_id   TEXT NOT NULL REFERENCES taxonomy_nodes(id) ON DELETE CASCADE,
    descendant_id TEXT NOT NULL REFERENCES taxonomy_nodes(id) ON DELETE CASCADE,
    rel           TEXT NOT NULL,
    depth         INTEGER NOT NULL,
    created_at    TEXT NOT NULL,
    PRIMARY KEY (ancestor_id, descendant_id, rel),
    CONSTRAINT taxonomy_closure_depth_ck CHECK (depth >= 0)
);
-- Leads with ancestor_id because expansion filters by ancestor and reads depth,
-- rather than following the primary key's column order.
CREATE INDEX taxonomy_closure_ancestor_idx ON taxonomy_closure (ancestor_id, rel);
-- "What is this tag a child of?" is the tag browser's tree view.
CREATE INDEX taxonomy_closure_descendant_idx ON taxonomy_closure (descendant_id);
