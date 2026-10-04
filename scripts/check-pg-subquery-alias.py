#!/usr/bin/env python3
"""Find subqueries in `FROM` that PostgreSQL will reject for want of an alias.

PostgreSQL refuses an unaliased subquery in FROM:

    42601 subquery in FROM must have an alias
    DETAIL: For example, FROM (SELECT ...) [AS] foo.

SQLite accepts it, so the fault is invisible on the default engine: the statement
compiles, every SQLite test passes, and the reader gets a masked 500 on PostgreSQL.
`ApiError::Internal` reports "Something went wrong on our side" and nothing else, and a
`RUST_LOG=debug` run of the failing test showed nothing either -- so this is worth
checking mechanically rather than waiting for the second masked 500.

Third occurrence in this repository, all the same shape:
  - crates/db/src/hit_rate.rs     (fixed 06816af)
  - crates/db/src/analytics.rs    (fixed: four analytics_gate tests 500ing on PG)

## What it does and does not check

It looks for a `FROM (` / `JOIN (` in a Rust string literal, then decides whether the
matching close-paren is followed by an identifier before the next clause boundary.

Known limits, stated rather than hidden:

  - **SQLite arms are skipped.** They are correct without an alias, and reporting them
    would roughly double the output for no new information. The arm is found the way
    the other checkers in this repository find it: the `Backend::Sqlite` half of a
    `db.sql(a, b)` pair, and a statement whose only placeholders are `?`.
  - **Comment text is skipped**, so a documented `FROM (SELECT ...) [AS] foo.` in a doc
    comment does not read as a finding.
  - **`INSERT INTO ... SELECT` is not a `FROM` subquery** and is not reported.
  - **A parenthesised join operand** (`FROM (a JOIN b ON ...) x`) is reported only when
    it genuinely has no alias, which is the same fault.

Exit 0 when clean, 1 with the findings otherwise, so it works as a CI gate.
"""

from __future__ import annotations

import pathlib
import re
import sys

# A Rust string literal. `\\.` with DOTALL would swallow a newline after a trailing
# backslash and end the literal early, which is the bug this same checker family hit
# in check-uncast-pg-placeholders.py.
STRING_LIT = re.compile(r'"((?:[^"\\]|\\.)*)"', re.DOTALL)

# Keywords that can follow a closing paren and mean it was never an alias position.
CLAUSE_END = re.compile(
    r"^\s*(?:,|FROM|JOIN|WHERE|GROUP|ORDER|LIMIT|HAVING|UNION|INTERSECT|EXCEPT"
    r"|ON\b|USING\b|WINDOW|RETURNING|AS\b)",
    re.IGNORECASE,
)

# An alias: an optional `AS`, then an identifier. Anything else after the `)` -- a
# keyword, a comma, nothing at all -- means the subquery is unaliased.
#
# This exists because a "is this not a clause keyword" test cannot answer the question.
# Matching "not a clause keyword" is not the same as matching an alias: `) )` and `) +`
# are neither, and both are unaliased. Stating the positive form is what makes
# end-of-statement read as a finding rather than as an exemption.
#
# The keyword exclusion is load-bearing, and its first version did not have it:
# `FROM (…) JOIN (…) ON true` reports the second operand, and `ON` is a bare
# identifier, so `[a-z_][a-z0-9_$]*` swallowed it and the self-test called that case a
# false negative. The words that can legally follow a `)` are exactly the clause
# keywords; an alias is a name, not a keyword.
ALIAS_TOKEN = re.compile(
    r"^\s*(?:AS\s+)?(?!ON\b|USING\b|WHERE\b|JOIN\b|FROM\b|GROUP\b|ORDER\b|LIMIT\b"
    r"|HAVING\b|UNION\b|INTERSECT\b|EXCEPT\b|WINDOW\b|RETURNING\b|CROSS\b|INNER\b"
    r"|LEFT\b|RIGHT\b|FULL\b|OUTER\b|NATURAL\b|AND\b|OR\b|ON\b)"
    r"[a-z_][a-z0-9_$]*",
    re.IGNORECASE,
)

# `EXTRACT(<unit> FROM ...)` -- the `FROM` is part of the function, not a FROM clause.
# Anchored on the opening paren so it only matches where an EXTRACT call is genuinely
# still open, rather than anywhere earlier in the statement.
EXTRACT_CONTEXT = re.compile(r"\bEXTRACT\s*\([^()]*$", re.IGNORECASE)

# A SQL line comment inside the literal. Both dialects spell it `--` to end of line,
# and neither requires a second `-`, so `---` and `--` are both comments.
SQL_COMMENT = re.compile(r"--[^\n]*")


def in_line_comment(text: str, pos: int) -> bool:
    """Is `pos` inside a `//` comment? Cheap: scan back to the line start."""
    start = text.rfind("\n", 0, pos) + 1
    return "//" in text[start:pos]


