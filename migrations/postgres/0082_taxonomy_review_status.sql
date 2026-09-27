-- M57 step 2: §15.17's unverified/curated split, on the taxonomy itself.
--
-- PostgreSQL arm of `migrations/sqlite/0082_taxonomy_review_status.sql`.
--
-- `ALTER TABLE ... ADD COLUMN ... NOT NULL DEFAULT` is a metadata-only
-- operation on PostgreSQL 11+, so adding a NOT NULL column to a table with rows
-- does not rewrite the table and does not take an ACCESS EXCLUSIVE lock for
-- longer than a catalog update. That matters here because `taxonomy_nodes` is
-- live on a running instance and this is not a table an operator can afford to
-- rebuild.
--
-- The SQLite arm gets the same statement verbatim, and the project keeps a test
-- that the two dialects declare the same columns — so a divergence in this
-- migration is caught by the parity test rather than by a production error.

ALTER TABLE taxonomy_nodes ADD COLUMN review_status TEXT NOT NULL DEFAULT 'curated';

-- §15.17: an unverified node is never rendered as curated, and a surface
-- showing it says so. This index serves the "show me what needs review" query,
-- which is the one that must not have to scan the whole vocabulary.
CREATE INDEX taxonomy_nodes_review ON taxonomy_nodes (review_status);

-- §15.17: `signal_count` is review priority and nothing else — never a demand
-- weight (§16.16.1), never a ranking input, and never a count of readers. The
-- column is named for what it counts, and the comment says what it is not,
-- because a column called `reader_count` would be read as a headcount by the
-- next person to see it.
--
-- Defaults to 0 because a node created through the taxonomy UI has no signals
-- behind it: its authority came from §19.4 quorum, not from the exchange.
ALTER TABLE taxonomy_nodes ADD COLUMN signal_count INTEGER NOT NULL DEFAULT 0;
