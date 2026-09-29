-- Snapshot re-key functions (spec §11.16.3, §11.16.3b; M60-03, M60-04).
--
-- Dialect: PostgreSQL.
--
-- The counterpart of migrations/sqlite/0092_snapshot_rekey.sql. Read that file
-- first: it carries the reasoning. This file differs only where the platform
-- does -- `digest()` from pgcrypto versus a hand-rolled loop, and `::uuid` for
-- the cast.
--
-- NUMBERING. 0092, following 0091. 0084 is reserved (gap D5), 0086-0088 belong
-- to the M59 retention work, and 0089 is a gap in both dialects.
--
-- WHY FUNCTIONS RATHER THAN A SCRIPT. §11.16.3 requires the re-key to be a
-- deterministic function of the original id ALONE, applied identically in every
-- table that references it. That is a property that is easy to state and easy
-- to violate by accident: a masking script that computes a replacement per row
-- will produce a set of unrelated tables, and the dataset is worthless. Making
-- it a database function makes the property structural -- every table calls the
-- same function, so the joins hold by construction.
--
-- WHY pgcrypto, AND WHY THESE EXACT CONSTRUCTIONS. `postgresql_anonymizer` has
-- CVEs, needs superuser, and cannot use `anon.random_id()` in backup masking
-- masks. pgcrypto is in the stock image. The re-key is sha256 over the
-- original uuid's TEXT with a fixed published salt, then forced into a valid
-- version-4 UUID shape by writing the version and variant nibbles directly.
-- Forcing the nibbles is what makes the result a real v4 UUID rather than a
-- string that merely parses as one: any tool that keys on UUID version (or
-- rejects a v5-shaped value from `uuid_generate_v5`) keeps working.
--
-- THE SALTS ARE DIFFERENT AND FIXED, AND THAT IS THE POINT. §11.16.3b: if
-- pseud_id and account_id were derived from the same salt, anyone holding one
-- re-keyed table could join them -- a pseud resolves to an account, defeating
-- the pseudonymisation. Two salts, both published, both fixed. Published
-- because a third party must be able to VERIFY the construction rather than
-- take it on trust, and non-secret because anyone holding the real id list could
-- otherwise reverse it.
--
-- These functions live in the DDL rather than only in the masking pipeline
-- because `pg_dump` captures the FUNCTIONS of a database. A dump that carried
-- the re-key would let a recipient reverse any published row: hash the value
-- back, or simply re-apply the function to a candidate id and see whether it
-- matches. build-snapshot-sql.py must therefore emit them, and must NOT emit
-- the published namespace constants as anything a recipient can read -- the
-- constants are in the dump ON PURPOSE, so the recipient can verify
-- determinism, which is the trade the spec makes and which is safe only
-- because the real ids are the thing being protected, not the function.

-- pgcrypto, for `digest()`. The extension is created HERE rather than assumed
-- to exist, and the reason is a bug this migration's first run actually hit:
-- it applied cleanly against a long-lived database where someone had already
-- run CREATE EXTENSION by hand, and then failed on a FRESH scratch database
-- with `function digest(text, unknown) does not exist`. A migration that only
-- works on the database its author happened to test against is a migration that
-- fails on every operator's clean install.
--
-- IF NOT EXISTS because the extension is harmless to keep when already present,
-- and because a snapshot pipeline re-running this SQL should not fail on a
-- database that has it. digest() is a pure function, so this changes no result.
CREATE EXTENSION IF NOT EXISTS pgcrypto;

-- The marker table, so the two dialects declare the same schema. The parity
-- test `the_two_dialects_declare_the_same_columns_and_indexes` caught its
-- absence here first, which is the test doing exactly the job its own comment
-- describes: a table present in one dialect and absent in the other is a schema
-- that works on SQLite and fails on PostgreSQL.
--
-- The VALUES differ per dialect -- SQLite's records that sha256 is unavailable
-- there, PostgreSQL's records that the functions above implement it -- and that
-- asymmetry is the point rather than an oversight. The schema is the contract;
-- the note text is documentation.
CREATE TABLE IF NOT EXISTS snapshot_rekey_notes (
    key   TEXT PRIMARY KEY,
    value TEXT NOT NULL
);

