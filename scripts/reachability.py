#!/usr/bin/env python3
"""Find the tables nothing reads and the tables nothing writes.

WHY THIS EXISTS

A table that is written and never read is a write amplification with no reader:
the instance pays for the row on every write and nobody ever looks at it. A table
that is read and never written is a reader waiting for a writer that will never
come — and it is usually the sign of a feature that was half-built and shipped
without its other half. **A table that is neither is a table nothing uses**, and
it is the most expensive of the three, because it is a migration with a table.

This is a **gate, not a report**. It exits 0 when every table has both a reader
and a writer, and 1 with the offending tables named when it does not.

WHAT COUNTS, AND WHAT DOES NOT

  * A **write** is `INSERT INTO`, `UPDATE`, `DELETE FROM` or `REPLACE INTO` on
    the table. A **read** is a `SELECT` that mentions it.
  * A `CREATE TABLE` in a migration is **neither** — it is the table's creation,
    not its use.
  * A **test fixture** is **neither** — a test that inserts a row to read it back
    proves the schema works, not that the instance uses the table.
  * A `#[cfg(test)]` module inside a `src/` file is **neither**, for the same
    reason.

The schema is read from **both dialects'** migrations. A table that exists in one
dialect and not the other is itself a finding, and the parity gate covers it —
this script only cares that every table has a reader and a writer.
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys
from collections import defaultdict

#: `CREATE TABLE [IF NOT EXISTS] <name>` — the table's creation, which is not a
#: use of it.
CREATE_TABLE = re.compile(
    r"CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?[\"'`\[]?(\w+)", re.IGNORECASE
)

#: The statements that write, **with the table immediately after the verb**.
#:
#: The table has to be the one being written, not merely mentioned nearby. A
#: table named `spaces` is also the English word, and a scan that looked for the
#: word near a `SELECT` would report a table as read because a sentence about
#: email addresses said "spaces".
WRITE = re.compile(
    r"\b(?:INSERT\s+INTO|UPDATE|DELETE\s+FROM|REPLACE\s+INTO|REPLACE\s+WITH)\s+"
    r"[\"`\[]?(?P<table>\w+)",
    re.IGNORECASE,
)

#: The statement that reads, **with the table immediately after `FROM` or a
#: `JOIN`**.
READ = re.compile(
    r"\b(?:FROM|JOIN)\s+[\"`\[]?(?P<table>\w+)", re.IGNORECASE
)

#: A `#[cfg(test)]` module, which is a test fixture whatever it is called.
CFG_TEST = re.compile(r"#\s*\[\s*cfg\s*\(\s*test\s*\)\s*\]")


#: `ALTER TABLE <old> RENAME TO <new>` -- a table that changed its name.
#:
#: A rename is a change of vocabulary and not of content, and the probe has to
#: follow it: the code reads the table under its NEW name, and a probe that
#: only knew the old one would report the table as having no reader and no
#: writer -- which is a finding about the probe and not about the instance.
RENAME_TABLE = re.compile(
    r"ALTER\s+TABLE\s+[\"'`\[]?(\w+)[\"'`\]]?\s+RENAME\s+TO\s+[\"'`\[]?(\w+)",
    re.IGNORECASE,
)

#: `DROP TABLE <name>` -- a table that stopped existing.
#:
#: The rebuild pattern is `RENAME TO x_broken` + `CREATE TABLE x` + `DROP TABLE
#: x_broken`, so the intermediate name is a name no code ever holds. Following
#: the renames without the drops would report those ghosts as tables with
#: neither a reader nor a writer.
DROP_TABLE = re.compile(
    r"DROP\s+TABLE\s+(?:IF\s+EXISTS\s+)?[\"'`\[]?(\w+)",
    re.IGNORECASE,
)


def schema_tables(migrations: pathlib.Path) -> dict[str, set[str]]:
    """Every table the migrations create, and the dialect it was created in.

    **A renamed table is reported under its new name.** The probe asks whether
    the instance reads and writes a table, and the instance's code names it
    the way the rename left it.
    """
    # The migrations are read IN ORDER, because the schema is a sequence: a
    # table is created, maybe renamed, maybe dropped. Reading only the
    # `CREATE TABLE` statements would report the ghosts of a rebuild as tables,
    # and reading the renames without the order would report a table under the
    # name it had before it was renamed away.
    tables: dict[str, set[str]] = defaultdict(set)
    for dialect in ("sqlite", "postgres"):
        directory = migrations / dialect
        if not directory.is_dir():
            continue
        live: set[str] = set()
        for path in sorted(directory.glob("*.sql")):
            text = path.read_text(encoding="utf-8", errors="replace")
            for name in CREATE_TABLE.findall(text):
                live.add(name.lower())
            for old, new in RENAME_TABLE.findall(text):
                live.discard(old.lower())
                live.add(new.lower())
            for name in DROP_TABLE.findall(text):
                live.discard(name.lower())
        for name in live:
            tables[name].add(dialect)
    return tables


def source_files(crates: pathlib.Path) -> list[pathlib.Path]:
    """Every Rust file under a crate's `src/`, and nothing else.

    `src/` only: the integration tests live in `crates/*/tests/`, and a test
    fixture is not a use of a table.
    """
    found: list[pathlib.Path] = []
    for crate in sorted(crates.iterdir()):
        if not crate.is_dir():
            continue
        src = crate / "src"
        if not src.is_dir():
            continue
        found.extend(sorted(src.rglob("*.rs")))
    return found


def strip_test_modules(text: str) -> str:
    """Drop everything from the first `#[cfg(test)]` onwards.

    A `#[cfg(test)]` module inside a `src/` file is a test fixture: a row it
    inserts to read back proves the schema works, not that the instance uses the
    table. It is the same exclusion as the `tests/` directory, and it is made
    here rather than by a path rule because these modules live in the source.
    """
    match = CFG_TEST.search(text)
    if not match:
        return text
    return text[: match.start()]


def scan(files: list[pathlib.Path], tables: dict[str, set[str]]) -> tuple[dict[str, set[str]], dict[str, set[str]]]:
    """Which tables have a reader, and which have a writer."""
    readers: dict[str, set[str]] = defaultdict(set)
    writers: dict[str, set[str]] = defaultdict(set)

    for path in files:
        text = strip_test_modules(path.read_text(encoding="utf-8", errors="replace"))
        # A statement may be split over lines, so the whole file is searched
        # rather than line by line — a table name on one line and the verb on
        # the next is still a statement about that table.
        for match in WRITE.finditer(text):
            table = match.group("table").lower()
            if table in tables:
                writers[table].add(str(path))
        for match in READ.finditer(text):
            table = match.group("table").lower()
            if table in tables:
                readers[table].add(str(path))
    return readers, writers


#: The section of the known-gaps file that lists the one-sided tables.
GAP_SECTION = "## Unreachable tables"


def known_gap_tables(path: pathlib.Path) -> set[str]:
    """The tables the known-gaps file claims are one-sided.

    The section is a markdown table whose first column is the table's name. The
    section is read **whole**, and every row of it is a claim — so a table that
    is listed and has since been given a reader is still listed, which is what
    the pinning test is for.
    """
    if not path.exists():
        return set()
    lines = path.read_text(encoding="utf-8", errors="replace").split("\n")
    names: set[str] = set()
    inside = False
    for line in lines:
        if line.startswith("## "):
            inside = line.strip() == GAP_SECTION
            continue
        if not inside:
            continue
        stripped = line.strip()
        if not stripped.startswith("|"):
            continue
        cells = [cell.strip() for cell in stripped.strip("|").split("|")]
        if not cells:
            continue
        first = cells[0].strip("`* ")
        if not first or first in ("table",) or set(first) <= set("-: "):
            continue
        names.add(first)
    return names


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--crates", type=pathlib.Path, default=pathlib.Path("crates"))
    parser.add_argument("--migrations", type=pathlib.Path, default=pathlib.Path("migrations"))
    parser.add_argument(
        "--allow",
        type=pathlib.Path,
        default=pathlib.Path("docs/known-gaps.md"),
        help="the known-gaps file whose entries exempt a table",
    )
    args = parser.parse_args(argv)

    tables = schema_tables(args.migrations)
    if not tables:
        print(f"no migrations found under {args.migrations}", file=sys.stderr)
        return 1

    files = source_files(args.crates)
    readers, writers = scan(files, tables)

    # A table listed in the known-gaps file's **Unreachable tables** section is
    # exempt: the gap is *pinned* by a test, and the pinning is what keeps this
    # gate green while the gap is open.
    #
    # The exemption is a **named section**, and not "a table mentioned anywhere in
    # the file". A file of prose mentions table names in passing — an unrelated
    # entry's explanation can name a table without claiming anything about it —
    # and a gate that exempts on a passing mention exempts on noise. The section
    # is where a table is *claimed* to be one-sided, and only there.
    exempt: set[str] = set()
    if args.allow.exists():
        exempt = {
            name.lower()
            for name in known_gap_tables(args.allow)
            if name.lower() in tables
        }

    written_never_read: list[str] = []
    read_never_written: list[str] = []
    neither: list[str] = []

    for table in sorted(tables):
        if table in exempt:
            continue
        has_reader = bool(readers.get(table))
        has_writer = bool(writers.get(table))
        if has_writer and not has_reader:
            written_never_read.append(table)
        elif has_reader and not has_writer:
            read_never_written.append(table)
        elif not has_reader and not has_writer:
            neither.append(table)

    if not (written_never_read or read_never_written or neither):
        print(f"every one of the {len(tables)} table(s) has a reader and a writer")
        return 0

    print("the reachability probe found tables with only one side:", file=sys.stderr)
    for label, names in (
        ("written but never read", written_never_read),
        ("read but never written", read_never_written),
        ("neither read nor written", neither),
    ):
        if names:
            print(f"  {label}:", file=sys.stderr)
            for name in names:
                print(f"    {name}", file=sys.stderr)
    if exempt:
        print(
            f"  ({len(exempt)} table(s) are exempt because they are listed in "
            f"{args.allow})",
            file=sys.stderr,
        )
    print(
        "\nEvery table named above is either given a reader and a writer, or listed in\n"
        f"{args.allow} with a test asserting the current behaviour. The known-gaps entry\n"
        "is what keeps this gate green while the gap is open.",
        file=sys.stderr,
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main())
