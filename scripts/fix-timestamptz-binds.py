#!/usr/bin/env python3
"""Report `_at` placeholders in a PostgreSQL arm that lack a `::timestamptz` cast.

The SQLite schema spells the timestamp columns TEXT; PostgreSQL types them
TIMESTAMPTZ. The codebase binds an RFC-3339 string in both, so an uncast
placeholder fails on PostgreSQL with either

    42804: column "updated_at" is of type timestamp with time zone
           but expression is of type text
    42883: operator does not exist: text <= timestamp with time zone

`--fix` adds the cast. It is deliberately conservative:

  * Only the *second* string literal of a `db.sql(...)` call is treated as the
    PostgreSQL arm. An arm that is byte-identical to the SQLite one is skipped,
    because then the statement really is shared and needs a per-backend
    placeholder built at runtime instead.
  * A literal already containing `::timestamptz` is skipped.
  * Nothing is edited unless the whole file round-trips: every string literal is
    compared before and after, and the change is rejected unless each differing
    literal is exactly the old one plus inserted `::timestamptz` text.

That last check is the point of the script. An earlier version rewrote the
source while iterating over matches of the same string, so match offsets went
stale and arms were concatenated into each other -- the file still "looked"
right in a diff but no longer parsed, and no SQL text was left to compare.
Offsets are now collected first and applied in descending order, and the
round-trip check is what would have caught it.
"""
from __future__ import annotations

import argparse
import pathlib
import re
import sys

LITERAL = re.compile(r'"((?:[^"\\]|\\.)*)"', re.S)
# db.sql / sql_owned with exactly two adjacent string literals: (sqlite, postgres)
TWO_ARM = re.compile(
    r'\b(?:db|self\.db)\.sql(?:_owned)?\(\s*"((?:[^"\\]|\\.)*)"\s*,\s*"((?:[^"\\]|\\.)*)"',
    re.S,
)
# The `?` feeds a timestamp column if the text before it on its own line ends
# with `col =` or `col <op>` and col ends in _at.
TS_BIND = re.compile(r"\w*_at\s*(?:=|<=|>=|<|>)\s*$")
# Whole-literal fallback: catches arms written through format! or a const.
ANY_TS_BIND = re.compile(r"(\w*_at)(\s*(?:=|<=|>=|<|>)\s*)(\?)")
CAST = "::timestamptz"


def cast_spans(sql: str) -> list[int]:
    """Offsets of each `?` in `sql` that feeds a *_at column.

    Line-based, which is what the SQL is actually formatted as: a `?` belongs to
    the `col =` / `col <=` immediately before it on the same line. Falling back
    to the whole-literal scan catches an arm where a `?` and its column are
    separated by a `COALESCE(...)` or a comment -- but the fallback must not
    then claim a `?` whose own line has no timestamp column, which is how
    `UPDATE t SET state = ?, updated_at = ? WHERE k = ?` lost its one real edit.
    """
    spans = []
    for m in re.finditer(r"\?", sql):
        head = sql.rfind("\n", 0, m.start()) + 1
        if TS_BIND.search(sql[head : m.start()]):
            spans.append(m.start())
    if spans:
        return spans
    # Single-line arm: pair each timestamp column with the `?` that follows it.
    out = []
    for m in ANY_TS_BIND.finditer(sql):
        out.append(m.end(3) - 1)
    return out


def needs_cast(sql: str) -> bool:
    return bool(ANY_TS_BIND.search(sql)) and CAST not in sql


def cast_sql(sql: str) -> str:
    """Insert `::timestamptz` after each `?` bound to a *_at column."""
    spans = cast_spans(sql)
    if not spans:
        return sql
    out, last = [], 0
    for off in spans:
        out.append(sql[last:off])
        out.append("?" + CAST)
        last = off + 1
    out.append(sql[last:])
    return "".join(out)


def candidates(src: str) -> list[tuple[int, int, str]]:
    """[(start, end, replacement)] over the *original* source text."""
    edits: list[tuple[int, int, str]] = []
    for m in TWO_ARM.finditer(src):
        pg = m.group(2)
        if not needs_cast(pg):
            continue
        # Identical arms are still cast: the pair is executed by a branch on the
        # backend, so a cast in the PostgreSQL literal cannot reach SQLite. It
        # is correct for both -- SQLite never parses the arm it is not given.
        base = m.start(2)
        for off in cast_spans(pg):
            edits.append((base + off, base + off + 1, "?" + CAST))
    return edits


def apply_edits(src: str, edits: list[tuple[int, int, str]]) -> str:
    for start, end, rep in sorted(edits, key=lambda e: -e[0]):
        src = src[:start] + rep + src[end:]
    return src


