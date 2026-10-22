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
-- THE DATA IS NOT REWRITTEN
--
-- The prominence keeps its number, the confidence keeps its value, and a row
-- that said `source = 'author'` still says it. The two renames are a change
-- of vocabulary and not of content.
--
-- Dialect: PostgreSQL.
--
-- The two renames first, so the CHECK that travels with the renamed column
-- is the role's.
ALTER TABLE work_tags RENAME COLUMN prominence TO role;
ALTER TABLE work_tags RENAME COLUMN weight TO prominence;

-- The CHECK that named the old column keeps the old name, which is a name
-- that is now wrong: a constraint called `work_tags_prominence_ck` that
-- constrains `role` is a question nobody can answer from the schema.
ALTER TABLE work_tags RENAME CONSTRAINT work_tags_prominence_ck TO work_tags_role_ck;

-- The provenance's vocabulary, widened from the one value the old CHECK
-- allowed.
ALTER TABLE work_tags DROP CONSTRAINT work_tags_source_ck;
ALTER TABLE work_tags
    ADD CONSTRAINT work_tags_source_ck
    CHECK (source IN ('author', 'curator', 'imported', 'exchanged', 'inferred'));

-- The indexes the estate already had, and the one the recount needs.
CREATE INDEX IF NOT EXISTS work_tags_node_work ON work_tags (node_id, work_id);
