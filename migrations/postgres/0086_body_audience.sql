-- M59 Phase C1: body audience (spec §7.7).
--
-- Dialect: PostgreSQL.
--
-- The counterpart of migrations/sqlite/0086_body_audience.sql. The declared
-- columns and indexes must match that file exactly, because
-- `the_two_dialects_declare_the_same_columns_and_indexes` in
-- crates/db/src/migrate.rs compares them.
--
-- The SQLite file's reasoning about NULL-versus-'anyone' and about the absent
-- DEFAULT applies identically here and is not repeated; what differs is only
-- the syntax. Two PostgreSQL-specific notes:
--
-- * `ALTER TABLE ... ADD CONSTRAINT ... CHECK` is supported, so the constraint
--   can be added in the same statement as the column. SQLite needs the same
--   shape and gets it; the two agree, which is what the comparison test checks.
--
-- * No index, for the same reason as the SQLite file: nothing queries by
--   audience yet, and an index with no query behind it is write amplification.
--
-- `TEXT` rather than an enum type. A PostgreSQL enum would make the domain
-- crate's `BodyAudience` and the database's type list two declarations that have
-- to be kept in step by hand, and `trust_at_least:4` does not fit an enum
-- anyway. A CHECK constraint is the same guarantee at the point of insertion
-- without the second declaration.

ALTER TABLE works ADD COLUMN body_audience TEXT;

ALTER TABLE works ADD CONSTRAINT works_body_audience_valid
  CHECK (
    body_audience IS NULL
    OR body_audience = 'anyone'
    OR body_audience = 'accounts_only'
    OR body_audience = 'role_operator'
    OR body_audience = 'role_vanguard'
    OR body_audience = 'role_curator'
    OR body_audience = 'trust_at_least'
    OR body_audience LIKE 'trust_at_least:%'
  );
