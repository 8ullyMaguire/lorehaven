#!/usr/bin/env python3
"""Rewrite `?` placeholders to `$n` in the PostgreSQL arms of backend matches.

`?` is SQLite syntax. A hand-written `match db.backend()` whose PostgreSQL arm
copied the SQLite arm's query literals is a syntax error at the first `AND` on
PostgreSQL, and is invisible on SQLite. Rewriting those arms by hand is
error-prone -- the strings appear twice, in adjacent arms, and a `replace(.., 1)`
will happily edit the SQLite one. This walks the arms by brace depth and only
touches the PostgreSQL ones.

    python3 scripts/fix-pg-arm-placeholders.py --write crates/db/src/foo.rs
    python3 scripts/fix-pg-arm-placeholders.py crates/db/src/foo.rs   # dry run

Placeholders are numbered across the whole PostgreSQL arm, in order of
appearance, starting at $1: an arm usually runs several statements, each with
its own `.bind(..)` chain, and a literal that restarted at $1 would collide with
the statement before it. The SQLite arm is never touched.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ARM = re.compile(r"Backend::(Sqlite|Postgres)\s*=>\s*\{")
# A Rust string literal holding SQL, with `?` and no `$` yet.
LITERAL = re.compile(r'"((?:[^"\\]|\\.)*)"')


def placeholder_count(lit: str) -> int:
    """Number of `?` bind parameters in a SQL literal.

    PostgreSQL's JSONB containment operators are also spelled `?`, `?|` and `?&`
    (e.g. `WHERE tags ? 'genre'`), so a `?` immediately followed by a quote, a
    pipe or an ampersand is an operator rather than a bind.

    Everything else counts, including a `?` that ends the literal or the line:
    `LIMIT ?` and a `?` before a line-continuation are ordinary placeholders.
    """
    n = 0
    for i, ch in enumerate(lit):
        if ch == "?" and not is_jsonb_op(lit, i):
            n += 1
    return n


def is_jsonb_op(lit: str, i: int) -> bool:
    """True when the `?` at `i` is PostgreSQL's JSONB containment operator."""
    j = i + 1
    while j < len(lit) and lit[j] == " ":
        j += 1
    return j < len(lit) and lit[j] in ("'", "|", "&")


def renumber(lit: str, start: int) -> tuple[str, int]:
    """Replace each bind `?` with `$n`, counting on from `start`."""
    out, idx = [], start
    for i, ch in enumerate(lit):
        if ch == "?":
            if is_jsonb_op(lit, i):
                out.append(ch)
            else:
                idx += 1
                out.append(f"${idx}")
        else:
            out.append(ch)
    return "".join(out), idx


def rewrite_literals(body: str) -> tuple[str, int]:
    """Renumber `?` placeholders to `$n`, continuing across the whole arm.

    Numbering restarts at $1 for the arm, not for each literal: an arm often
    runs several statements, each with its own `.bind(..)` chain, and a literal
    that started again at $1 would collide with the statement before it.
    """
    out, pos, idx, changed = [], 0, 0, 0
    for m in LITERAL.finditer(body):
        lit = m.group(1)
        if not placeholder_count(lit):
            out.append(body[pos : m.end()])
            pos = m.end()
            continue
        new_lit, idx = renumber(lit, idx)
        changed += placeholder_count(lit)
        out.append(body[pos : m.start()])
        out.append('"' + new_lit + '"')
        pos = m.end()
    out.append(body[pos:])
    return "".join(out), changed


def fix_text(text: str) -> tuple[str, int]:
    total = 0
    for m in list(ARM.finditer(text)):
        if m.group(1) != "Postgres":
            continue
        start = m.end() - 1
        depth, i = 0, m.end() - 1
        while i < len(text):
            if text[i] == "{":
                depth += 1
            elif text[i] == "}":
                depth -= 1
                if depth == 0:
                    break
            i += 1
        arm = text[start + 1 : i]
        if "?" not in arm:
            continue
        new_arm, n = rewrite_literals(arm)
        if n:
            text = text[: start + 1] + new_arm + text[i:]
            total += n
    return text, total


def main(argv: list[str]) -> int:
    write = "--write" in argv
    paths = [a for a in argv if not a.startswith("--")]
    if not paths:
        print(__doc__)
        return 2
    for raw in paths:
        path = Path(raw)
        text = path.read_text()
        fixed, n = fix_text(text)
        if n:
            print(f"{path}: {n} placeholder(s)" + ("  [written]" if write else "  [dry run]"))
            if write:
                path.write_text(fixed)
        else:
            print(f"{path}: clean")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
