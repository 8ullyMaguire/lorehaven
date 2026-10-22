-- 0126: the work-tag assertions carry their provenance (M6-04).
--
-- Spec §6.4: a **tag assertion** is "a work's claim about an entity, with
-- provenance and confidence".
--
-- THE ESTATE ALREADY HAD THREE OF THE FOUR COLUMNS, AND THE CARD'S NAMES
-- ARE SWAPPED AGAINST THEM.
--
-- `work_tags` already carried `source` (TEXT, CHECKed to `'author'`), and
-- `confidence` (REAL). And it carried a `prominence` (TEXT) whose CHECK is
-- `('primary', 'secondary', 'background')` -- which is the card's **`role`**,
-- word for word. So the card's end state is:
--
--   * `role` (TEXT: primary | secondary | background) -- the estate's old
--     `prominence`, renamed to free the name;
--   * `prominence` (INTEGER) -- the estate's old `weight`, which was always a
--     prominence and was called the other thing;
--   * `source` (TEXT), widened from one value to five;
--   * `confidence` (REAL), unchanged.
--
-- The vocabulary of `source` is `author` · `curator` · `imported` ·
-- `exchanged` · `inferred`. **The distinction is the one that matters to a
-- reader**: a tag the *author* put on their own work is a statement of fact
-- about the work, and a tag the importer guessed from a source's own metadata
-- is a guess. Both are in the table, and a surface that shows a work's tags
-- can say which is which.
--
-- THE TABLE IS REBUILT, AND THAT IS THE EXPENSIVE PART
--
-- `source`'s CHECK is `source = 'author'`, and SQLite cannot widen a CHECK
-- without a rebuild -- the same "a 12-step table rebuild per table" the
-- estate's own notes warn about. So the table is rebuilt, and the rebuild is
-- honest about what it preserves:
--
--   * every row, with its prominence and its confidence;
--   * the `confirmation` state and the `flagged_by`/`flagged_at` pair, which
--     are the reader's own record of whether a tag is right (§49.2);
--   * the `status` column.
--
-- `work_tags` has no foreign keys *pointing at it* and only one of its own
-- (`node_id` -> `taxonomy_nodes`), so the rebuild strands nothing. The
-- indexes are recreated under their own names, and the new ones
-- (`work_tags_node_work`) are created after the rename.
--
-- The data is not rewritten: the prominence keeps its number, the confidence
-- keeps its value, and a row that said `source = 'author'` still says it.
--
-- Dialect: SQLite.
CREATE TABLE work_tags_rebuilt (
    work_id       TEXT NOT NULL,
    node_id       TEXT NOT NULL REFERENCES canonical_entities(id) ON DELETE CASCADE,
    -- The tag's prominence: how much the work leans on it. The old `weight`.
    prominence    INTEGER NOT NULL DEFAULT 0,
    added_at      TEXT NOT NULL,
    -- The reader's own record of whether a tag is right (§49.2).
    confirmation  TEXT NOT NULL DEFAULT 'unconfirmed'
        CHECK (confirmation IN ('unconfirmed', 'reader', 'wrangler', 'inaccurate')),
    flagged_by    TEXT,
    flagged_at    TEXT,
    -- The claim's role in the work's description: primary, secondary or
    -- background. The primary/secondary split is the theme overlap rule
    -- (§2.2): a tag's primary theme is the theme whose seed claims it.
    role          TEXT NOT NULL DEFAULT 'secondary'
        CHECK (role IN ('primary', 'secondary', 'background')),
    -- Where the claim came from.
    source        TEXT NOT NULL DEFAULT 'imported'
        CHECK (source IN ('author', 'curator', 'imported', 'exchanged', 'inferred')),
    -- How sure the claim is. Fed by the votes and by nothing else.
    confidence    REAL NOT NULL DEFAULT 1.0,
    status        TEXT NOT NULL DEFAULT 'applied',
    PRIMARY KEY (work_id, node_id)
);

INSERT INTO work_tags_rebuilt
    (work_id, node_id, prominence, added_at, confirmation, flagged_by, flagged_at,
     role, source, confidence, status)
SELECT work_id, node_id, weight, added_at, confirmation, flagged_by, flagged_at,
       -- The old `prominence` was the role, and its vocabulary is the role's.
       prominence,
       -- The old CHECK allowed only 'author', and 'author' is in the new
       -- vocabulary, so every existing row keeps its source.
       source,
       confidence,
       status
FROM work_tags;

DROP TABLE work_tags;

ALTER TABLE work_tags_rebuilt RENAME TO work_tags;

CREATE INDEX work_tags_node ON work_tags (node_id);
-- The recount reads the work counts by node, so the index has to answer
-- "which nodes carry this many works" without a scan of `work_tags`.
CREATE INDEX work_tags_node_work ON work_tags (node_id, work_id);
