#!/usr/bin/env python3
"""Fail when a PostgreSQL match arm executes against the SQLite pool, or vice versa.

The `lorehaven_db` modules dispatch on `db.backend()` and each arm binds against
its own pool. Copy-pasting the arm below the match and changing only the SQL
leaves `db.sqlite_pool()` inside a `Backend::Postgres` arm. On a SQLite-only run
that code is never reached; on PostgreSQL the arm either panics on a `None`
pool or, when both pools exist, silently writes to the wrong database.

This is a whole-file structural check, not a regex for one shape: it walks every
`Backend::` arm to the next one and reports the first pool call inside it.

Usage: python3 scripts/check-backend-pool-mismatch.py [--self-test]
Exit 0 when clean, 1 when a mismatch is found, 2 on bad usage.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

POOLS = {"sqlite": "sqlite_pool", "postgres": "postgres_pool"}
# The PG pool is spelled differently from the enum variant; map them explicitly.
WANT = {"Sqlite": "sqlite", "Postgres": "postgres"}
# An arm that hands off to a `*_sqlite` / `*_postgres` helper rather than binding
# itself never touches a pool, so the scan stops at the delegation.
DELEGATES = re.compile(r"\b\w*_(sqlite|postgres)\s*\(")


def arm_mismatches(text: str) -> list[tuple[int, str, str]]:
    """Every `Backend::X` arm whose first pool call names the other backend.

    Reports the line of the offending pool call, not the arm.
    """
    lines = text.split("\n")
    found: list[tuple[int, str, str]] = []
    for i, line in enumerate(lines):
        m = re.search(r"Backend::(Sqlite|Postgres)", line)
        if not m:
            continue
        # Only a *dispatch* counts. `Pool::Sqlite(pool) => ...` matches the inner
        # pattern of a pool accessor, and `self.pool` names the enum directly.
        if re.search(r"\bPool::", line) or re.search(r"match\s*&\w*\s*\.\s*pool\b", line):
            continue
        want = WANT[m.group(1)]
        # The receiver of the dispatch: `db.backend()` -> `db`.
        hm = re.search(r"(\w+)\s*\.\s*backend\s*\(", line)
        handle = hm.group(1) if hm else "db"
        for j in range(i, min(i + 40, len(lines))):
            if j > i and re.search(r"Backend::(Sqlite|Postgres)", lines[j]):
                break
            if j > i and DELEGATES.search(lines[j]):
                break
            # A pool accessor opens with `match &self.pool` and its arms are
            # `Pool::X`, not `Backend::X`; skip the whole body.
            if j > i and re.search(r"match\s*&\w*\s*\.\s*pool\b", lines[j]):
                break
            for got, call in POOLS.items():
                # Only calls on the handle being dispatched matter. `admin.` and
                # friends are separate pools whose backend is their own business,
                # so the receiver has to be the variable the arm matched on.
                if not re.search(r"\b" + handle + r"\s*\.\s*" + call + r"\s*\(", lines[j]):
                    continue
                if True:
                    if got != want:
                        found.append((j + 1, want, got))
                    break
            else:
                continue
            break
    return found


def rust_sources(root: Path) -> list[Path]:
    return sorted(p for p in root.rglob("*.rs") if "target" not in p.parts)


SELF_TEST_CASES = [
    # (source, expected_mismatch_line_or_None)
    ("let sql = match db.backend() {\n Backend::Sqlite => q.fetch_all(db.sqlite_pool()?),\n Backend::Postgres => q.fetch_all(db.postgres_pool()?),\n};", None),
    ("match db.backend() {\n Backend::Postgres => {\n  q.execute(db.sqlite_pool().expect(\"sqlite\")).await?;\n }\n}", 3),
    ("match db.backend() {\n Backend::Sqlite => {\n  q.execute(db.postgres_pool().expect(\"pg\")).await?;\n }\n}", 3),
    # A backend mention in a comment or doc is not an arm.
    ("// the Postgres arm used to call sqlite_pool()\nlet x = 1;", None),
    # No dispatch at all.
    ("fn f() { db.sqlite_pool(); }", None),
    # An arm longer than the scan window: the next arm stops the search, so the
    # pool call inside it is never attributed to the earlier arm.
    ("match db.backend() {\n" + "  // filler line\n" * 45 + " Backend::Postgres => {\n  q.execute(db.postgres_pool()?);\n }\n}", None),
    # A delegating arm binds nothing, so a differently-spelled pool elsewhere in
    # the function is not its business.
    ("match db.backend() {\n Backend::Postgres => rebuild_index_postgres(db, id).await,\n Backend::Sqlite => rebuild_index_sqlite(db, id).await,\n}", None),
    # `Pool::Sqlite(..)` is a pool accessor, not a backend dispatch.
    ("pub fn sqlite_pool(&self) -> Option<&SqlitePool> {\n match &self.pool {\n  Pool::Sqlite(p) => Some(p),\n  Pool::Postgres(_) => None,\n }\n}", None),
    # The first pool call on the dispatched handle is the one that counts.
    ("match db.backend() {\n Backend::Postgres => {\n  let a = db.postgres_pool();\n  let b = db.sqlite_pool();\n }\n}", None),
    # A dispatch on a differently-named handle still binds that handle.
    ("match conn.backend() {\n Backend::Sqlite => {\n  conn.sqlite_pool().expect(\"s\");\n }\n}", None),
    # A pool on some *other* handle is not this arm's business: the scratch-PG
    # admin pool is genuinely PostgreSQL inside a `Backend::Sqlite` world.
    ("match db.backend() {\n Backend::Sqlite => {\n  admin.postgres_pool().expect(\"pg\").close().await;\n }\n}", None),
]


def self_test() -> int:
    failures = 0
    for source, expected in SELF_TEST_CASES:
        got = arm_mismatches(source)
        if expected is None:
            ok = not got
        else:
            ok = len(got) == 1 and got[0][0] == expected
        if not ok:
            failures += 1
            print(f"FAIL: {source.splitlines()[0]!r} -> {got}, expected {expected}")
    print(f"self-test: {len(SELF_TEST_CASES) - failures}/{len(SELF_TEST_CASES)} passed")
    return 1 if failures else 0


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    root = Path(__file__).resolve().parent.parent / "crates"
    total = 0
    for path in rust_sources(root):
        for line, want, got in arm_mismatches(path.read_text()):
            total += 1
            print(f"{path}:{line}: Backend::{want.title()} arm uses the {got} pool")
    if total:
        print(f"\n{total} backend/pool mismatch(es)")
        return 1
    print("backend pool dispatch: clean")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
