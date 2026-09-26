#!/usr/bin/env python3
"""Fail when a `Backend::Postgres` arm runs a query with SQLite `?` placeholders.

Most dual-backend code goes through `db.sql("<sqlite>", "<postgres>")` or
`crate::library::placeholders`, which pick the right form. The exception is a
hand-written `match db.backend()` where each arm calls `sqlx::query(..)` with
a literal. In the SQLite arm that literal has `?`; in the PostgreSQL arm it has
to have `$1`, `$2`, ...

It is easy to write the SQLite literal in both arms. It compiles, it is green on
SQLite, and on PostgreSQL

    WHERE entry_id = ? AND account_id = ?

is a syntax error at the `AND` -- so the endpoint returns 500 and the tests pass.
That exact bug shipped twice in this repo: the `vote_tx!` macro in
`crates/db/src/directory.rs`, and two arms in `settings.rs`.

The check pairs each `Backend::Postgres` arm with the `Backend::Sqlite` arm in
the same `match` and only fires when a query literal is unconverted. Arm
bounding is by brace depth, so a query cannot be attributed to the wrong arm.

Usage: python3 scripts/check-pg-backend-arm-placeholders.py [--self-test]
Exit 1 when a PostgreSQL arm has a `?` placeholder, 0 otherwise.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ARM = re.compile(r"Backend::(Sqlite|Postgres)\s*=>\s*\{")
QUERY = re.compile(r'sqlx::query(?:_as|_scalar)?\(\s*&?("(?:[^"\\]|\\.)*")')


def arm_bodies(text: str) -> list[tuple[str, int, str]]:
    """Return (arm_name, line_of_arm, body) for each `Backend::X => { ... }`."""
    out: list[tuple[str, int, str]] = []
    for m in ARM.finditer(text):
        start = m.end() - 1  # the '{'
        depth, i = 0, m.end() - 1
        while i < len(text):
            if text[i] == "{":
                depth += 1
            elif text[i] == "}":
                depth -= 1
                if depth == 0:
                    break
            i += 1
        body = text[start + 1 : i]
        out.append((m.group(1), text[: m.start()].count("\n") + 1, body))
    return out


def has_placeholder(lit: str) -> bool:
    """True when the literal still carries a `?` bind parameter.

    PostgreSQL's JSONB containment operators are also spelled `?`, `?|` and `?&`
    -- `WHERE tags ? 'genre'` -- so a `?` followed by a quote, a pipe or an
    ampersand is an operator rather than a bind. A `?` that ends the literal or
    the line is an ordinary placeholder, so `LIMIT ?` counts.
    """
    for i, ch in enumerate(lit):
        if ch != "?":
            continue
        j = i + 1
        while j < len(lit) and lit[j] == " ":
            j += 1
        if j < len(lit) and lit[j] in ("'", "|", "&"):
            continue
        return True
    return False


SELF_TEST = [
    # A proper pair: each arm has its own form.
    (
        "match db.backend() {\n"
        "  Backend::Sqlite => { sqlx::query(\"SELECT 1 WHERE a = ?\") }\n"
        "  Backend::Postgres => { sqlx::query(\"SELECT 1 WHERE a = $1\") }\n"
        "}",
        0,
    ),
    # The bug: the SQLite literal repeated in the PostgreSQL arm.
    (
        "match db.backend() {\n"
        "  Backend::Sqlite => { sqlx::query(\"SELECT 1 WHERE a = ?\") }\n"
        "  Backend::Postgres => { sqlx::query(\"SELECT 1 WHERE a = ?\") }\n"
        "}",
        1,
    ),
    # A JSONB `?` in the PostgreSQL arm is an operator, not a placeholder.
    (
        "match db.backend() {\n"
        "  Backend::Postgres => { sqlx::query(\"SELECT 1 WHERE tags ? 'k'\") }\n"
        "}",
        0,
    ),
    # No query in the PostgreSQL arm at all.
    (
        "match db.backend() {\n"
        "  Backend::Sqlite => { sqlx::query(\"SELECT 1 WHERE a = ?\") }\n"
        "  Backend::Postgres => { 0 }\n"
        "}",
        0,
    ),
    # A `$1::uuid` in a PostgreSQL arm is correct and not reported.
    (
        "match db.backend() {\n"
        "  Backend::Postgres => { sqlx::query(\"DELETE WHERE a = $1::uuid AND b = $2\") }\n"
        "}",
        0,
    ),
    # Two PostgreSQL arms, one bad.
    (
        "fn a() {\n"
        "  match db.backend() {\n"
        "    Backend::Postgres => { sqlx::query(\"SELECT 1\") }\n"
        "  }\n"
        "  match db.backend() {\n"
        "    Backend::Postgres => { sqlx::query(\"SELECT 1 WHERE a = ?\") }\n"
        "  }\n"
        "}",
        1,
    ),
]


def self_test() -> int:
    failures = 0
    for src, expected in SELF_TEST:
        got = 0
        for name, _line, body in arm_bodies(src):
            if name != "Postgres":
                continue
            for q in QUERY.finditer(body):
                if has_placeholder(q.group(1)):
                    got += 1
        if got != expected:
            failures += 1
            print(f"FAIL: expected {expected}, got {got} for:\n{src}")
    print(f"self-test: {len(SELF_TEST) - failures}/{len(SELF_TEST)} passed")
    return 1 if failures else 0


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    root = Path(__file__).resolve().parent.parent
    bad = 0
    for path in sorted((root / "crates").rglob("*.rs")):
        if "target" in path.parts:
            continue
        text = path.read_text()
        for name, line, body in arm_bodies(text):
            if name != "Postgres":
                continue
            for q in QUERY.finditer(body):
                if has_placeholder(q.group(1)):
                    bad += 1
                    print(
                        f"{path.relative_to(root)}:{line}  PostgreSQL arm has a `?` "
                        f"placeholder: {q.group(1)[:70]}"
                    )
    if bad:
        print(f"\n{bad} PostgreSQL arm(s) with SQLite placeholders -- use $1, $2.")
        return 1
    print("pg backend-arm placeholders: clean")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