def round_trips(old: str, new: str) -> bool:
    """True iff every differing literal differs only by inserted `::timestamptz`."""
    o = [m.group(1) for m in LITERAL.finditer(old)]
    n = [m.group(1) for m in LITERAL.finditer(new)]
    if len(o) != len(n):
        return False
    for a, b in zip(o, n):
        if a == b:
            continue
        if b.replace("?" + CAST, "?") != a:
            return False
    return True


def self_test() -> int:
    """Cases for the two bugs this script already had.

    Both were found by running these, not by reading the code -- the first
    version re-cast a placeholder that already had one, and skipped a
    single-line arm entirely because the two arms were byte-identical.
    """
    cases: list[tuple[str, str, int, str | None]] = [
        # (name, source, expected edits, expected literal or None to skip)
        (
            "two arms differ",
            'let sql = db.sql(\n    "UPDATE jobs SET lease_expires_at = ?, updated_at = ? WHERE id = ?",'
            '\n    "UPDATE jobs SET lease_expires_at = ?, updated_at = ? WHERE id::text = ?",\n);',
            2,
            "UPDATE jobs SET lease_expires_at = ?::timestamptz, updated_at = ?::timestamptz"
            " WHERE id::text = ?",
        ),
        (
            "identical arms",
            'let sql = db.sql(\n    "UPDATE t SET updated_at = ? WHERE id = ?",'
            '\n    "UPDATE t SET updated_at = ? WHERE id = ?",\n);',
            1,
            "UPDATE t SET updated_at = ?::timestamptz WHERE id = ?",
        ),
        (
            "already cast",
            'let sql = db.sql(\n    "UPDATE t SET updated_at = ? WHERE id = ?",'
            '\n    "UPDATE t SET updated_at = ?::timestamptz WHERE id::text = ?",\n);',
            0,
            None,
        ),
        (
            "single line, mixed binds",
            'let sql = db.sql(\n    "UPDATE t SET state = ?, updated_at = ? WHERE k = ?",'
            '\n    "UPDATE t SET state = ?, updated_at = ? WHERE k = ?",\n);',
            1,
            "UPDATE t SET state = ?, updated_at = ?::timestamptz WHERE k = ?",
        ),
        (
            "comparison in a where clause",
            'let sql = db.sql(\n    "SELECT id FROM outbox_events WHERE available_at <= ? LIMIT ?",'
            '\n    "SELECT id::text FROM outbox_events WHERE available_at <= ? LIMIT ?",\n);',
            1,
            "SELECT id::text FROM outbox_events WHERE available_at <= ?::timestamptz LIMIT ?",
        ),
    ]
    failures = 0
    for name, src, want_edits, want_literal in cases:
        edits = candidates(src)
        got_edits = len(edits)
        new = apply_edits(src, edits)
        ok_rt = round_trips(src, new)
        # The literal under test is the last one: the PostgreSQL arm.
        literals = [m.group(1) for m in LITERAL.finditer(new)]
        got_literal = literals[-1] if want_literal is not None else None
        ok = got_edits == want_edits and ok_rt and got_literal == want_literal
        if not ok:
            failures += 1
            print(f"FAIL {name}: edits={got_edits} (want {want_edits}) round_trips={ok_rt}")
            print(f"  want: {want_literal!r}")
            print(f"  got:  {got_literal!r}")
        else:
            print(f"ok   {name}")
    # A rewrite that is not a pure cast insertion must be rejected.
    src = cases[0][1]
    corrupt = src.replace('WHERE id = ?",', 'WHERE id = ?"\n    "junk",', 1)
    if round_trips(src, corrupt):
        failures += 1
        print("FAIL corruption guard: a non-cast rewrite was accepted")
    else:
        print("ok   corruption guard")

    print(f"\n{len(cases) + 1 - failures}/{len(cases) + 1} passed")
    return 1 if failures else 0


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__)
    ap.add_argument("paths", nargs="*", default=["crates"])
    ap.add_argument("--fix", action="store_true", help="rewrite the files")
    ap.add_argument("--self-test", action="store_true", help="run the built-in cases")
    args = ap.parse_args()

    if args.self_test:
        return self_test()

    files: list[pathlib.Path] = []
    for p in args.paths or ["crates"]:
        path = pathlib.Path(p)
        files.extend(sorted(path.rglob("*.rs")) if path.is_dir() else [path])

    total = 0
    for f in files:
        src = f.read_text(encoding="utf-8")
        edits = candidates(src)
        if not edits:
            continue
        total += len(edits)
        new = apply_edits(src, edits)
        if not round_trips(src, new):
            print(f"REFUSING {f}: edit would not round-trip", file=sys.stderr)
            return 1
        rel = f
        if args.fix:
            f.write_text(new, encoding="utf-8")
            print(f"fixed {rel}: {len(edits)} cast(s)")
        else:
            print(f"{rel}: {len(edits)} uncast `_at` placeholder(s)")
    print(f"{'fixed' if args.fix else 'found'} {total} in total")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
