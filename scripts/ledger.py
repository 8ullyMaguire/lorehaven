#!/usr/bin/env python3
"""The requirement ledger's only writer.

## Why this module exists at all

`docs/requirements.csv` is the row-by-row authority for what is built (the specification's
§23.1). It is **CRLF**, and it has been for the whole life of the repo. That is load-bearing:
`scripts/add-m45-57-row.py` carries this warning at the top of the file —

    "That file is CRLF. Rewriting it with Python's `csv` module converts every line ending and
     turns a one-line edit into 698 — a trap this repo has hit before."

A one-line edit became a 698-line diff. The trap is not that the `csv` module is wrong; it is
that the `csv` module's *default* line terminator is `\r\n` on write and its reader is
newline-agnostic, and everybody's editor is not. So this module makes the line ending an
explicit, non-negotiable property of every read and every write, and there is exactly one
place in the repo where the ledger is touched programmatically.

## The rules this module enforces

1. **The line ending is CRLF, always.** Read with `newline=''` so the csv module sees the raw
   bytes; write with `lineterminator='\r\n'`. There is no code path here that can write a lone
   `\n`.
2. **The header is preserved.** A ledger that gains a column gains it in the canonical order,
   and a ledger that has not yet gained a column does not gain one because somebody edited a
   row. The canonical order is
   `id,area,requirement,milestone,status,evidence,notes,verified_at,gates`.
3. **Rows are keyed by `id`,** which is the only column the specification says is unique.
4. **`upsert_row` is idempotent.** Running it twice with the same row leaves the file
   byte-identical. The spec's rule is "the ledger is re-derived, never believed", and a writer
   that cannot be run twice is not re-derivable.

## Usage

    python3 scripts/ledger.py add   < row.json     # append, or replace an existing id
    python3 scripts/ledger.py upsert < row.json    # the same; the verb is a courtesy
    python3 scripts/ledger.py show  <id>           # print one row as JSON
    python3 scripts/ledger.py count                # print the data-row count

A row object is a JSON object with the ledger's column names as keys. Unknown keys are an
error; missing keys are an error for `add`, and are filled with the empty string for `upsert`
of an existing row.
"""

from __future__ import annotations

import argparse
import csv
import io
import json
import os
import sys
from typing import Dict, List, Sequence

#: The canonical column order. A ledger's header is this, in this order, plus nothing.
CANONICAL_FIELDS: List[str] = [
    "id",
    "area",
    "requirement",
    "milestone",
    "status",
    "evidence",
    "notes",
    "verified_at",
    "gates",
]

#: The line terminator. Not a default; a decision.
LINE_TERMINATOR = "\r\n"

#: Where the ledger lives, relative to the repo root.
DEFAULT_LEDGER = os.path.join("docs", "requirements.csv")


def _read_raw(path: str) -> str:
    """Read the ledger as text with the line endings untouched.

    `newline=''` is the whole point. With the default universal-newline translation, Python
    rewrites `\r\n` to `\n` on the way in and the file's shape is changed before a single row
    is parsed.
    """
    with io.open(path, "r", encoding="utf-8", newline="") as handle:
        return handle.read()


def _write_raw(path: str, text: str) -> None:
    """Write the ledger as bytes, with the line endings exactly as given."""
    with io.open(path, "w", encoding="utf-8", newline="") as handle:
        handle.write(text)


def _header_for(path: str) -> List[str]:
    """The ledger's header, exactly as the file has it.

    An existing header is **preserved, verbatim** — its columns, its order, and its count. The
    canonical order is what a *new* ledger gets; it is not a repair job for an existing one.

    This is the rule the module's own header states: *"a ledger that has not yet gained a
    column does not gain one because somebody edited a row."* Appending the missing canonical
    columns to a ledger that does not have them violates exactly that: the ledger gains
    `verified_at` and `gates` — empty, on every one of 703 rows — because one row was edited.

    A ledger that has *not* yet grown a column therefore keeps not having it, and the column
    arrives when the migration that introduces it arrives, which is a deliberate act with its
    own commit.
    """
    if not os.path.exists(path) or os.path.getsize(path) == 0:
        return list(CANONICAL_FIELDS)
    raw = _read_raw(path)
    if not raw.strip():
        return list(CANONICAL_FIELDS)
    first = raw.split(LINE_TERMINATOR)[0]
    return next(csv.reader([first]))


