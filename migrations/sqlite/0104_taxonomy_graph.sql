-- M46-03 step 1: the taxonomy graph the advanced-search plan builds on.
--
-- §15.17's taxonomy is a flat list of names today. The plan
-- (`docs/plans/advanced-search-as-built.md` §2, "Taxonomy") needs three things it
-- does not have:
--
--   1. **Scoping.** "Spike (Buffy)" and "Spike (Cowboy Bebop)" must coexist, or a
--      character name collides across fandoms and the omnibox has nothing to
--      disambiguate with. Many-to-many, because a crossover belongs to several.
--   2. **Edges.** parent / implies / related / adjacent_fandom, with a cycle check
--      on write. `implies` is off by default in search precisely because a bad
--      curator edge would otherwise silently distort every query.
--   3. **A materialised closure**, so `tag:"Fake Dating"+children` is a join
--      against `taxonomy_closure` rather than a recursive CTE or a huge `IN` list.
--      A recursive CTE is dialect-specific in shape and cost; the closure is one
--      ordinary join that behaves identically on both engines.
--
-- ── A correction to the plan, and why ────────────────────────────────────────
--
-- The plan's DDL reads:
--
--     ALTER taxonomy_nodes ADD status TEXT NOT NULL DEFAULT 'active';
--     ALTER taxonomy_nodes ADD merged_into INTEGER NULL;
--
-- `merged_into INTEGER` is wrong for this schema. `taxonomy_nodes.id` is TEXT on
-- **both** dialects (verified: `migrations/{sqlite,postgres}/0011_taxonomy.sql`),
-- so an INTEGER foreign key cannot reference a node. On PostgreSQL that is a hard
-- type error, not a coercion: `integer` -> `text` has no implicit cast in a
-- foreign key. It is TEXT here, and the self-reference is a real FK so a merge
-- cannot point at a node that does not exist.
--
-- `status` also has to coexist with the `review_status` that 0082 already added,
-- and they are not the same axis:
--
--   * `review_status` is 'unverified' | 'curated' — §15.17's *curation* state.
--   * `status` is 'active' | 'pending' | 'merged' | 'deprecated' — the node's
--     *lifecycle*.
--
-- Overloading one column for both would make "curated but merged" and "unverified
-- but active" inexpressible, and would break 0082's own rule that the two tables
-- describing the review state spell it the same way. So `status` is separate, with
-- `merged_into` the only way `status = 'merged'` is expressed.
--
-- `-- kind widened` is a comment in the plan, not a DDL statement, and no CHECK
-- constraint exists on `kind` today. Adding one now would reject names already in
-- the table, so the kind list is recorded as a comment on the new tables instead
-- and left unenforced until there is a migration that can backfill.

ALTER TABLE taxonomy_nodes ADD COLUMN status TEXT NOT NULL DEFAULT 'active';

-- CHECK rather than trusting every writer: `status` decides whether a name is
-- offered in the tag browser, so an unrecognised value would silently mean
-- "somewhere in between". SQLite and PostgreSQL both enforce CHECK on ADD COLUMN.
-- NOTE ON ENFORCEMENT. The two CHECKs below are written as triggers, not as
-- `ALTER TABLE ... ADD CONSTRAINT`, because the SQLite the application links
-- cannot parse that form. Measured, not assumed:
--
--   * `libsqlite3-sys 0.30.1` bundles SQLite **3.46.0**; the system `sqlite3` CLI
--     here is **3.53.4**. The CLI accepts `ADD CONSTRAINT`, the app's SQLite does
--     not -- `near "CONSTRAINT": syntax error`. A probe that ran only the CLI would
--     have passed and shipped a migration that fails on the default engine.
--   * `ADD CONSTRAINT` on ALTER TABLE arrived in SQLite 3.50.0. The bundled 3.46.0
--     predates it.
--
-- A 12-step table rebuild is the alternative, and 0103 declined it for the same
-- reason recorded there: rebuilding `taxonomy_nodes` recreates a table that other
-- migrations, `search_nodes`, and every `work_tags` row depend on, to add a
-- constraint the store also validates. Triggers are the established idiom in this
-- repo for exactly this shape (see `migrations/sqlite/0103_character_relationships.sql`),
-- and the PostgreSQL half of this migration keeps the real CHECKs, so the stronger
-- gate exists wherever SQLite is not in play.
--
-- Both INSERT and UPDATE are covered: a CHECK constrains every write, and a
-- BEFORE-INSERT-only trigger would let any later UPDATE set a bad value.

CREATE TRIGGER taxonomy_nodes_status_ck_insert
    BEFORE INSERT ON taxonomy_nodes
    FOR EACH ROW
    WHEN NEW.status NOT IN ('active', 'pending', 'merged', 'deprecated')
    BEGIN
        SELECT RAISE(ABORT, 'taxonomy_nodes.status must be active|pending|merged|deprecated');
    END;

CREATE TRIGGER taxonomy_nodes_status_ck_update
    BEFORE UPDATE ON taxonomy_nodes
    FOR EACH ROW
    WHEN NEW.status NOT IN ('active', 'pending', 'merged', 'deprecated')
    BEGIN
        SELECT RAISE(ABORT, 'taxonomy_nodes.status must be active|pending|merged|deprecated');
    END;

CREATE TRIGGER taxonomy_nodes_merge_ck_insert
    BEFORE INSERT ON taxonomy_nodes
    FOR EACH ROW
    WHEN (NEW.status = 'merged') <> (NEW.merged_into IS NOT NULL)
    BEGIN
        SELECT RAISE(ABORT,
            'taxonomy_nodes: status=''merged'' and merged_into must be set together');
    END;

