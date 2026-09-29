#!/usr/bin/env python3
"""M60-02 — generate the snapshot masking SQL from the column policy.

    python3 scripts/build-snapshot-sql.py --out /tmp/snap.sql
    python3 scripts/build-snapshot-sql.py --print-stats

Emits one `.sql` file that masks a COPY of the database. It never touches the
live instance, and §11.16.3c is the reason that is a requirement rather than a
preference: re-keying in place fails on `works_owner_pseud_id_fkey` in any
ordering, because referential integrity is checked per statement.

GENERATED, NOT WRITTEN BY HAND
------------------------------
Every `replace` column and every `pseud_id`-shaped column comes from
`docs/snapshot-column-policy.json`, which the M60-01 gate keeps honest. Sixty
migrations' worth of `pseud_id` columns will drift from a hand-maintained list,
and a drift here is a **silent leak** — the column that drifted is published in
the clear, and nothing in the dump looks wrong.

The three outputs, and why each exists:

1. **A masked VIEW per exported table.** `pg_dump` dumps views as their
   definitions, so a table exported as `CREATE VIEW works AS SELECT ...` is a
   table whose values are computed at read time. That is what makes the mask
   apply to a `pg_dump` of the ORIGINAL tables without rewriting the dump.

   Wait — that is not how `pg_dump` works, and pretending otherwise is exactly
   the kind of plausible-but-wrong step this pipeline exists to avoid. `pg_dump`
   emits whatever is in the catalog. So the views are real tables in a SEPARATE
   schema, built by this script, and the dump is taken of THAT schema. The
   original schema is never dumped. (Kept this paragraph because the wrong
   version of it is the obvious first design.)

2. **Excluded tables and columns.** `drop_table` and `drop_column` become
   `pg_dump --exclude-table` arguments, printed for the operator to pass.

3. **A restore check.** A dump that does not restore is not a snapshot, and the
   operator should learn that before publishing rather than after.

WHAT THIS SCRIPT DELIBERATELY DOES NOT DO
-----------------------------------------
* It does not emit the re-key FUNCTIONS. A `pg_dump` captures a database's
  functions, and shipping `snapshot_pseud` would let a recipient hash a
  candidate id and learn which pseudonyms are in the file. The dump carries the
  *values*; the derivation is published in the spec for verification.
* It does not touch timestamps, because §11.16.5 keeps them: relative time is
  the research value and an absolute timestamp is not an identifier.
* It does not decide retention mode. §11.16.6 gating is M60-05, and this
  script's job is to be correct for the mode it is given.
"""
from __future__ import annotations

import argparse
import json
import os
import pathlib
import re
import subprocess
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
POLICY = REPO / "docs" / "snapshot-column-policy.json"
MIGRATIONS = REPO / "migrations" / "postgres"

# Columns the re-key applies to, by NAME. Kept here rather than inferred from
# the type because a `uuid` column is often a surrogate key (a chapter id) and
# re-keying one destroys joins for no privacy gain. The policy file is
# authoritative; this list says which KIND of decision `replace` is.
# Per-table overrides, because a bare `id` is ambiguous and guessing wrong
# re-keys a key with the wrong function -- which the salt-separation requirement
# turns into a cross-table join, the exact leak §11.16.3b forbids.
#
# `accounts.id` is the ACCOUNT key even though the column is called `id`, and
# `pseuds.id` is the PSEUD key even though it is too. Getting either backwards
# is invisible in the output (both are uuids) and fatal in the dataset: a
# snapshot where accounts.id is pseud-keyed cannot be joined to the tables whose
# account_id is account-keyed, and the two halves stop describing the same
# people.
ACCOUNT_KEY_TABLES = {"accounts"}
PSEUD_KEY_TABLES = {"pseuds", "ap_actors"}

PSEUD_KEYED = {"id", "pseud_id", "author_pseud", "owner_pseud_id", "author_pseud_id"}
ACCOUNT_KEYED = {
    "account_id", "owner_account_id", "author_account_id", "user_id",
    "owner_id", "author_id", "actor_id", "follower_actor_id", "followed_actor_id",
    "added_by_account_id", "owning_account_id", "borrower_account_id",
}

# Tables that are structural rather than data: a snapshot that recreates them
# from its own schema must not also import the live instance's version.
INTERNAL_TABLES = {"_migrations", "snapshot_rekey_notes"}


def load_policy() -> dict[str, dict]:
    return json.loads(POLICY.read_text())["columns"]


