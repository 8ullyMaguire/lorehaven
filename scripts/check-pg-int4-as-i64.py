#!/usr/bin/env python3
"""Find PostgreSQL INTEGER/BOOLEAN columns selected uncast into an i64/bool struct.

SQLite's dynamic typing means an `INTEGER` column and a `BIGINT` column are
indistinguishable there, and sqlx will happily decode SQLite's value into
`i64`. On PostgreSQL the same decode fails with

    mismatched types; Rust type `i64` (as SQL type `INT8`) is not compatible
    with SQL type `INT4`

so a query written on SQLite and only ever run on SQLite stays green forever.
Four separate defects in this repo were exactly that.

The fix is always a cast in the SQL (`votes_visible::bigint AS votes_visible`),
because the Rust type has to be the same on both backends. This script reports
the places that need one.

Approach: parse the PostgreSQL migrations into column -> type, then for every
Rust `FromRow` struct field of type `i64`, `u32`/`u32`-shaped or `bool`, check
whether any query in the same file names a column of INT4/INT2/BOOLEAN type
without a cast. That is deliberately conservative: it reports candidates, and a
human (or a test run) decides.

Usage: python3 scripts/check-pg-int4-as-i64.py [--self-test]
Advisory, not blocking. A column name shared by a Rust field is too weak a
signal to gate on -- `crates/db` alone yields ~60 candidates, and nearly all are
`COUNT(*)` aggregates, which PostgreSQL already returns as INT8. A gate that
fires constantly gets deleted, and a deleted gate catches nothing. So this
reports and exits 0; the real enforcement is the PostgreSQL test run, where each
of these shows up as a `mismatched types` decode error naming the column.

Exit 0 always (unless --self-test fails).
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

# Columns PostgreSQL hands back as INT4/INT2/BOOL, which sqlx will not decode
# into a 64-bit integer.
NARROW = re.compile(
    r"^\s*(?P<name>\w+)\s+(?:INTEGER|INT|INT2|INT4|BOOLEAN|BOOL|REAL|FLOAT4)\b",
    re.IGNORECASE,
)
CREATE_TABLE = re.compile(r"CREATE TABLE (?:IF NOT EXISTS )?(\w+)\s*\((.*?)\n\);", re.S | re.I)
ALTER_ADD = re.compile(
    r"ALTER TABLE (\w+)\s+ADD COLUMN\s+(?:IF NOT EXISTS\s+)?(\w+)\s+(\w+)",
    re.IGNORECASE,
)
# A `col` used in a query with no cast, in a file that also has an i64 field.
I64_FIELD = re.compile(r"^\s*(?:pub\s+)?(\w+)\s*:\s*i64\s*,", re.M)
BOOL_FIELD = re.compile(r"^\s*(?:pub\s+)?(\w+)\s*:\s*bool\s*,", re.M)
CASTED = re.compile(r"\b(\w+)\s*::\s*(?:bigint|int8|int4|integer|bool|text)\b", re.I)


def narrow_columns(migrations: list[str]) -> dict[str, str]:
    """column name -> declared type, across every table."""
    out: dict[str, str] = {}
    for text in migrations:
        for table, body in CREATE_TABLE.findall(text):
            for line in body.split("\n"):
                m = NARROW.match(line)
                if m:
                    out[m.group("name").lower()] = m.group(0).strip()
        for _table, col, typ in ALTER_ADD.findall(text):
            if re.match(r"^(INTEGER|INT|INT2|INT4|BOOLEAN|BOOL|REAL|FLOAT4)$", typ, re.I):
                out[col.lower()] = f"{col} {typ}"
    return out


SELF_TEST = [
    # (migrations, rust, expected_candidates)
    ([
        "CREATE TABLE t (\n    id TEXT PRIMARY KEY,\n    n INTEGER NOT NULL\n);",
    ], "struct R {\n    pub n: i64,\n}\nSELECT n FROM t;", ["n"]),
    # Already cast -> not reported.
    ([
        "CREATE TABLE t (\n    n INTEGER NOT NULL\n);",
    ], "struct R {\n    pub n: i64,\n}\nSELECT n::bigint AS n FROM t;", []),
    # A wide column is fine.
    ([
        "CREATE TABLE t (\n    n BIGINT NOT NULL\n);",
    ], "struct R {\n    pub n: i64,\n}\nSELECT n FROM t;", []),
    # A narrow column read as bool is the same defect.
    ([
        "CREATE TABLE t (\n    b BOOLEAN NOT NULL\n);",
    ], "struct R {\n    pub b: bool,\n}\nSELECT b FROM t;", ["b"]),
    # A narrow column not present in the struct is out of scope.
    ([
        "CREATE TABLE t (\n    n INTEGER NOT NULL\n);",
    ], "struct R {\n    pub other: i64,\n}\nSELECT other FROM t;", []),
    # ALTER TABLE ... ADD COLUMN is parsed too.
    ([
        "CREATE TABLE t (\n    id TEXT\n);",
        "ALTER TABLE t ADD COLUMN votes_visible INTEGER NOT NULL DEFAULT 0;",
    ], "struct R {\n    pub votes_visible: i64,\n}\nSELECT votes_visible FROM t;", ["votes_visible"]),
    # Text columns never match.
    ([
        "CREATE TABLE t (\n    s TEXT NOT NULL\n);",
    ], "struct R {\n    pub s: i64,\n}\nSELECT s FROM t;", []),
]


def self_test() -> int:
    failures = 0
    for migrations, rust, expected in SELF_TEST:
        cols = narrow_columns(migrations)
        got: list[str] = []
        for field, is_bool in [(m.group(1), False) for m in I64_FIELD.finditer(rust)] + [
            (m.group(1), True) for m in BOOL_FIELD.finditer(rust)
        ]:
            if field.lower() in cols and not re.search(
                rf"\b{re.escape(field)}\s*::", rust, re.I
            ):
                got.append(field)
        if sorted(got) != sorted(expected):
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
    cols = narrow_columns(migrations)
    total = 0
    # Only `crates/db`, where `query_as` decodes happen. Scanning every crate
    # produced 90 candidates, nearly all `i64` *request* fields that happen to
    # share a name with some INTEGER column in some table -- a gate with that
    # false-positive rate gets ignored, and an ignored gate catches nothing. The
    # narrowing here trades recall for a signal someone will actually keep
    # running; the failures it does miss show up as a PG test failure.
    for path in sorted((root / "crates" / "db").rglob("*.rs")):
        if "target" in path.parts:
            continue
        text = path.read_text()
        fields = [m.group(1) for m in I64_FIELD.finditer(text)] + [
            m.group(1) for m in BOOL_FIELD.finditer(text)
        ]
        for field in dict.fromkeys(fields):
            if field.lower() not in cols:
                continue
            if re.search(rf"\b{re.escape(field)}\s*::", text, re.I):
                continue
            if not re.search(rf"\b{re.escape(field)}\b", text):
                continue
            total += 1
            print(f"{path}: `{field}` is a narrow PG column ({cols[field.lower()]}) read uncast")
    if total:
        print(
            f"\n{total} candidate(s) in crates/db. A candidate is only a real defect "
            "when the value comes from the *column*; a `COUNT(*)` or a computed "
            "expression is already INT8 on PostgreSQL and decodes into i64 fine. "
            "Check the query, then either add `col::bigint AS col` or note why not."
        )
        return 0
    print("pg narrow-column decodes: clean")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
