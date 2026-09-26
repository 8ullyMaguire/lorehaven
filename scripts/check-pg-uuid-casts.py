#!/usr/bin/env python3
"""Flag a PostgreSQL ``::cast`` that disagrees with the column it is applied to.

A bind's cast has to match the column it is compared against, not the Rust
type of the value. ``groups.owner`` is declared ``TEXT`` in
``migrations/postgres/0013_community.sql`` but every reading of it said
``$1::uuid``, which is ``operator does not exist: text = uuid`` -- a 500 that
only ever appears on the PostgreSQL backend. Five such casts were live in
``community.rs`` and four of the six standing PG test failures traced back to
this class.

The check is derived from the migrations, not from a hand-written column list,
so a migration that changes a column's type is picked up automatically.

Usage:  check-pg-uuid-casts.py [PATH ...]      (default: crates)
        check-pg-uuid-casts.py --self-test
"""
from __future__ import annotations

import pathlib
import re
import sys

MIGRATIONS = "migrations/postgres"

CREATE_TABLE = re.compile(r"CREATE TABLE (?:IF NOT EXISTS )?(\w+)\s*\((.*?)\n\);", re.S)
COLUMN = re.compile(r"\s*(\w+)\s+(\w+)")
TABLE_REF = re.compile(r"\b(?:FROM|INTO|UPDATE|JOIN)\s+(\w+)(?:\s+(?:AS\s+)?(\w+))?")
# The fault is a *bind* cast that does not match the column it is compared
# against: `text_column = $n::uuid`. It is not a cast on the column side, because
# `uuid_column::text = $1` is the correct idiom for binding a Rust `String` and
# appears throughout the tree -- flagging that too reported dozens of false
# positives and would have made this gate noise nobody reads.
# The leading `(?<![:\w])` is load-bearing: without it, `work_id::text = $1::uuid`
# matches with the *column* read as `text`, and the finding is attributed to the
# wrong column -- which is how a self-test asserting on this case can pass a gate
# that is not looking at what you think.
CAST_AFTER = re.compile(r"(?<![:\w])(?:(\w+)\.)?(\w+)\s*=\s*\$\d+::(\w+)")
CAST_BEFORE = re.compile(r"\$\d+::(\w+)\s*=\s*(?:(\w+)\.)?(\w+)::(\w+)")

# PostgreSQL type names, mapped to what `text`/`varchar` collapse to. A cast to
# a type in the same family is harmless; anything else is a mismatch.
FAMILY = {
    "uuid": "uuid",
    "text": "text", "varchar": "text", "char": "text", "bpchar": "text",
    "int2": "int", "int4": "int", "int8": "int", "integer": "int",
    "bigint": "int", "smallint": "int", "serial": "int", "bigserial": "int",
    "bool": "bool", "boolean": "bool",
    "json": "json", "jsonb": "json",
    "timestamptz": "ts", "timestamp": "ts", "timestamptz": "ts", "date": "date",
    "numeric": "numeric", "real": "numeric", "double precision": "numeric",
}


def same_family(declared: str, cast: str) -> bool:
    """Whether a cast to `cast` is safe on a column declared `declared`."""
    want = FAMILY.get(declared, declared)
    got = FAMILY.get(cast, cast)
    # `integer` casts to `int8` widen, and `int8` to `integer` narrows; both are
    # legal and neither is a type error, so any int-to-int is fine.
    if want == "int" and got == "int":
        return True
    return want == got


def column_types(migration_dir: pathlib.Path) -> dict[tuple[str, str], str]:
    """Map ``(table, column) -> declared type`` from the PostgreSQL migrations."""
    types: dict[tuple[str, str], str] = {}
    for path in sorted(migration_dir.glob("*.sql")):
        for match in CREATE_TABLE.finditer(path.read_text()):
            table, body = match.group(1), match.group(2)
            for line in body.split("\n"):
                column = COLUMN.match(line)
                if column:
                    # First definition wins: a later ALTER is a different table.
                    types.setdefault((table, column.group(1)), column.group(2).lower())
    return types


def aliases(statement: str) -> dict[str, str]:
    """Map a table alias to its table: ``JOIN pseuds p ON`` -> ``{p: pseuds}``."""
    mapping: dict[str, str] = {}
    for table, alias in TABLE_REF.findall(statement):
        if not alias:
            continue
        # `ON` and the next clause are not aliases; neither is a real table.
        if alias.upper() in {"ON", "WHERE", "SET", "AND", "OR", "VALUES", "USING"}:
            continue
        mapping[alias] = table
    return mapping


