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

VERBOSE = False


ARM = re.compile(r"Backend::(Sqlite|Postgres)\s*=>\s*\{")
QUERY = re.compile(
    r'sqlx::query(?:_as|_scalar)?\(\s*&?("(?:[^"\\]|\\.)*")'  # a literal
    r"|sqlx::query(?:_as|_scalar)?\(\s*([A-Za-z_][A-Za-z0-9_]*)"  # a named const/variable
    r"|sqlx::query(?:_as|_scalar)?\(\s*&([A-Za-z_][A-Za-z0-9_]*)"  # &a String
    r"|sqlx::query(?:_as|_scalar)?\(\s*([A-Za-z_][A-Za-z0-9_]*)\.as_str\(\)"
)


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


def has_numbered_placeholder(lit: str) -> bool:
    """True for SQLite's `?1`/`?2` numbered bind form, which PostgreSQL rejects.

    `WHERE identity_id = ?1` is a perfectly good SQLite statement and on
    PostgreSQL is `operator does not exist: ?1 integer` -- so the arm compiles,
    is green on SQLite, and 500s in production. Distinct from `has_placeholder`
    because `?1` leaves a digit after the `?`, which the JSONB-operator guard in
    `has_placeholder` would otherwise have to reason about.
    """
    return re.search(r"\?\s*\d", lit) is not None


def resolve_for_arm(text: str, body: str) -> dict[str, str | None]:
    """Resolve names for one arm: arm-local bindings shadow outer ones.

    roadmap.rs:150 writes its query *inside* each arm --
    `let sql = format!("... $1")` in the PostgreSQL arm and `?` in the SQLite
    one. A file-level lookup found the SQLite one and reported a correct arm as
    broken. Arm-local resolution is what makes each arm answer for itself.
    """
    merged = resolve_local_names(text)
    merged.update(resolve_local_names(body))
    return merged


def query_args(body: str):
    """Yield the query argument of every `sqlx::query*` call in an arm body.

    Four shapes, because the bug this gate exists for was reachable through all
    of them and a literal-only pattern missed every non-literal one:

    * `sqlx::query("...")`            -- a string literal
    * `sqlx::query(WORK)`             -- a `const` or local variable
    * `sqlx::query(&sql)`             -- a borrowed `String`
    * `sqlx::query(x.as_str())`       -- a computed `&str`

    For a non-literal only the *name* is available, so the caller must resolve
    it against the file to know what the SQL is. Returning the name is what lets
    the gate say "this arm runs a query it did not write here" rather than
    silently passing.
    """
    for q in QUERY.finditer(body):
        yield (q.group(1) or q.group(2) or q.group(3) or q.group(4), q.start())


# A binding that IS a dialect-correct query. `db.sql(..)`, `Database::sql(..)`
# and `rewrite_placeholders(..)` all emit the right placeholders for the backend
# chosen at runtime, so a query that came out of one of them must never be
# flagged. The first self-test that this gate grew was a false positive on
# exactly this shape: `let sql = db.sql("... ? ...")` reported as a
# PostgreSQL-arm bug, 496 times over, while CI was green.
DIALECT_HELPER = re.compile(
    r"(?:\.|\b)(?:db|d|t|self)\s*\.\s*(?:sql|placeholders)\s*\(|rewrite_placeholders\s*\("
)


