-- M57 step 2: §15.17's unverified/curated split, on the taxonomy itself.
--
-- Migration 0081 created `canonical_entities` with a `review_status`, and that
-- was enough for the exchange's own `GET /canonical` response. It is not enough
-- for §15.17's actual claim, which is that an entity created from a signal is
-- **usable immediately** — attachable to a work, searchable, visible in the tag
-- browser — while being visibly not-curated.
--
-- "Usable" means the existing taxonomy. `taxonomy_nodes` is what a tag browser
-- reads, what `search_nodes` searches, and what `work_tags` references, so an
-- entity that lives only in `canonical_entities` is a second, parallel
-- vocabulary that nothing else in the instance can see. That is the failure
-- §15.17 names: the taxonomy stalls because new names are invisible.
--
-- So the split belongs on the node itself, not only in the exchange's table.
-- Without `review_status` here, a tag created from a signal either has to be
-- presented as curated — a lie, and §15.17's second acceptance line forbids it —
-- or the tag browser has to know about two tables, which is how a "curated"
-- label ends up meaning "exists in the new table" rather than "was reviewed".
--
-- `ALTER TABLE ... ADD COLUMN` rather than a rebuild: the table already exists
-- on both backends with data, and a rebuild here is exactly the operation the
-- project skill warns about (renaming the table rewrites the foreign keys that
-- reference it, and dropping it then strands `work_tags`).
--
-- The default is `'curated'` and that is deliberate, not an oversight. Every
-- node that exists today was created by a human through the taxonomy UI, and
-- §19.4 quorum work is what makes a node curated. A node created *by the
-- exchange* passes `review_status = 'unverified'` explicitly. Defaulting to
-- `'unverified'` would relabel the entire existing vocabulary as unreviewed the
-- moment this migration ran, which would be both wrong and invisible.

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