def scan(source: str, types: dict[tuple[str, str], str]) -> list[tuple[int, str, str]]:
    """Return ``(line, table, column)`` for every column whose type and its
    `::cast` disagree. Scans per string literal so a statement is never split."""
    findings: set[tuple[int, str, str]] = set()
    tables = {table for table, _ in types}
    for literal in re.finditer(r'"((?:[^"\\]|\\.)*)"', source, re.DOTALL):
        statement = literal.group(1)
        if "$" not in statement:
            continue
        named = aliases(statement)
        line = source[: literal.start()].count("\n") + 1
        for pattern, order in ((CAST_AFTER, "after"), (CAST_BEFORE, "before")):
            for match in pattern.findall(statement):
                if order == "after":
                    qualifier, column, cast = match
                else:
                    # CAST_BEFORE groups as (cast, qualifier, column, _).
                    cast, qualifier, column, _ = match
                if qualifier:
                    # `p.account_id` belongs to whatever `p` is aliased to. If that
                    # table is unknown, say nothing -- a wrong guess here is how a
                    # gate starts crying wolf on correct code.
                    table = named.get(qualifier, "")
                    if table and not same_family(types.get((table, column), ""), cast):
                        findings.add((line, table, f"{column}:: {cast}"))
                    continue
                # Unqualified: only a problem if EVERY table in the statement
                # declares that column, and they all disagree.
                owners = [t for t in tables if re.search(rf"\b{t}\b", statement)]
                declared = [types.get((t, column)) for t in owners]
                if declared and all(d is not None for d in declared) and all(
                    not same_family(d, cast) for d in declared
                ):
                    findings.add((line, ", ".join(owners), f"{column}:: {cast}"))
    return sorted(findings)


def check(root: pathlib.Path, paths: list[str]) -> int:
    types = column_types(root / MIGRATIONS)
    if not types:
        print(f"no migrations under {root / MIGRATIONS} -- cannot verify casts")
        return 2
    failures = 0
    for rel in paths:
        for path in sorted(root.glob(rel)):
            for line, table, column in scan(path.read_text(), types):
                print(f"{path.relative_to(root)}:{line}: {column} on {table} "
                      f"does not match the cast the statement applies")
                failures += 1
    return 1 if failures else 0


# A whole statement, so the check cannot pass by reading only part of one.
SOURCE_GOOD = '''\
fn get(db: &Database) {
    let sql = db.sql(
        "SELECT id FROM accounts WHERE id = $1::uuid",
        "SELECT id FROM accounts WHERE id = ?",
    );
}
fn group(db: &Database) {
    let sql = db.sql(
        "SELECT id FROM groups WHERE id = ?",
        "SELECT id FROM groups WHERE id = $1",
    );
}
'''
SOURCE_BAD = '''\
fn group(db: &Database) {
    let sql = db.sql(
        "SELECT id FROM groups WHERE id = ?",
        "SELECT id FROM groups WHERE id = $1::uuid",
    );
}
'''


def self_test(root: pathlib.Path) -> int:
    types = column_types(root / MIGRATIONS)
    if not types:
        print("FAIL cannot read migrations")
        return 1
    # Pin the premise: these columns really are TEXT and accounts.id really is uuid.
    # `work_tags` is the trap: `work_id` is UUID while `node_id` is TEXT, in the
    # same table. An exclusion predicate cast `work_id::text` on the assumption
    # the whole table was TEXT, which made every filtered search 500 on
    # PostgreSQL and pass on SQLite. Pin both so the assumption cannot return.
    for table, column, want in (
        ("groups", "id", "text"),
        ("groups", "owner", "text"),
        ("group_members", "account", "text"),
        ("accounts", "id", "uuid"),
        ("work_tags", "work_id", "uuid"),
        ("work_tags", "node_id", "text"),
        ("taxonomy_nodes", "id", "text"),
    ):
        got = types.get((table, column))
        if got != want:
            print(f"FAIL {table}.{column} is {got}, expected {want}")
            return 1
    cases = [
        ("a uuid column cast to uuid is fine", SOURCE_GOOD, 0),
        ("a TEXT column cast to uuid is reported", SOURCE_BAD, 1),
        # The qualified form `g.owner = $1::uuid` must be caught too.
        ("qualified column", SOURCE_BAD.replace("id = $1::uuid", "owner = $1::uuid"), 1),
        # A `::uuid` inside a string with no table reference is not our business.
        ("no table reference", 'let x = "SELECT $1::uuid";', 0),
        # The live bug, verbatim in shape: a UUID column cast to text and
        # compared against a *uuid* bind, so the comparison is text = uuid.
        # (`work_id::text = $1` with a text bind is the correct idiom and must
        # not be reported -- it appears throughout the tree.)
        (
            "uuid column compared to a uuid bind",
            '"SELECT 1 FROM work_tags cf_wt '
            "WHERE cf_wt.work_id::text = $1::uuid",
            0,
        ),
        (
            "text column compared to a uuid bind",
            '"SELECT 1 FROM groups g WHERE g.owner = $1::uuid"',
            1,
        ),
        # The SQLite arm is never flagged: it has no $n at all.
        ("sqlite arm ignored", SOURCE_GOOD.split("let sql = db.sql(")[0] + '"WHERE id = ?"', 0),
        ("empty source", "", 0),
    ]
    failures = 0
    for name, source, want in cases:
        got = len(scan(source, types))
        if got != want:
            print(f"FAIL {name}: got {got}, want {want}")
            failures += 1
    if failures:
        print(f"{failures} case(s) failed")
        return 1
    print(f"{len(cases)} cases passed")
    return 0


def main(argv: list[str]) -> int:
    root = pathlib.Path(__file__).resolve().parent.parent
    if "--self-test" in argv:
        return self_test(root)
    paths = [a for a in argv[1:] if not a.startswith("-")] or ["crates/**/*.rs"]
    return check(root, paths)


if __name__ == "__main__":
    sys.exit(main(sys.argv))
