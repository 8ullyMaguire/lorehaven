-- Migration 0011: structured taxonomy, body search index, mood taxonomy
-- Both dialects: identical column sets; types follow ADR 0004.

CREATE TABLE taxonomy_nodes (
    id TEXT PRIMARY KEY,
    kind TEXT NOT NULL,
    canonical TEXT NOT NULL,
    norm TEXT NOT NULL,
    created_at TEXT NOT NULL
);
CREATE UNIQUE INDEX taxonomy_nodes_kind_norm ON taxonomy_nodes (kind, norm);
CREATE INDEX taxonomy_nodes_norm ON taxonomy_nodes (norm);

CREATE TABLE taxonomy_aliases (
    alias TEXT NOT NULL,
    norm TEXT NOT NULL,
    node_id TEXT NOT NULL,
    source TEXT NOT NULL,
    PRIMARY KEY (alias, node_id)
);
CREATE INDEX taxonomy_aliases_norm ON taxonomy_aliases (norm);
CREATE INDEX taxonomy_aliases_node ON taxonomy_aliases (node_id);

CREATE TABLE work_tags (
    work_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    weight INTEGER NOT NULL DEFAULT 0,
    added_at TEXT NOT NULL,
    PRIMARY KEY (work_id, node_id)
);
CREATE INDEX work_tags_node ON work_tags (node_id);

CREATE TABLE works_index (
    -- The PostgreSQL arm of this same migration declares
    -- `work_id ... REFERENCES works (id) ON DELETE CASCADE` on both index
    -- tables. These were missing here, so a deleted work left its index rows
    -- behind on SQLite and hard-deleting a work cascaded on PostgreSQL only.
    -- The pool runs with `PRAGMA foreign_keys = ON`, so the constraints were
    -- always enforceable. Found by the search-index suite, which had no tests.
    work_id TEXT PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    body_text TEXT NOT NULL
);

CREATE TABLE works_index_terms (
    work_id TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    term TEXT NOT NULL,
    pos INTEGER NOT NULL
);
CREATE INDEX works_index_terms_term ON works_index_terms (term, work_id);
CREATE INDEX works_index_terms_work_pos ON works_index_terms (work_id, pos);

CREATE TABLE moods (
    node_id TEXT PRIMARY KEY,
    axes TEXT NOT NULL
);

CREATE TABLE work_moods (
    work_id TEXT NOT NULL,
    node_id TEXT NOT NULL,
    score INTEGER NOT NULL,
    PRIMARY KEY (work_id, node_id)
);
CREATE INDEX work_moods_node ON work_moods (node_id);