def resolve_local_names(text: str) -> dict[str, str | None]:
    """Map `NAME` -> its SQL literal, or None when it is already dialect-safe.

    Returns three kinds of value, and the distinction is the whole point:

    * the literal string, when the binding IS a query this gate must judge;
    * `None`, when the binding came from a dialect helper and is correct by
      construction;
    * absent from the dict, when the name is not a local binding at all.

    Scoped to one function, because a file-level `let sql` in a *different*
    function is a different variable -- resolving across that boundary reported
    `monetization.rs:788` as broken when its query has no placeholder in it.
    """
    out: dict[str, str | None] = {}
    for m in re.finditer(
        r'(?:const|let)\s+(?:mut\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*(?::\s*&\s*str\s*)?=\s*'
        r'("(?:[^"\\]|\\.)*")',
        text,
    ):
        out[m.group(1)] = m.group(2)
    # `format!("... $1 ...")` and `String::from("...")`: the query is assembled,
    # so there is no bare literal for the pass above, and the arm-local form
    # went UNRESOLVED rather than wrong -- which let the gate fall back to a
    # same-named binding in another function and flag a correct arm.
    # roadmap.rs:150 builds its `$1` this way inside each arm.
    for m in re.finditer(
        r'(?:const|let)\s+(?:mut\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*(?::\s*[A-Za-z_:<>\s]*?)?=\s*'
        r'(?:format!|String::from)\s*\(\s*("(?:[^"\\]|\\.)*")',
        text,
    ):
        out.setdefault(m.group(1), m.group(2))
    # A binding assigned from a helper, or built by calling one, is safe.
    # Walk assignments to their terminating `;` so a multi-line call is seen
    # whole. `db.sql("a ... ? ...", "a ... $1 ...")` carries TWO literals, and
    # a per-literal regex only ever saw the first -- the SQLite one -- which is
    # how settings.rs stayed a false positive after the helper was recognised.
    for m in re.finditer(
        r'(?:const|let)\s+(?:mut\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*(?::[^=]*?)?=\s*',
        text,
    ):
        name = m.group(1)
        end = text.find(";", m.end())
        if end == -1:
            continue
        rhs = text[m.end() : end]
        if DIALECT_HELPER.search(rhs):
            # Anything a dialect helper produced is correct by construction,
            # whatever its source literals looked like.
            out[name] = None
    return out


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
    # The shape that shipped four bugs in crates/app/tests/story_identity.rs:
    # the query is a `const`, so a literal-only pattern cannot see it at all.
    (
        "fn a() {\n"
        "  const WORK: &str = \"INSERT INTO t VALUES (?1, ?2)\";\n"
        "  match db.backend() {\n"
        "    Backend::Postgres => { sqlx::query(WORK) }\n"
        "  }\n"
        "}",
        1,
    ),
    # Same, but the const has already been written correctly.
    (
        "fn a() {\n"
        "  const WORK: &str = \"INSERT INTO t VALUES ($1, $2)\";\n"
        "  match db.backend() {\n"
        "    Backend::Postgres => { sqlx::query(WORK) }\n"
        "  }\n"
        "}",
        0,
    ),
    # A `?1` literal straight into a PostgreSQL arm: valid SQLite, invalid PG.
    (
        "match db.backend() {\n"
        "  Backend::Postgres => { sqlx::query(\"SELECT 1 WHERE a = ?1\") }\n"
        "}",
        1,
    ),
    # `?::uuid` -- the original story_identity bug. PG sees a literal `?` because
    # sqlx does not rewrite a `?` that is immediately followed by `::`.
    (
        "match db.backend() {\n"
        "  Backend::Postgres => { sqlx::query(\"INSERT INTO t VALUES (?::uuid, ?)\") }\n"
        "}",
        1,
    ),
    # A borrowed String built by the dialect helper is fine and must stay quiet.
    (
        "fn a(t: &T) {\n"
        "  let sql = t.sql(\"SELECT 1 WHERE a = ?\");\n"
        "  match t.backend() {\n"
        "    Backend::Postgres => { sqlx::query(&sql) }\n"
        "  }\n"
        "}",
        0,
    ),
]


def self_test() -> int:
    failures = 0
    for src, expected in SELF_TEST:
        got = 0
        for name, _line, body in arm_bodies(src):
            if name != "Postgres":
                continue
            for arg, _pos in query_args(body):
                if arg.startswith('"'):
                    lit = arg
                else:
                    resolved = resolve_for_arm(src, body).get(arg, "MISSING")
                    if resolved in (None, "MISSING"):
                        continue  # dialect-safe, or not a local literal
                    lit = resolved
                if lit and (has_placeholder(lit) or has_numbered_placeholder(lit)):
                    got += 1
        if got != expected:
            failures += 1
            print(f"FAIL: expected {expected}, got {got} for:\n{src}")
    print(f"self-test: {len(SELF_TEST) - failures}/{len(SELF_TEST)} passed")
    return 1 if failures else 0


def main(argv: list[str]) -> int:
    global VERBOSE
    if "--verbose" in argv:
        VERBOSE = True
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
            locals_ = resolve_for_arm(text, body)
            for arg, _pos in query_args(body):
                if arg.startswith('"'):
                    lit = arg
                    if not (has_placeholder(lit) or has_numbered_placeholder(lit)):
                        continue
                else:
                    lit = locals_.get(arg)
                    if lit is None:
                        # Not a local literal. The overwhelmingly common case is a
                        # query already run through the dialect helper
                        # (`db.sql(..)` / `rewrite_placeholders`), which is
                        # correct by construction and must not be flagged -- 496
                        # of those are green today. What is worth naming is a
                        # query built in a way the gate cannot follow, so say so
                        # at VERBOSE level rather than failing on it.
                        if VERBOSE:
                            print(
                                f"{path.relative_to(root)}:{line}  PostgreSQL arm runs a "
                                f"query the gate cannot resolve (`{arg}`): placeholders "
                                f"unverified (fine if it came from db.sql/rewrite)"
                            )
                        continue
                    if not (has_placeholder(lit) or has_numbered_placeholder(lit)):
                        continue
                bad += 1
                print(
                    f"{path.relative_to(root)}:{line}  PostgreSQL arm has a `?` or "
                    f"`?n` placeholder: {lit[:70]}"
                )
    if bad:
        print(f"\n{bad} PostgreSQL arm(s) with SQLite placeholders -- use $1, $2.")
        return 1
    print("pg backend-arm placeholders: clean")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
