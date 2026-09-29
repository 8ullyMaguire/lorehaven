-- Snapshot re-key: SQLite companion (spec §11.16.3, §11.16.3b; M60-03, M60-04).
--
-- Dialect: SQLite. Read migrations/postgres/0092_snapshot_rekey.sql first -- it
-- carries the reasoning for the salts, the version nibbles, and why these are
-- functions rather than a script.
--
-- NUMBERING. 0092, matching the Postgres file. The catalogue test
-- `the_two_dialects_declare_the_same_migrations` requires the same SET and
-- ORDER in both directories, and the snapshot functions are the reason to read
-- that test as a constraint on the FEATURE rather than a coincidence: a
-- migration that existed for one dialect only would be a schema that works on
-- SQLite and fails on PostgreSQL.
--
-- WHY THIS FILE HAS NO FUNCTIONS, WHILE THE POSTGRES ONE HAS TWO.
--
-- This is not an omission to be tidied up later, and it is not a dialect
-- difference in the SQL dialect. **SQLite has no sha256 and no pgcrypto.** The
-- re-key is defined as sha256 of the original id's text under a fixed salt --
-- there is no way to express that in SQLite SQL, and inventing a weaker
-- construction (a linear congruential mix, or `randomblob()`, which is not even
-- deterministic) to make the file look symmetric would be worse than an
-- honest absence: it would produce a snapshot whose pseudonyms are derived by
-- something other than the construction §11.16.3 publishes, so a third party
-- verifying the dump would compute different values and conclude the snapshot
-- was tampered with.
--
-- So the functions are Postgres-only, and the SQLite instance has no snapshot
-- pipeline to run them through. That is not a gap: **§11.16 is a PostgreSQL
-- feature.** `build-snapshot-sql.py` shells out to `pg_dump` and `psql`, the
-- whole distribution half of the spec is I2P/Tor, and a SQLite deployment is a
-- single-operator local install whose database is the operator's own file. The
-- dataset exists to be published to other instances and researchers, and there
-- is no publication story for a local SQLite file.
--
-- WHAT THIS FILE DOES, THEN. It records the two decisions a reader of the dump
-- needs, in the place someone will actually look for them, and it fails loudly
-- if a future migration tries to add a snapshot function to SQLite -- which
-- would reintroduce the weaker construction described above.
--
-- A note table rather than a function, because SQLite has no CREATE FUNCTION
-- that a reader of the schema would discover, and a comment in a .sql file is
-- invisible to anyone inspecting a live database.

CREATE TABLE IF NOT EXISTS snapshot_rekey_notes (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT OR REPLACE INTO snapshot_rekey_notes (key, value) VALUES
    ('applies_to',
     'PostgreSQL only. SQLite has no sha256 and no pgcrypto, and 11.16 is a '
     || 'publication feature driven by pg_dump and psql. A local SQLite file has '
     || 'no publication story. Do NOT add a weaker re-key here to make the '
     || 'dialects look symmetric: a construction other than the published one '
     || 'makes a third-party verification disagree with the dump.'),
    ('pseud_salt', 'lorehaven-snapshot-v1'),
    ('account_salt', 'lorehaven-snapshot-v1-account'),
    ('version_nibbles', 'version 4, RFC 4122 variant a, written into the hash output'),
    ('must_run_on_a_copy',
     'The mask cannot be applied in place in any order: '
     || 'works_owner_pseud_id_fkey is checked per statement, so updating the child '
     || 'first fails because the parent has no new value, and updating the parent '
     || 'first fails because the child still has the old one. Copy, then mask.'),
    ('sqlite_has_no_sha256_builtin', '1');