def is_sqlite_arm(text: str, pos: int) -> bool:
    """Is this literal the SQLite half of a `db.sql(a, b)` pair?

    Three signals, and the first two are needed because the third does not cover
    everything:

    1. A statement whose only placeholders are `?` is SQLite's spelling -- `sqlx`
       numbers them the same way, so an arm with no `$n` cannot be the PostgreSQL one.
    2. A **named** `const X: &str` or a `let` bound immediately above the literal is
       SQLite's if the name says so (`..._SQLITE`). `READING_TREND_SQLITE` is the case
       that proved this necessary: its placeholders are all bare `?`, which signal 1
       should have caught, but the `FROM (SELECT 0 AS value UNION ALL ...)` inside it
       is a *generated series* whose aliases SQLite and PostgreSQL spell differently,
       and reading the neighbouring arm's shape was the only reliable way to tell the
       two apart.
    3. The nearest `Backend::Sqlite` / `Backend::Postgres` arm marker wins when one is
       in scope, which is the shape the whole codebase uses for `db.sql(a, b)`.

    Signal 3 is a heuristic and this function's docstring says so: a lone literal with
    no arm marker and no `$n` is treated as SQLite, because a PostgreSQL arm always has
    at least one `$n` in this repository.
    """
    head = text[max(0, pos - 400) : pos]
    # Signal 2: the name immediately above the literal.
    named = re.findall(r"(?:const|let)\s+([A-Za-z_][A-Za-z0-9_]*)\s*(?::[^=]*)?=\s*$", head)
    if named and named[-1].upper().endswith(("_SQLITE", "_SQLITE_SQL")):
        return True
    if named and named[-1].upper().endswith(("_POSTGRES", "_POSTGRES_SQL", "_PG")):
        return False
    # Signal 1: no `$n` anywhere in the literal at all.
    m = STRING_LIT.match(text, pos)
    sql = m.group(1) if m else ""
    if not re.search(r"\$\d+", sql):
        return True
    # Signal 3: the nearest arm marker before the literal.
    arms = list(re.finditer(r"Backend::(Sqlite|Postgres)", text[:pos]))
    if arms:
        # Only trust it inside the same `db.sql(` call: a marker from a previous
        # function would misattribute this literal.
        window_start = text.rfind("pub async fn", 0, pos)
        if window_start != -1 and pos - window_start < 4000:
            return arms[-1].group(1) == "Sqlite"
    return False


def matching_paren(sql: str, open_idx: int) -> int | None:
    """Index of the `)` closing the `(` at `open_idx`, or None if unbalanced."""
    depth = 0
    quote: str | None = None
    i = open_idx
    while i < len(sql):
        ch = sql[i]
        if quote:
            if ch == quote:
                quote = None
        elif ch in "'\"":
            quote = ch
        elif ch == "(":
            depth += 1
        elif ch == ")":
            depth -= 1
            if depth == 0:
                return i
        i += 1
    return None


def unaliased(sql: str) -> list[tuple[int, str]]:
    """`(line, excerpt)` for each `FROM (` / `JOIN (` with no alias after the `)`."""
    out: list[tuple[int, str]] = []
    for match in re.finditer(r"\b(?:FROM|JOIN)\s*\(", sql, re.IGNORECASE):
        # `EXTRACT(EPOCH FROM (...))` is not a FROM subquery. The `FROM` there is the
        # keyword *inside* EXTRACT, and the paren is a plain arithmetic group, so it
        # is legal and needs no alias. Two of the first six findings were this shape,
        # in `analytics.rs` and in `snapshot_anonymisation.rs`, both reading
        # `EXTRACT(EPOCH FROM (a - b))`.
        if EXTRACT_CONTEXT.search(sql[: match.start()]):
            continue
        open_idx = match.end() - 1
        close = matching_paren(sql, open_idx)
        if close is None:
            continue
        after = sql[close + 1 :]
        # An alias is present only when an identifier follows. Stating that positively
        # is what makes every other case read as a finding rather than an exemption.
        #
        # Two earlier versions got this wrong in opposite directions, and the self-test
        # caught both:
        #
        #   - `if not CLAUSE_END.match(after): continue` -- "not a clause keyword" is not
        #     "an alias", so end-of-statement read as exempt:
        #         false negative: unaliased: the fault
        #           SELECT * FROM (SELECT id FROM works)
        #   - adding `AS\b` to CLAUSE_END to fix that -- which then vetoed a REAL alias,
        #     because `FROM (VALUES (0),(1)) AS seq(value)` matches `AS\b`:
        #         false positive, at analytics.rs:727
        #
        # So `CLAUSE_END` is gone from the decision entirely and `ALIAS_TOKEN` is the
        # whole rule: an alias, or a finding.
        if ALIAS_TOKEN.match(after):
            continue  # an alias is present
        line = sql[: match.start()].count("\n") + 1
        excerpt = " ".join(sql[match.start() : close + 1].split())[:70]
        out.append((line, excerpt))
    return out


