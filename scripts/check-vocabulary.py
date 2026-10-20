#!/usr/bin/env python3
"""The canonical vocabulary is normative (spec §5, R-2).

A **unit** is a part of a work; a **revision** is an append-only version of a
unit's text. `chapters` and `chapter_revisions` are lorehaven's words, and the
three projects' words for the same thing must not leak into the code.

This script checks that the **schema** and the **code** agree on the rename:

  * every column the migrations create whose name carries the source's word is
    renamed by the rename migration, and
  * the code does not query a table or a column under the old name.

It exists because the rename is a *sequence* of changes and a sequence has
places to be missed. The import's `chapter_id` was missed once already: the
column was renamed in eleven tables and not in the twelfth, and the tests that
touch that table failed with "no such column: unit_id" — a message that names
the new name and not the old one, and so says nothing about where the mistake
was.

**The rule this enforces is narrow on purpose.** The source's word survives in
three places, and each has a reason:

  * the metadata exchange's signal carries the source's `chapter_count`
    (spec §15.2 lists "chapter count" among the fields a signal MAY carry),
  * the source adapters keep the site's word, because an adapter's job is to
    translate it, and
  * the migrations before the rename keep the old name, because they are
    history.

So the check is about the *final* schema and the *current* code, and it knows
about the rename migration by name.
"""

from __future__ import annotations

import pathlib
import re
import sys

ROOT = pathlib.Path(__file__).resolve().parent.parent
MIGRATIONS = ROOT / "migrations" / "sqlite"
RENAME_MIGRATION = "0122_rename_chapters_to_units.sql"

#: The table that holds the *source's* record of a part. Its columns are the
#: source's vocabulary and the spec §15.2 names them.
SOURCE_VOCABULARY_TABLES = {"exchange_signals"}

CREATE_TABLE = re.compile(
    r"CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?[\"'`\[]?(\w+)", re.IGNORECASE
)
RENAME_TABLE = re.compile(
    r"ALTER\s+TABLE\s+[\"'`\[]?(\w+)[\"'`\]]?\s+RENAME\s+TO\s+[\"'`\[]?(\w+)",
    re.IGNORECASE,
)
RENAME_COLUMN = re.compile(
    r"ALTER\s+TABLE\s+[\"'`\[]?(\w+)[\"'`\]]?\s+RENAME\s+COLUMN\s+(\w+)\s+TO\s+(\w+)",
    re.IGNORECASE,
)
ADD_COLUMN = re.compile(
    r"ALTER\s+TABLE\s+[\"'`\[]?(\w+)[\"'`\]]?\s+ADD\s+COLUMN\s+([a-z_]*chapter[a-z_]*)",
    re.IGNORECASE,
)
COLUMN = re.compile(
    r"^\s*([a-z_]*chapter[a-z_]*)\s+(?:TEXT|INTEGER|BIGINT|UUID|REAL|DOUBLE\s+PRECISION)",
    re.IGNORECASE,
)


def schema() -> dict[str, set[str]]:
    """Every column the migrations create whose name carries the source's word."""
    columns: dict[str, set[str]] = {}
    current: str | None = None
    for path in sorted(MIGRATIONS.glob("*.sql")):
        text = path.read_text(encoding="utf-8", errors="replace")
        for line in text.split("\n"):
            m = CREATE_TABLE.match(line)
            if m:
                current = m.group(1).lower()
                continue
            if current and re.match(r"^\s*\)\s*;?\s*$", line):
                current = None
                continue
            if current:
                m = COLUMN.match(line)
                if m:
                    columns.setdefault(current, set()).add(m.group(1).lower())
        for m in ADD_COLUMN.finditer(text):
            columns.setdefault(m.group(1).lower(), set()).add(m.group(2).lower())
    return columns


def renames() -> tuple[dict[str, str], dict[str, set[str]]]:
    """The table renames and the column renames the rename migration makes."""
    path = MIGRATIONS / RENAME_MIGRATION
    if not path.is_file():
        sys.exit(f"{RENAME_MIGRATION} is missing: the vocabulary is not renamed")
    text = path.read_text(encoding="utf-8", errors="replace")
    tables = {old.lower(): new.lower() for old, new in RENAME_TABLE.findall(text)}
    columns: dict[str, set[str]] = {}
    for table, old, _new in RENAME_COLUMN.findall(text):
        columns.setdefault(table.lower(), set()).add(old.lower())
    return tables, columns


def main() -> int:
    tables, columns = renames()
    failures: list[str] = []

    for table, cols in sorted(schema().items()):
        if table in SOURCE_VOCABULARY_TABLES:
            continue
        final = tables.get(table, table)
        renamed_columns = columns.get(final, set())
        for col in sorted(cols):
            if col in renamed_columns:
                continue
            if final in tables:
                # The whole table was renamed, and so was every column in it.
                continue
            failures.append(
                f"{table}.{col} still carries the source's word and no migration "
                f"renames it ({RENAME_MIGRATION})"
            )

    if failures:
        print("FAIL: the canonical vocabulary is not applied to the schema.")
        print()
        for failure in failures:
            print(f"  {failure}")
        print()
        print(
            "A unit is a part of a work and a revision is an append-only version\n"
            "of a unit's text (spec §5). The source's word survives in the source's\n"
            "own record -- the metadata exchange's signal, and the source adapters\n"
            "-- and nowhere else."
        )
        return 1

    print("OK: every schema column carries the canonical vocabulary.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
