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
-- THE TABLE IS NOT REBUILT, AND THAT IS A DECISION
--
-- Migration 0103 already refused to rebuild this table: *"rebuilding
-- `work_tags` on SQLite means recreating a table other migrations and the
-- store both depend on, for a constraint the store already guarantees."*
-- This migration needs no rebuild either -- SQLite can rename a column
-- (3.25 and later, and the bundled engine is 3.46), and `source`'s CHECK is
-- a **trigger**, which is dropped and recreated. The table is the largest
-- in the schema, and copying it to change a column's name is a cost the
-- rename does not ask for.
--
-- THE DEFAULTS STAY THE ESTATE'S, AND THAT IS A DECISION TOO
--
-- `role` keeps `DEFAULT 'secondary'` and `source` keeps `DEFAULT 'author'`.
-- The card asks for `'primary'` and `'imported'`. A default must not
-- manufacture a claim -- migration 0099's own note says so, and it is why
-- `confirmation` defaults to `'unconfirmed'` rather than to `'reader'` --
-- and both of the estate's defaults are the modest ones: a tag is not
-- central until somebody says it is, and an unlabelled tag is not an
-- authorship claim until somebody says it is. Changing a column's default
-- needs a table rebuild on SQLite, and 0103 refused to rebuild this table
-- for a constraint the store already guarantees; the store now writes
-- `source` on every path, so the guarantee is in the store.
--
-- THE DATA IS NOT REWRITTEN
--
-- The renames are a change of vocabulary and not of content: the number
-- keeps its number, the confidence keeps its value, and a row that said
-- `source = 'author'` still says it.
--
-- Dialect: SQLite.

-- The trigger that constrained the old `prominence` is dropped *before* the
-- rename, so nothing has to be rewritten under it, and the constraint comes
-- back under the column's new name.
DROP TRIGGER work_tags_prominence_ck;

-- The categorical column is the specification's `role`.
ALTER TABLE work_tags RENAME COLUMN prominence TO role;

-- The number was always a prominence and was called the other thing.
ALTER TABLE work_tags RENAME COLUMN weight TO prominence;

CREATE TRIGGER work_tags_role_ck
    BEFORE INSERT ON work_tags
    FOR EACH ROW
    WHEN NEW.role NOT IN ('primary', 'secondary', 'background')
    BEGIN
        SELECT RAISE(ABORT, 'work_tags.role must be primary|secondary|background');
    END;

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
DROP TRIGGER work_tags_source_ck;

CREATE TRIGGER work_tags_source_ck
    BEFORE INSERT ON work_tags
    FOR EACH ROW
    WHEN NEW.source NOT IN ('author', 'curator', 'imported', 'exchanged', 'inferred')
    BEGIN
        SELECT RAISE(ABORT, 'work_tags.source must be author|curator|imported|exchanged|inferred');
    END;