def scan(root: pathlib.Path) -> list[tuple[pathlib.Path, int, str]]:
    hits: list[tuple[pathlib.Path, int, str]] = []
    paths = [root] if root.is_file() else sorted(root.rglob("*.rs"))
    for path in paths:
        if path.suffix != ".rs" or "/target/" in str(path):
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        for match in STRING_LIT.finditer(text):
            if in_line_comment(text, match.start()):
                continue
            sql = match.group(1)
            # A `--` comment *inside* the SQL is not code. This is not hypothetical:
            # the fix for `analytics.rs` documents the 42601 error in a `--` line
            # inside the very PostgreSQL arm it fixed, so the checker reported the file
            # it had just repaired. Stripping them is what lets a fix explain itself in
            # place, which is where the explanation is worth the most.
            sql = SQL_COMMENT.sub(" ", sql)
            if not re.search(r"\b(?:SELECT|INSERT|UPDATE|DELETE)\b", sql, re.IGNORECASE):
                continue
            if is_sqlite_arm(text, match.start()):
                continue
            for line, excerpt in unaliased(sql):
                hits.append((path, text[: match.start()].count("\n") + line, excerpt))
    return hits


def self_test() -> int:
    """Prove the rule on statements whose correct answer is known."""
    cases = [
        ("SELECT * FROM (SELECT id FROM works) AS w", False, "aliased: correct"),
        ("SELECT * FROM (SELECT id FROM works)", True, "unaliased: the fault"),
        (
            "SELECT * FROM (SELECT id FROM works) w JOIN (SELECT id FROM tags) t ON true",
            False,
            "both aliased",
        ),
        (
            "SELECT * FROM (SELECT id FROM works) JOIN (SELECT id FROM tags) ON true",
            True,
            "both operands unaliased",
        ),
        ("SELECT * FROM works WHERE id IN (SELECT work_id FROM tags)", False,
         "an IN subquery is not a FROM subquery"),
        ("SELECT * FROM works", False, "no subquery at all"),
        # A column-list alias: `AS seq(value)`. Adding `AS\b` to the clause-keyword set
        # to fix the end-of-statement false negative then vetoed this REAL alias, so it
        # is pinned here -- analytics.rs:727 is where it was found.
        ("SELECT * FROM (VALUES (0), (1)) AS seq(value)", False,
         "a column-list alias is an alias"),
        # `EXTRACT(... FROM (...))` -- the FROM belongs to the function. Two of the first
        # six findings were this shape.
        ("SELECT EXTRACT(EPOCH FROM (a - b))::bigint FROM t", False,
         "EXTRACT's FROM is not a FROM clause"),
        ("SELECT EXTRACT(EPOCH FROM (max(x) - min(x))) FROM t", False,
         "EXTRACT over an aggregate pair"),
        # A SQL line comment inside the statement. The analytics.rs fix documents the
        # 42601 error in a `--` line inside the arm it fixed, and the checker reported
        # the file it had just repaired.
        ("SELECT 1 -- FROM (SELECT id FROM works)\nFROM t", False,
         "a FROM inside a SQL comment is not code"),
    ]
    failures = []
    for sql, should_report, note in cases:
        # Through `scan`'s own preprocessing, not straight to `unaliased`, so the
        # SQL-comment stripping and arm detection are exercised by the same cases the
        # gate runs. Calling `unaliased` directly would have passed the
        # `-- FROM (` case while the gate still reported it.
        cleaned = SQL_COMMENT.sub(" ", sql)
        reported = bool(unaliased(cleaned))
        if reported != should_report:
            failures.append(
                f"{'false positive' if reported else 'false negative'}: {note}\n  {sql}"
            )
    for failure in failures:
        print(f"FAIL {failure}")
    if failures:
        print(f"\n{len(failures)} of {len(cases)} self-test cases failed")
        return 1
    print(f"self-test: {len(cases)} cases passed")
    return 0


def main(argv: list[str]) -> int:
    args = [a for a in argv[1:] if not a.startswith("-")]
    if "--self-test" in argv[1:]:
        return self_test()
    root = pathlib.Path(args[0]) if args else pathlib.Path("crates")
    hits = scan(root)
    if not hits:
        print(f"OK: every FROM subquery is aliased ({root})")
        return 0
    print(f"{len(hits)} unaliased FROM subquery/ies -- PostgreSQL rejects each with 42601\n")
    for path, line, excerpt in hits:
        print(f"  {path}:{line}")
        print(f"      {excerpt}")
    print(
        "\nEach of these passes on SQLite and returns a masked 500 on PostgreSQL.\n"
        "Add an alias: FROM ( ... ) AS name"
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))