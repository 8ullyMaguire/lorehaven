#!/usr/bin/env python3
"""Fail if a `Backend::Postgres` arm reaches for the SQLite pool.

`Database::sqlite_pool()` returns `None` for a PostgreSQL handle, so the
`expect("sqlite")` that virtually every call site carries fires. The panic
message is the bare string "sqlite", which does not name the real problem --
it reads like "the fixture was wired to the wrong database", when in fact
production code asked for the wrong pool.

That is exactly how `crates/db/src/thread_modes.rs` shipped: `create_forum_topic`
and `add_schedule_section` each had a `Backend::Postgres` arm whose statement
was written for PostgreSQL (`$1::uuid`) and whose executor was
`db.sqlite_pool().expect("sqlite")`. Under SQLite both call sites were never
taken, so the tests passed; under PostgreSQL 27 forum tests died with "sqlite",
which is most of what was failing.

The check is a brace-depth scan rather than a regex over a lookback window,
because a `sqlite_pool()` is only wrong when it sits inside a *Postgres* arm --
roughly 800 of the 824 uses in `crates/*/src` are inside `Backend::Sqlite` arms
and are correct.

Usage:  python3 scripts/check-pg-arm-uses-sqlite-pool.py [paths...]
Exit 0 when clean, 1 with a report when not.
"""

from __future__ import annotations

import pathlib
import re
import sys

# `sqlite_pool()` and the arm header forms that can enclose it.
POOL = re.compile(r"sqlite_pool\(\)")
# Backend::Postgres / Backend::Sqlite, however the arm is spelled.
POSTGRES = re.compile(r"Postgres")


def scan(text: str) -> list[tuple[int, str]]:
    """Every `sqlite_pool()` in `text`, with the header of the block it sits in.

    Walks the source once maintaining brace depth and a depth -> opening-line
    map, then re-derives that map at each match. String literals are skipped so
    a brace inside SQL text does not shift the depth.
    """
    out: list[tuple[int, str]] = []
    for m in POOL.finditer(text):
        depth = 0
        opened: dict[int, str] = {}
        in_str: str | None = None
        prev = ""
        i = 0
        while i < m.start():
            ch = text[i]
            if in_str is not None:
                if ch == in_str and prev != "\\":
                    in_str = None
            elif ch in "\"'":
                in_str = ch
            elif ch == "{":
                depth += 1
                head = text[:i].rstrip()
                opened[depth] = head[head.rfind("\n") + 1 :].strip()
            elif ch == "}":
                opened.pop(depth, None)
                depth -= 1
            prev = ch
            i += 1
        out.append((m.start(), opened.get(depth, "")))
    return out


def main(argv: list[str]) -> int:
    roots = [pathlib.Path(a) for a in argv[1:]] or [pathlib.Path("crates")]
    files: list[pathlib.Path] = []
    for root in roots:
        if root.is_dir():
            files += [
                p
                for p in sorted(root.rglob("*.rs"))
                if "/src/" in p.as_posix() or "/tests/" in p.as_posix()
            ]
        else:
            files.append(root)

    bad: list[tuple[pathlib.Path, int, str]] = []
    for path in files:
        text = path.read_text(encoding="utf-8", errors="replace")
        if "sqlite_pool()" not in text:
            continue
        for pos, header in scan(text):
            if POSTGRES.search(header):
                line = text[:pos].count("\n") + 1
                bad.append((path, line, header))

    if not bad:
        print("ok: no Backend::Postgres arm calls sqlite_pool()")
        return 0

    print(f"{len(bad)} site(s) call sqlite_pool() inside a PostgreSQL arm:\n")
    for path, line, header in bad:
        print(f"  {path}:{line}")
        print(f"      inside: {header}")
    print(
        "\nThe executor for a PostgreSQL arm is\n"
        "    .execute(db.postgres_pool().expect(\"postgres\"))\n"
        "A `Backend::Postgres` arm writing `$1::uuid` while executing on the\n"
        "SQLite pool passes every SQLite test and panics with the message\n"
        "\"sqlite\" under LOREHAVEN_TEST_PG_URL."
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
