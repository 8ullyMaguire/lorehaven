#!/usr/bin/env python3
"""Report PostgreSQL placeholders bound to uuid/typed columns without a cast.

Most bind parameters in this codebase are `&str`, but the PostgreSQL migrations
declare many id columns as UUID, TEXT, or BIGINT. A bare `$1` against a UUID
column fails at runtime:

    operator does not exist: uuid = text
    column "work_id" is of type uuid but expression is of type text

SQLite has no static types, so the same statement is fine there and the defect
only appears on a PostgreSQL test run. This has bitten five separate call sites
in this repo (all found by a PG test run, none by review).

The script cannot resolve placeholders to columns without a full SQL parser, so
it reports a *file* as suspicious when its PostgreSQL arm contains both a `$N`
placeholder and a uuid-typed column, but no cast anywhere. That is coarse on
purpose: it points at a file to read, it does not claim a specific statement is
wrong. Exit 0 always.

Usage: python3 scripts/check-pg-uuid-placeholder-casts.py [--self-test]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

CREATE_TABLE = re.compile(
    r"CREATE TABLE (?:IF NOT EXISTS )?(\w+)\s*\((.*?)\n\);", re.S | re.I
)
ALTER_ADD = re.compile(
    r"ALTER TABLE (\w+)\s+ADD COLUMN\s+(?:IF NOT EXISTS )?(\w+)\s+(\w+)", re.I
)
COLUMN = re.compile(r"^\s*(\w+)\s+(UUID|BIGINT|TEXT|INTEGER|INT|INT4|BOOLEAN|BOOL)\b", re.I)
PLACEHOLDER = re.compile(r"\$(\d+)")
# `$1::uuid` or `$1::bigint` is the fix; so is `col::text = $1` on the other side.
CAST = re.compile(r"\$\d+\s*::\s*\w+", re.I)
TYPED = re.compile(r"::\s*(?:uuid|text|bigint|int8|int4|integer|bool)\b", re.I)

SELF_TEST = [
    # A uuid column, an uncast placeholder, and no cast in the file -> reported.
    (["CREATE TABLE t (\n    work_id UUID NOT NULL\n);"],
     'sql("...", "INSERT INTO t (work_id) VALUES ($1)",)', True),
    # Cast on the placeholder -> clean.
    (["CREATE TABLE t (\n    work_id UUID NOT NULL\n);"],
     'sql("...", "INSERT INTO t (work_id) VALUES ($1::uuid)",)', False),
    # Cast on the column side -> clean.
    (["CREATE TABLE t (\n    work_id UUID NOT NULL\n);"],
     'sql("...", "SELECT 1 FROM t WHERE work_id::text = $1",)', False),
    # No placeholder at all -> nothing to bind -> clean.
    (["CREATE TABLE t (\n    work_id UUID NOT NULL\n);"],
     'sql("...", "SELECT work_id FROM t",)', False),
    # A TEXT column takes a text bind with no cast -> clean, even though the
    # file mentions no uuid column at all.
    (["CREATE TABLE t (\n    name TEXT NOT NULL\n);"],
     'sql("...", "INSERT INTO t (name) VALUES ($1)",)', False),
    # ALTER TABLE ADD COLUMN uuid is picked up too.
    (["CREATE TABLE t (\n    id TEXT\n);",
      "ALTER TABLE t ADD COLUMN owner_id UUID NOT NULL;"],
     'sql("...", "INSERT INTO t (owner_id) VALUES ($1)",)', True),
]


def typed_columns(migrations: list[str]) -> dict[str, str]:
    """column name -> declared type, across every table."""
    out: dict[str, str] = {}
    for text in migrations:
        for _table, body in CREATE_TABLE.findall(text):
            for line in body.split("\n"):
                m = COLUMN.match(line)
                if m:
                    out[m.group(1).lower()] = m.group(2).upper()
        for _table, col, typ in ALTER_ADD.findall(text):
            out[col.lower()] = typ.upper()
    return out


def self_test() -> int:
    failures = 0
    for migrations, rust, expected in SELF_TEST:
        cols = typed_columns(migrations)
        has_ph = bool(PLACEHOLDER.search(rust))
        has_cast = bool(CAST.search(rust) or TYPED.search(rust))
        # Only uuid-typed columns are the defect; TEXT takes a text bind as is.
        has_uuid = any(t == "UUID" for t in cols.values())
        got = has_ph and not has_cast and has_uuid
        if got != expected:
            failures += 1
            print(f"FAIL: {rust!r} -> {got}, expected {expected}")
    print(f"self-test: {len(SELF_TEST) - failures}/{len(SELF_TEST)} passed")
    return 1 if failures else 0


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    root = Path(__file__).resolve().parent.parent
    migrations = [
        p.read_text() for p in sorted((root / "migrations" / "postgres").glob("*.sql"))
    ]
    cols = typed_columns(migrations)
    if not cols:
        print("no columns parsed from migrations/postgres; is the path right?")
        return 0
    flagged = 0
    for path in sorted((root / "crates").rglob("*.rs")):
        if "target" in path.parts:
            continue
        text = path.read_text()
        if not PLACEHOLDER.search(text) or TYPED.search(text):
            continue
        named = {
            c for c, t in cols.items()
            if t == "UUID" and re.search(rf"\b{c}\b", text, re.I)
        }
        if not named:
            continue
        flagged += 1
        sample = ", ".join(sorted(named)[:4])
        print(f"{path.relative_to(root)}: $N placeholders, no cast, uses {sample}")
    if flagged:
        print(
            f"\n{flagged} file(s) to read. A placeholder is fine when the column is "
            "TEXT or when the value is already a typed uuid; the runtime error "
            "names the column, so start from the failing statement."
        )
    else:
        print("pg placeholder casts: clean")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