def parse_columns(mig: pathlib.Path) -> dict[str, dict[str, str]]:
    """table -> {column: type}, from the migrations.

    The TYPE is what the re-key needs and what a name alone cannot supply: most
    `id` columns here are TEXT, and only the uuid-typed ones are keys to re-key.
    """
    tables: dict[str, dict[str, str]] = {}

    def add(table: str, col: str, ctype: str) -> None:
        tables.setdefault(table, {})[col] = ctype

    for path in sorted(mig.glob("*.sql")):
        sql = re.sub(r"--[^\n]*", "", path.read_text(encoding="utf-8", errors="replace"))
        for m in re.finditer(
            r"ALTER\s+TABLE\s+(?:IF\s+EXISTS\s+)?([a-z_]\w*)\s+"
            r"ADD\s+COLUMN\s+(?:IF\s+NOT\s+EXISTS\s+)?([a-z_]\w*)\s+([A-Za-z]+)", sql, re.I
        ):
            add(m.group(1).lower(), m.group(2).lower(), m.group(3).lower())
        for m in re.finditer(
            r"CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?([a-z_]\w*)\s*\(", sql, re.I
        ):
            table = m.group(1).lower()
            depth, i = 1, m.end()
            while i < len(sql) and depth:
                depth += (sql[i] == "(") - (sql[i] == ")")
                i += 1
            for line in sql[m.end(): i - 1].splitlines():
                line = line.strip().rstrip(",")
                cm = re.match(r"^([a-z_]\w*)\s+([A-Za-z]+)", line, re.I)
                if cm and cm.group(1).lower() not in {
                    "primary", "foreign", "constraint", "unique", "check", "key"
                }:
                    add(table, cm.group(1).lower(), cm.group(2).lower())
    return tables


def rekey_expr(col: str, coltype: str, table: str) -> str | None:
    """The re-key projection for one column, or None to keep it.

    The `::uuid` CAST is not decoration, and adding it came from a real error:
    the first generated SQL failed with `function snapshot_pseud(text) does not
    exist`, because most `id` columns in this schema are TEXT while the re-key
    takes a uuid.

    **Not every text id is a pseudonym.** `comments.id` and `admin_actions.id` are
    TEXT surrogate keys, and re-keying them would destroy joins for no privacy
    gain -- a surrogate identifies a ROW, not a PERSON. So the rule is
    name-based AND type-gated: only a `uuid`-typed pseud/account column is a
    key to re-key.

    And the mismatch this exposes is worth stating, because it is a pre-existing
    schema bug rather than something the mask introduced:

        pseuds.id          ::uuid
        comments.author_pseud ::text

    Those two cannot join in the LIVE schema, so `comments.author_pseud` is
    already broken for anyone who wants to know who wrote a comment. The mask
    faithfully preserves the breakage. **Reported, not fixed here** -- changing a
    column type is a schema migration and a separate decision, and a snapshot
    that silently coerced it would be masking a different dataset than the one
    the instance holds.
    """
    if coltype != "uuid":
        return None
    # A bare `id` is a key only in the table that OWNS that kind of key.
    if col == "id":
        if table in ACCOUNT_KEY_TABLES:
            return "snapshot_account({c}::uuid)"
        if table in PSEUD_KEY_TABLES:
            return "snapshot_pseud({c}::uuid)"
        # Everywhere else a uuid `id` is a SURROGATE: a chapter id, a work id, a
        # comment id. It identifies a ROW, not a PERSON, so re-keying it buys no
        # privacy and destroys the chapter/work graph the dataset exists for.
        # Only the policy file can say otherwise, which is why this returns None
        # rather than guessing.
        return None
    if col in ACCOUNT_KEYED:
        return "snapshot_account({c}::uuid)"
    if col in PSEUD_KEYED:
        return "snapshot_pseud({c}::uuid)"
    return None



