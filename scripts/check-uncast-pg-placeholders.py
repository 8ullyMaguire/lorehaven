#!/usr/bin/env python3
"""Find PostgreSQL statements that bind text to a column the dialect types.

`Database::sql` rewrites `?` to `$n` but never casts, and sqlx sends a bound
`&str` as `text`. So a statement that reads

    SELECT dimension_key FROM arena_weights WHERE account_id = $1

works on SQLite and fails on PostgreSQL with

    42804 column "account_id" is of type uuid but expression is of type text

or, where the statement does cast the placeholder and the *column* is read into
a String instead, with a decode error:

    ColumnDecode { source: "mismatched types; Rust type `String`
                   (as SQL type `TEXT`) is not compatible with SQL type `UUID` }

Both are the same underlying mistake, and both are invisible on SQLite -- which
is how 62 of them shipped. The gate that catches this is running the suite on
both dialects; this script is the cheap pre-flight that says where to look.

The verdict comes from the migrations, not from a hand-kept list of tables. An
earlier version carried a `UUID_KEYED_TABLES` set that had already drifted: it
claimed `admin_actions` and `listings` were uuid-keyed when both are TEXT in
`migrations/postgres/`, so it reported statements that were never wrong. A
list of tables cannot stay right; parsing the schema can. Types are read out of
`migrations/postgres/*.sql`, so a new migration that retypes a column is picked
up on the next run with no edit here.

A column is reported when it is bound or compared to a placeholder and its
PostgreSQL type is not one that a bound `&str` can satisfy: that is, not text,
varchar, char, or any other string-typed column.

Usage:  check-uncast-pg-placeholders.py [crates/...]

Options: --migrations DIR   where the PostgreSQL DDL lives
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys
from collections import defaultdict

# Column types a bound `&str` cannot satisfy by accident, and which therefore
# have to be named in the statement. A TIMESTAMPTZ column is here because the
# codebase stores timestamps as RFC 3339 strings and binds them as text, which
# PostgreSQL will not coerce: the statement has to say `::timestamptz`. Numeric
# and boolean types are deliberately absent -- a test may well be binding a real
# `i64`/`bool` there, and the type alone cannot tell, so those are settled by
# running the suite on both dialects rather than by guessing.
NEEDS_EXPLICIT_CAST = {
    "uuid", "timestamptz", "timestamp", "timestamp with time zone",
    "timestamp without time zone", "date", "json", "jsonb", "uuid[]",
}

# `CREATE TABLE x (` ... `);` in a migration, then `col  TYPE ...` lines.
CREATE_TABLE = re.compile(
    r"CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?([a-z_][a-z0-9_]*)\s*\((.*?)\n\);",
    re.IGNORECASE | re.DOTALL,
)
COLUMN = re.compile(
    r"^\s*([a-z_][a-z0-9_]*)\s+([A-Za-z][A-Za-z0-9_ ]*?(?:\([^)]*\))?)"
    r"(?:\s+(?:NOT\s+NULL|NULL|DEFAULT\b[^,]*|PRIMARY\s+KEY|UNIQUE|REFERENCES\b.*?))*\s*,",
    re.IGNORECASE | re.MULTILINE,
)
# The final column of a CREATE TABLE has no trailing comma, so COLUMN cannot
# match it. Take it separately: same shape, comma optional only at the end, and
# anchored so it cannot re-read a line COLUMN already handled.
LAST_COLUMN = re.compile(
    r"^\s*([a-z_][a-z0-9_]*)\s+([A-Za-z][A-Za-z0-9_ ]*?(?:\([^)]*\))?)"
    r"(?:\s+(?:NOT\s+NULL|NULL|DEFAULT\b[^,]*|PRIMARY\s+KEY|UNIQUE|REFERENCES\b.*))?\s*$",
    re.IGNORECASE | re.MULTILINE,
)


def load_schema(migrations: pathlib.Path) -> dict[str, dict[str, str]]:
    """table -> {column: postgres type}, read from the PostgreSQL migrations."""
    schema: dict[str, dict[str, str]] = {}
    for path in sorted(migrations.glob("*.sql")):
        text = path.read_text(encoding="utf-8", errors="replace")
        for match in CREATE_TABLE.finditer(text):
            table, body = match.group(1).lower(), match.group(2)
            columns = schema.setdefault(table, {})
            found = list(COLUMN.finditer(body)) + list(LAST_COLUMN.finditer(body))
            for col in found:
                # The trailing constraint group is non-greedy, so on
                # `uuid NOT NULL` the captured type can still carry it.
                coltype = " ".join(col.group(2).split()).lower()
                for noise in ("not null", "null", "primary key", "unique"):
                    coltype = re.sub(rf"\b{noise}\b.*", "", coltype).strip()
                # Last writer wins. A table is often declared once and then
                # redeclared by a later migration that retypes columns, and the
                # schema the database actually has is the last one applied.
                columns[col.group(1).lower()] = coltype
    return schema


def tables_in(sql: str) -> set[str]:
    lowered = sql.lower()
    return {
        t for t in re.findall(r"\b(?:from|join|into|update)\s+([a-z_][a-z0-9_]*)", lowered)
    }


# `col = $n` / `col IN ($n...)` / `col <op> $n` -- the comparison form.
COMPARED = re.compile(
    r"\b([a-z_][a-z0-9_.]*)\s*(?:=|<>|!=|>|<|like|ilike|in)\s*\$(\d+)",
    re.IGNORECASE,
)
# The INSERT form. `\s*` spans the newline, because these statements are often
# written with the column list on its own lines and a naive `VALUES (` match
# misses the wrapped ones.
VALUES_START = re.compile(r"VALUES\s*\(", re.IGNORECASE)
INSERT_COLUMN = re.compile(r"^\s*([a-z_][a-z0-9_]*)\s*(?:\([^)]*\))?\s*(?:,|$)", re.IGNORECASE | re.MULTILINE)

STRING_LIT = re.compile(r'"((?:[^"\\]|\\.)*)"', re.DOTALL)


def referenced_columns(sql: str) -> set[str]:
    """Every column the statement names, so we can ask the schema about it."""
    cols = {m.group(1).split(".")[-1].lower() for m in COMPARED.finditer(sql)}
    cols |= {c.lower() for c in re.findall(r"\bSELECT\b(.*?)\bFROM\b", sql, re.IGNORECASE | re.DOTALL)
             for c in re.findall(r"[a-z_][a-z0-9_.]*", c)}
    return cols


def insert_column_order(sql: str) -> list[str]:
    """The column list of an INSERT, in order, so binds line up positionally."""
    head = sql.split("VALUES", 1)
    if len(head) < 2 or "(" not in head[0]:
        return []
    inner = head[0][head[0].rindex("(") + 1:]
    return [m.group(1).lower() for m in INSERT_COLUMN.finditer(inner)]


def offending_lines(sql: str, schema: dict[str, dict[str, str]]) -> bool:
    """True when this statement hands text to a column the dialect types."""
    if "::" in sql:
        return False  # already casts somewhere; not a bare-placeholder site
    if not re.search(r"\$\d+", sql):
        # `?` is the SQLite spelling. The PostgreSQL arm is the one rewritten to
        # `$n`, so without a placeholder this is a SQLite arm and cannot fail.
        return False

    tables = tables_in(sql)
    known = {t: schema[t] for t in tables if t in schema}
    if not known:
        return False

    # Comparison form: a non-textual column compared to a placeholder.
    for match in COMPARED.finditer(sql):
        bare = match.group(1).split(".")[-1].lower()
        for columns in known.values():
            coltype = columns.get(bare)
            if coltype and not _accepts_text(coltype):
                return True

    # INSERT form: a non-textual column in the list taking a positional bind.
    order = insert_column_order(sql)
    if order and VALUES_START.search(sql):
        for columns in known.values():
            for col in order:
                coltype = columns.get(col)
                if coltype and not _accepts_text(coltype):
                    return True

    return False


def _accepts_text(coltype: str) -> bool:
    base = coltype.split("(")[0].strip()
    return base not in NEEDS_EXPLICIT_CAST


def scan(root: pathlib.Path, schema: dict[str, dict[str, str]]) -> list[tuple[pathlib.Path, int, str]]:
    hits: list[tuple[pathlib.Path, int, str]] = []
    # Accept a file as well as a directory, so a single module can be checked
    # while working on it.
    paths = [root] if root.is_file() else sorted(root.rglob("*.rs"))
    for path in paths:
        if path.suffix != ".rs" or "/target/" in str(path):
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        for match in STRING_LIT.finditer(text):
            sql = match.group(1)
            if not re.search(r"\b(?:SELECT|INSERT|UPDATE|DELETE)\b", sql, re.IGNORECASE):
                continue
            if not offending_lines(sql, schema):
                continue
            line = text[: match.start()].count("\n") + 1
            flat = " ".join(sql.split())[:78]
            hits.append((path, line, flat))
    return hits


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paths", nargs="*", type=pathlib.Path,
                        default=[pathlib.Path("crates")])
    parser.add_argument("--migrations", type=pathlib.Path,
                        default=pathlib.Path("migrations/postgres"),
                        help="directory holding the PostgreSQL DDL")
    args = parser.parse_args(argv[1:])

    schema = load_schema(args.migrations)
    if not schema:
        print(f"no tables parsed from {args.migrations}", file=sys.stderr)
        return 2

    hits: list[tuple[pathlib.Path, int, str]] = []
    for root in (args.paths or [pathlib.Path("crates")]):
        hits.extend(scan(root, schema))

    if not hits:
        print(f"OK: no text bound to a typed column "
              f"({len(schema)} tables read from {args.migrations})")
        return 0

    by_file: dict[pathlib.Path, list[tuple[int, str]]] = defaultdict(list)
    for path, line, sql in hits:
        by_file[path].append((line, sql))

    total = sum(len(v) for v in by_file.values())
    print(f"{total} site(s) binding text to a typed column, in {len(by_file)} file(s):\n")
    for path, entries in sorted(by_file.items(), key=lambda kv: -len(kv[1])):
        print(f"  {path}  ({len(entries)})")
        for line, sql in entries:
            print(f"    {line:5}  {sql}")
    print(
        "\nEach of these passes on SQLite and fails on PostgreSQL. Fix by casting\n"
        "the placeholder in the PostgreSQL arm ($1::uuid), or -- when the column is\n"
        "read into a String -- casting the column in the SELECT ($1::text). The\n"
        "SQLite arm is unchanged. Column types come from the migrations, so if a\n"
        "report turns out to be wrong the migration is what needs fixing, not this\n"
        "script."
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