INSERT INTO snapshot_rekey_notes (key, value) VALUES
    ('pseud_salt', 'lorehaven-snapshot-v1'),
    ('account_salt', 'lorehaven-snapshot-v1-account'),
    ('version_nibbles', 'version 4, RFC 4122 variant a, written into the hash output'),
    ('must_run_on_a_copy',
     'The mask cannot be applied in place in any order: '
     || 'works_owner_pseud_id_fkey is checked per statement, so updating the child '
     || 'first fails because the parent has no new value, and updating the parent '
     || 'first fails because the child still has the old one. Copy, then mask.');

CREATE OR REPLACE FUNCTION snapshot_pseud(src uuid)
RETURNS uuid
LANGUAGE plpgsql
IMMUTABLE
PARALLEL SAFE
AS $$
DECLARE
    h text := encode(public.digest(src::text || 'lorehaven-snapshot-v1', 'sha256'::text), 'hex');
BEGIN
    -- plpgsql, NOT LANGUAGE sql. This is a fix, not a style choice: a
    -- `LANGUAGE sql` function is INLINED into its calling query, and inside a
    -- MATERIALIZED VIEW the inlined body's digest(text, 'sha256') loses its
    -- argument types -- the literal resolves as `unknown`, no overload matches,
    -- and the view fails with
    --     function digest(text, unknown) does not exist
    -- Called DIRECTLY the same function works, which is why this survived the
    -- M60-01 tests: they called it, they never put it in a view. Found by
    -- M60-02, whose whole job is to build a dump.
    --
    -- The parameter was also named `raw`, a reserved SQL word. That is not the
    -- bug (the failure is the inlining, not the name) but a reserved word is a
    -- poor name for a parameter in a re-keying function, so it is `src` now.
    --
    -- `public.digest`, SCHEMA-QUALIFIED. plpgsql resolves function names at
    -- parse time under the CALLER's search_path, and a materialized view is
    -- created with a search_path that does not include wherever pgcrypto was
    -- installed. Unqualified, this fails at REFRESH time with
    --     function digest(text, text) does not exist
    -- even though `SELECT digest('a','sha256')` works in the same database.
    -- The 'sha256'::text cast is for the same reason: the bare literal arrives
    -- as `unknown` and no overload matches it.
    --
    -- Salt and digest are UNCHANGED, so a value re-keyed by the old definition
    -- equals the value re-keyed by this one. Verified: new(x) = old(x) -> true.
    RETURN (
        substr(h, 1, 8) || '-' ||
        substr(h, 9, 4) || '-' ||
        '4' || substr(h, 14, 3) || '-' ||   -- version 4
        'a' || substr(h, 18, 3) || '-' ||   -- RFC 4122 variant
        substr(h, 21, 12)
    )::uuid;
END;
$$;

COMMENT ON FUNCTION snapshot_pseud(uuid) IS
  'Snapshot re-key for a pseud_id. Deterministic in the input alone, so every table that references the same pseud gets the same replacement. Published salt: the spec requires a third party to be able to verify the construction. NOT REVERSIBLE, and must not be dumped.';

CREATE OR REPLACE FUNCTION snapshot_account(src uuid)
RETURNS uuid
LANGUAGE plpgsql
IMMUTABLE
PARALLEL SAFE
AS $$
DECLARE
    h text := encode(public.digest(src::text || 'lorehaven-snapshot-v1-account', 'sha256'::text), 'hex');
BEGIN
    -- plpgsql, NOT LANGUAGE sql: a `LANGUAGE sql` function is INLINED into its
    -- caller, and inside a materialized view the inlined digest() call loses its
    -- argument types and fails with 'function digest(text, unknown) does not
    -- exist'. See the note on snapshot_pseud above.
    RETURN (
        substr(h, 1, 8) || '-' ||
        substr(h, 9, 4) || '-' ||
        '4' || substr(h, 14, 3) || '-' ||
        'a' || substr(h, 18, 3) || '-' ||
        substr(h, 21, 12)
    )::uuid;
END;
$$;

COMMENT ON FUNCTION snapshot_account(uuid) IS
  'Snapshot re-key for an account_id, on a SEPARATE salt from snapshot_pseud. Sharing the salt would let anyone holding one re-keyed table join pseud to account. NOT REVERSIBLE, and must not be dumped.';