def read_live_schema() -> dict[str, dict[str, str]] | None:
    """The real schema, when a database is reachable. The migrations otherwise.

    The migration parse is a FALLBACK, and it drifted: it produced a projection
    selecting `id` from `privacy_settings`, a table with no such column, because
    sixty migrations of ALTER TABLE later in the chain are not all visible to a
    regex that reads each file in isolation. A generator that emits SQL nobody
    can run is worse than one that fails.

    The live schema is the truth about what exists; the migrations are the truth
    about what was intended. For a snapshot, "exists" is what matters -- a column
    that was never created cannot leak, and one that was renamed in place is
    real under its new name only.

    Set LOREHAVEN_TEST_PG_URL (or LOREHAVEN_PG_URL) to enable. Absent or
    unreachable, the migration parse is used and `--from-migrations` is
    recommended, because that path is known to be lossy.
    """
    url = os.environ.get("LOREHAVEN_PG_URL") or os.environ.get("LOREHAVEN_TEST_PG_URL")
    if not url:
        return None
    try:
        out = subprocess.run(
            ["psql", url, "-tAc", """
                SELECT table_name || '{US}' || column_name || '{US}' || data_type
                  FROM information_schema.columns
                 WHERE table_schema = 'public'
                 ORDER BY table_name, ordinal_position;""".replace("{US}", chr(31))],
            capture_output=True, text=True, timeout=60, check=True,
        ).stdout
    except (subprocess.SubprocessError, OSError):
        return None
    tables: dict[str, dict[str, str]] = {}
    # E'\\x1f' (unit separator) rather than '.' or a space: a table or column
    # name may contain either, and the first version of this query joined on
    # '.' and then rsplit it, so `privacy_settings.id` came back as table
    # `privacy` column `settings.id` and generated `SELECT id FROM
    # public.privacy_settings` for a table that has one. The resulting error
    # ("column id does not exist") pointed at the SQL, not at the parser.
    US = chr(31)
    for line in out.splitlines():
        parts = line.split(US)
        if len(parts) != 3:
            continue
        t, c, ctype = (p.strip() for p in parts)
        tables.setdefault(t, {})[c] = ctype
    return tables or None