CREATE TRIGGER taxonomy_nodes_merge_ck_update
    BEFORE UPDATE ON taxonomy_nodes
    FOR EACH ROW
    WHEN (NEW.status = 'merged') <> (NEW.merged_into IS NOT NULL)
    BEGIN
        SELECT RAISE(ABORT,
            'taxonomy_nodes: status=''merged'' and merged_into must be set together');
    END;

-- NULL for every status except 'merged', and a self-reference: a node merged into
-- itself is a cycle of length one and would make the closure computation loop.
ALTER TABLE taxonomy_nodes
    ADD COLUMN merged_into TEXT REFERENCES taxonomy_nodes(id) ON DELETE SET NULL;

-- A merged node must say what it merged into; anything else may not. Expressed as
-- a CHECK rather than a trigger so it is visible in the schema and enforced by
-- both engines identically.
-- Enforced by trigger for the reason given above. This one is the load-bearing
-- pair: without it a node can claim to be merged while pointing nowhere, which is
-- precisely the state a merge is supposed to resolve.

-- A merge chain must terminate. `ON DELETE SET NULL` on the reference above can
-- leave a node whose target was deleted, leaving `status = 'merged'` with nothing
-- to point at; the CHECK above catches that too, since merged_into becomes NULL.
CREATE INDEX taxonomy_nodes_status_idx ON taxonomy_nodes (status);
CREATE INDEX taxonomy_nodes_merged_into_idx ON taxonomy_nodes (merged_into);

-- Fandom scoping. Many-to-many on purpose: a crossover belongs to several fandoms,
-- and a shared-universe character to all of them. A single `fandom_id` column on
-- the node would force exactly the duplication this table exists to remove.
--
-- No `kind` check on the fandom side. `taxonomy_nodes.kind` carries no CHECK today,
-- and adding one here would be an unenforced pretence; the store validates the
-- kinds on write instead, where it can name them in the error.
CREATE TABLE taxonomy_node_scope (
    node_id       TEXT NOT NULL REFERENCES taxonomy_nodes(id) ON DELETE CASCADE,
    fandom_node_id TEXT NOT NULL REFERENCES taxonomy_nodes(id) ON DELETE CASCADE,
    created_at    TEXT NOT NULL,
    -- A node scoped to itself is not a crossover, it is a mistake, and it makes
    -- every scoped expansion include the node from its own scope row.
    PRIMARY KEY (node_id, fandom_node_id),
    CHECK (node_id <> fandom_node_id)
);
-- The reverse direction is the one queries use: "everything in the Buffy fandom",
-- which must not be a full scan of the table.
CREATE INDEX taxonomy_node_scope_fandom_idx ON taxonomy_node_scope (fandom_node_id);

-- Graph edges. `rel` is CHECKed rather than left free: an unrecognised relation
-- would be an edge the expansion query cannot interpret, so it would be stored and
-- then ignored, which is the silent-wrong-answer failure the plan warns about for
-- bad curator edges.
CREATE TABLE taxonomy_edges (
    src_id     TEXT NOT NULL REFERENCES taxonomy_nodes(id) ON DELETE CASCADE,
    dst_id     TEXT NOT NULL REFERENCES taxonomy_nodes(id) ON DELETE CASCADE,
    rel        TEXT NOT NULL,
    created_by TEXT,
    created_at TEXT NOT NULL,
    -- Two nodes cannot be joined to themselves: `parent` self-loop makes the
    -- closure computation non-terminating, and the other relations are
    -- meaningless for a node against itself.
    PRIMARY KEY (src_id, dst_id, rel),
    CHECK (src_id <> dst_id),
    CHECK (rel IN ('parent', 'implies', 'related', 'adjacent_fandom'))
);
-- Expansion walks *outward* from a node, so the index leads with src_id. The
-- primary key already covers (src_id, dst_id, rel) for an exact match; this one is
-- for "every edge of any relation from this node", which is the query expansion
-- actually issues and which a leading-src index serves.
CREATE INDEX taxonomy_edges_src_idx ON taxonomy_edges (src_id);
-- `implies` is off by default in search, so the toggle reads only these rows. A
-- partial index would be smaller, but partial indexes are not portable to SQLite
-- in the same form, and this table is small enough that a full index on the
-- relation is the honest choice.
CREATE INDEX taxonomy_edges_rel_idx ON taxonomy_edges (rel);

-- Materialised closure. `ancestor_id` is the node expansion started from and
-- `descendant_id` one reached; `depth` is the shortest path, so a node reachable
-- by two routes keeps the smaller one and expansion never walks a longer path than
-- it has to.
--
-- A node is its own ancestor at depth 0. Without that self-row, `tag:X+children`
-- would exclude X itself, which is the opposite of what the operator asked for.
CREATE TABLE taxonomy_closure (
    ancestor_id   TEXT NOT NULL REFERENCES taxonomy_nodes(id) ON DELETE CASCADE,
    descendant_id TEXT NOT NULL REFERENCES taxonomy_nodes(id) ON DELETE CASCADE,
    rel           TEXT NOT NULL,
    depth         INTEGER NOT NULL,
    created_at    TEXT NOT NULL,
    PRIMARY KEY (ancestor_id, descendant_id, rel),
    CHECK (depth >= 0)
);
-- The plan's expansion query filters by ancestor and reads depth, so the index
-- leads with ancestor_id rather than following the primary key's column order.
CREATE INDEX taxonomy_closure_ancestor_idx ON taxonomy_closure (ancestor_id, rel);
-- The reverse question — "what is this tag a child of?" — is asked by the tag
-- browser's tree view, so it gets its own index rather than a scan.
CREATE INDEX taxonomy_closure_descendant_idx ON taxonomy_closure (descendant_id);