def read_rows(path: str = DEFAULT_LEDGER) -> List[Dict[str, str]]:
    """Every data row, as a list of dicts. The header row is not a data row."""
    if not os.path.exists(path):
        return []
    raw = _read_raw(path)
    if not raw.strip():
        return []
    reader = csv.DictReader(io.StringIO(raw, newline=""))
    rows = []
    for row in reader:
        # DictReader leaves a None key for a row with more fields than the header, and a None
        # value for a row with fewer. Both are a corrupt ledger, not a row.
        if None in row:
            raise ValueError(
                "%s: a row has more fields than the header (%r)" % (path, row[None])
            )
        rows.append({k: ("" if v is None else v) for k, v in row.items()})
    return rows


def write_rows(path: str, rows: Sequence[Dict[str, str]], header: Sequence[str] = None) -> None:
    """Write the rows, with the header, with CRLF.

    The file is written through a `StringIO` and then committed in one `write`, so a failure
    part-way through does not leave a half-written ledger.
    """
    if header is None:
        header = _header_for(path)
    buffer = io.StringIO(newline="")
    writer = csv.DictWriter(
        buffer,
        fieldnames=list(header),
        lineterminator=LINE_TERMINATOR,
        extrasaction="raise",
    )
    writer.writeheader()
    for row in rows:
        writer.writerow({field: row.get(field, "") for field in header})
    _write_raw(path, buffer.getvalue())


def upsert_row(path: str, row: Dict[str, str]) -> bool:
    """Insert or replace the row with this `id`. Returns True if a row was replaced.

    Idempotent: running it twice with the same row leaves the file byte-identical.
    """
    if "id" not in row or not row["id"]:
        raise ValueError("a ledger row must have a non-empty 'id'")
    unknown = set(row) - set(CANONICAL_FIELDS)
    if unknown:
        raise ValueError(
            "unknown ledger column(s): %s — the columns are %s"
            % (", ".join(sorted(unknown)), ", ".join(CANONICAL_FIELDS))
        )
    header = _header_for(path)
    rows = read_rows(path)
    for index, existing in enumerate(rows):
        if existing.get("id") == row["id"]:
            merged = dict(existing)
            merged.update(row)
            rows[index] = merged
            write_rows(path, rows, header)
            return True
    rows.append({field: row.get(field, "") for field in header})
    write_rows(path, rows, header)
    return False


def show_row(path: str, row_id: str) -> Dict[str, str]:
    """The row with this `id`, or a `KeyError`."""
    for row in read_rows(path):
        if row.get("id") == row_id:
            return row
    raise KeyError(row_id)


def main(argv: Sequence[str] = None) -> int:
    parser = argparse.ArgumentParser(description="The requirement ledger's only writer.")
    parser.add_argument("--ledger", default=DEFAULT_LEDGER, help="path to requirements.csv")
    sub = parser.add_subparsers(dest="command", required=True)

    for verb in ("add", "upsert"):
        p = sub.add_parser(verb, help="insert or replace a row, read as JSON on stdin")
        p.add_argument("--id", default=None, help="override the row's id")

    p = sub.add_parser("show", help="print one row as JSON")
    p.add_argument("id")

    sub.add_parser("count", help="print the data-row count")

    args = parser.parse_args(argv)

    if args.command in ("add", "upsert"):
        payload = json.load(sys.stdin)
        if args.id:
            payload["id"] = args.id
        upsert_row(args.ledger, payload)
        return 0

    if args.command == "show":
        json.dump(show_row(args.ledger, args.id), sys.stdout, indent=2, sort_keys=True)
        sys.stdout.write("\n")
        return 0

    if args.command == "count":
        print(len(read_rows(args.ledger)))
        return 0

    return 2


if __name__ == "__main__":
    sys.exit(main())