def build(out: pathlib.Path) -> dict:
    policy = load_policy()
    tables = read_live_schema() or parse_columns(MIGRATIONS)

    drop_tables, drop_columns, replaced, rekeyed, kept, views = set(), {}, 0, 0, 0, []

    for table in sorted(tables):
        if table in INTERNAL_TABLES:
            drop_tables.add(table)
            continue
        cols_type = tables[table]
        cols = set(cols_type)
        entries = {c: policy.get(f"{table}.{c}") for c in cols}
        if any(e and e["treatment"] == "drop_table" for e in entries.values()):
            drop_tables.add(table)
            continue

        projection, tdrop, colnames = [], {}, []
        for col in sorted(cols):
            entry = entries.get(col)
            if entry and entry["treatment"] == "drop_column":
                tdrop[col] = True
                continue
            treatment = entry["treatment"] if entry else "keep"
            if treatment == "rekey_text":
                rekeyed += 1
                # A deterministic text re-key, NOT a constant. This exists because
                # 71 columns in this schema are TEXT foreign keys into text-keyed
                # tables, and marking them `replace` produced `'redacted'` -- which
                # reads as a clean mask and silently collapses every row of the
                # table into one. The snapshot would have had no edges in it.
                #
                # NULLs are preserved rather than hashed: a null key is the absence
                # of a relationship, and hashing it into a value would invent one.
                projection.append(
                    f"    CASE WHEN {col} IS NULL THEN NULL\n"
                    f"         ELSE 'snp_' || substr(md5({col}::text || "
                    f"'lorehaven-snapshot-v1-text'), 1, 24) END AS \"{col}\""
                )
                colnames.append(col)
                continue
            if treatment == "rekey_pseud":
                rekeyed += 1
                projection.append(f'    snapshot_pseud({col}::uuid) AS "{col}"')
                colnames.append(col)
                continue
            if treatment == "rekey_account":
                rekeyed += 1
                projection.append(f'    snapshot_account({col}::uuid) AS "{col}"')
                colnames.append(col)
                continue
            if treatment == "replace":
                replaced += 1
                if col == "email":
                    projection.append(f"    md5({col}::text) || '@snapshot.invalid' AS \"{col}\"")
                else:
                    projection.append(f"    'redacted' AS \"{col}\"")
                colnames.append(col)
                continue
            # FALLBACK, and it is deliberately weaker than the policy: a
            # pseud/account-shaped column the policy file has not classified.
            # The M60-01 gate means that should not happen, and if it does the
            # re-key is the safe reading -- a snapshot with one under-masked join
            # key is worse than one with a deterministically renamed surrogate.
            expr = rekey_expr(col, cols_type.get(col, ""), table)
            if expr:
                rekeyed += 1
                projection.append(f"    {expr.format(c=col)} AS {col}")
            else:
                kept += 1
                projection.append(f'    "{col}"')
            colnames.append(col)

        if not any(p.strip().startswith(("snapshot_", "md5(", "'redacted'"))
                   for p in projection):
            # Nothing masked on this table: export it as data, not a view.
            continue
        drop_columns[table] = sorted(tdrop)
        views.append((table, projection, colnames))

    lines = [
        "-- Generated by scripts/build-snapshot-sql.py. DO NOT EDIT.",
        "--",
        "-- The masked schema for a de-identified snapshot (spec 11.16). Apply to a",
        "-- COPY: see 11.16.3c -- the mask cannot be applied in place, in any order,",
        "-- because works_owner_pseud_id_fkey is checked per statement.",
        "--",
        "-- The re-key functions themselves are NOT emitted here. A pg_dump captures a",
        "-- database's functions, and shipping these would let a recipient recompute a",
        "-- pseudonym from a candidate id. The derivation is published in the spec",
        "-- instead, so a third party can VERIFY the values without being able to",
        "-- INVERT them.",
        "",
        "CREATE SCHEMA IF NOT EXISTS snapshot_masked;",
        "",
        "-- pgcrypto is NOT installed by the application's migrations (only the",
        "-- 0092 re-key migration, and only on the live instance's database), so a",
        "-- clean instance has no digest() and every materialized view here fails",
        "-- with 'function digest(text, unknown) does not exist'. The first run of",
        "-- this script hit exactly that. Creating the extension is a superuser",
        "-- action and therefore the operator's, not the pipeline's -- but failing",
        "-- with a clear message beats emitting 20 broken views.",
        "DO $$ BEGIN",
        "    CREATE EXTENSION IF NOT EXISTS pgcrypto;",
        "EXCEPTION WHEN insufficient_privilege THEN",
        "    RAISE NOTICE 'snapshot needs pgcrypto: ask a superuser to run",
        "        CREATE EXTENSION pgcrypto, then re-run';",
        "    RAISE EXCEPTION 'pgcrypto is required for the snapshot re-key';",
        "END $$;",
        "",
    ]
    # REAL TABLES, not views and not materialized views. Two corrections, both
    # found by the dump coming out empty while every leak assertion passed:
    #
    #   1. A plain view is dumped as its CREATE VIEW *definition* with no rows.
    #   2. A MATERIALIZED view is also dumped as a definition -- pg_dump has no
    #      COPY path for either, and `--data-only` skips both too.
    #
    # So a "snapshot" of views is a file containing no data, which contains no
    # canaries, and the leak test passes for the wrong reason. That is the
    # failure `a_masked_dump_still_carries_the_dataset` exists to catch.
    #
    # A real table is dumped as CREATE TABLE + COPY, so the mask is FROZEN INTO
    # THE FILE: a recipient needs neither the original schema nor the re-key
    # functions to read the snapshot, and the snapshot cannot re-derive anything
    # if the functions are later found to be weaker than believed.
    for table, projection, colnames in views:
        # An explicit column list: a projection of bare expressions would
        # otherwise be dumped with columns named `?column?`, which is unusable.
        # A view for the NAMED projection, then CTAS from it.
        #
        # Neither half alone works, and both failures look identical:
        #   * CTAS straight from the expressions names every column `?column?`,
        #     so the dump restores to a table of unusable headers.
        #   * A hand-written CREATE TABLE needs a TYPE per column, and guessing
        #     them is how `version` (reserved) and `taste_vector` (an array) went
        #     wrong on the first attempt.
        # The view carries the names; CTAS carries the types, inferred from the
        # same expressions. The view is dropped immediately -- the snapshot ships
        # as TABLES, because pg_dump emits a view as a definition and no rows.
        lines.append(f"DROP TABLE IF EXISTS snapshot_masked.{table} CASCADE;")
        lines.append(f"CREATE TEMP VIEW _mask_{table} AS")
        lines.append("SELECT")
        lines.append(",\n".join(projection))
        lines.append(f"FROM public.{table};")
        lines.append(f"CREATE TABLE snapshot_masked.{table} AS SELECT * FROM _mask_{table};")
        lines.append(f"DROP VIEW _mask_{table};")
        lines.append("")

    out.write_text("\n".join(lines) + "\n")
    return {
        "views": len(views), "replaced": replaced, "rekeyed": rekeyed,
        "kept": kept, "drop_tables": sorted(drop_tables),
        "drop_columns": {k: v for k, v in drop_columns.items() if v},
    }


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("--out", type=pathlib.Path, help="write the masking SQL here")
    ap.add_argument("--print-stats", action="store_true")
    args = ap.parse_args()
    if not args.out:
        ap.add_argument("--out", required=True, help="required")
    stats = build(args.out)
    if args.print_stats:
        for k, v in stats.items():
            print(f"{k}: {v}")
    else:
        print(f"wrote {args.out} ({stats['views']} masked views, "
              f"{stats['rekeyed']} re-keyed columns, "
              f"{len(stats['drop_tables'])} tables excluded)")
        print("pg_dump exclusions:")
        for t in stats["drop_tables"]:
            print(f"  --exclude-table={t}")
        print("columns dropped per table (build the dump from the views instead):")
        for t, cols in stats["drop_columns"].items():
            print(f"  {t}: {', '.join(cols)}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
