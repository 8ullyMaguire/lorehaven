-- 0126: the work-tag assertions carry their provenance (M6-04).
--
-- Spec §6.4: a **tag assertion** is "a work's claim about an entity, with
-- provenance and confidence", and the work_tag it describes carries
-- `source` (author · curator · imported · exchanged · inferred),
-- `confidence` (0..1), `role` (primary · secondary · background) and
-- `prominence`.
--
-- THE ESTATE ALREADY HAD THREE OF THE FOUR COLUMNS, AND THE CARD'S NAMES
-- ARE SWAPPED AGAINST THEM.
--
-- `work_tags` already carried `source` (TEXT, CHECKed to `'author'`), and
-- `confidence` (REAL). And it carried a `prominence` (TEXT) whose CHECK is
-- `('primary', 'secondary', 'background')` -- which is the specification's
-- **`role`**, word for word. So the end state is:
--
--   * `role` (TEXT: primary | secondary | background) -- the old
--     `prominence`, renamed to free the name for the number;
--   * `prominence` (INTEGER) -- the old `weight`, which was always a
--     prominence and was called the other thing;
--   * `source` (TEXT), widened from one value to five;
--   * `confidence` (REAL), unchanged.
--
-- TWO RENAMES AND A CONSTRAINT, AND NO DATA MOVED
--
-- PostgreSQL can rename a column in place, and a CHECK constraint on it
-- follows the rename by itself; only the constraint's *name* needs
-- correcting, because a constraint called `work_tags_prominence_ck` that
-- constrains `role` is a question nobody can answer from the schema.
--
-- THE DEFAULTS STAY THE ESTATE'S
--
-- `role` keeps `DEFAULT 'secondary'` and `source` keeps `DEFAULT 'author'`.
-- The card asks for `'primary'` and `'imported'`. A default must not
-- manufacture a claim -- migration 0099's own note says so, and it is why
-- `confirmation` defaults to `'unconfirmed'` rather than to `'reader'` --
-- and both of the estate's defaults are the modest ones. The store writes
-- `source` on every path, so the value is the store's guarantee rather than
-- the column's default.
--
-- THE DATA IS NOT REWRITTEN
--
-- The renames are a change of vocabulary and not of content: the number
-- keeps its number, the confidence keeps its value, and a row that said
-- `source = 'author'` still says it.
--
-- Dialect: PostgreSQL.

-- The categorical column is the specification's `role`.
ALTER TABLE work_tags RENAME COLUMN prominence TO role;

-- The number was always a prominence and was called the other thing.
ALTER TABLE work_tags RENAME COLUMN weight TO prominence;

-- The constraint that names the old column keeps the old name unless it is
-- told otherwise: a constraint called `work_tags_prominence_ck` that
-- constrains `role` is a name that is now wrong.
ALTER TABLE work_tags RENAME CONSTRAINT work_tags_prominence_ck TO work_tags_role_ck;

-- The provenance's vocabulary, widened from the one value the old CHECK
-- allowed.
--
-- **The widening is not a griefing vector, because there is still no
-- reader-applied value.** ADR 0026 refused a reader *tag layer* so that "a
-- reader tag is never a gravity input" would be an invariant rather than a
-- convention every write path must remember. The five values here are
-- author, curator, imported, exchanged and inferred -- none of them is a
-- reader -- so the invariant holds by the vocabulary itself. A reader's
-- contribution is a *vote*, and a vote feeds `work_tags.confidence` only.
ALTER TABLE work_tags DROP CONSTRAINT work_tags_source_ck;
ALTER TABLE work_tags
    ADD CONSTRAINT work_tags_source_ck
    CHECK (source IN ('author', 'curator', 'imported', 'exchanged', 'inferred'));
