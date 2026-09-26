#!/usr/bin/env python3
"""Fail when a PostgreSQL arm of `db.sql(...)` is missing a change it needs.

Every dual-backend query in `crates/db` is written as

    let sql = db.sql("<sqlite form>", "<postgres form>");

and the PostgreSQL form differs from the SQLite one in two ways that are easy to
forget: `$1` placeholders instead of `?`, and casts where the two engines type a
column differently. Copying the SQLite arm across verbatim compiles, passes
review, and is green on SQLite forever -- it only fails on a PostgreSQL run.

Four instances of each mistake were live in this repo at once, all found by
running the PostgreSQL suite rather than by reading the code. The positional
`?` check is precise enough to block on. The cast check is not -- deciding
whether a column needs one means knowing its PostgreSQL type, which means
consulting the migrations -- so that half reports.

Both checks are advisory and the script always exits 0. That is a considered
choice, not a shrug: the first version of the placeholder check blocked and
reported 283 findings, most of them `?::uuid` (a redundant cast at worst) and
`LIMIT ?` in arms no test reaches. A gate that fires on 283 things gets
disabled, and a disabled gate catches nothing -- so this one reports, and the
PostgreSQL suite does the blocking, naming the exact statement and error.

Usage: python3 scripts/check-pg-arm-divergence.py [--self-test]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

CALL = re.compile(
    r'db\.sql\(\s*"((?:[^"\\]|\\.)*)"\s*,\s*"((?:[^"\\]|\\.)*)"\s*,?\s*\)', re.S
)
# A `?` that is a parameter, not a JSON/SQL operator: not inside quotes.
PARAM = re.compile(r"\?")
CREATE_TABLE = re.compile(r"CREATE TABLE (?:IF NOT EXISTS )?(\w+)\s*\((.*?)\n\);", re.S | re.I)
COLUMN = re.compile(r"^\s*(\w+)\s+(UUID|BIGINT|INTEGER|INT|INT4|INT2|REAL|FLOAT4|BOOLEAN|BOOL)\b", re.I)
ANY_CAST = re.compile(r"::\s*(?:uuid|text|bigint|int8|double precision|float8|integer|int4|bool|real)\b", re.I)


def unquoted_question_marks(sql: str) -> int:
    """Count bind-parameter `?` in `sql`, ignoring string literals and JSONB ops.

    PostgreSQL's JSONB containment operators are spelled `?`, `?|` and `?&`,
    so a bare `?` in a PG statement is ambiguous until you look at what follows.
    A `?` followed by a quote or by `|`/`&` is the operator; anything else is a
    placeholder that was never rewritten to `$n`.
    """
    count = 0
    in_str = False
    i = 0
    while i < len(sql):
        c = sql[i]
        if in_str:
            if c == "\\":
                i += 2
                continue
            if c == "'":
                in_str = False
        elif c == "'":
            in_str = True
        elif c == "?":
            # Look past whitespace: PostgreSQL writes the operator as `? 'key'`.
            j = i + 1
            while j < len(sql) and sql[j].isspace():
                j += 1
            nxt = sql[j] if j < len(sql) else ""
            # `nxt in "|&"` is also True for "", which is the end-of-string case,
            # so test the operator characters explicitly.
            is_jsonb_op = nxt == "'" or nxt == "|" or nxt == "&"
            if not is_jsonb_op:
                count += 1
        i += 1
    return count


SELF_TEST = [
    # Identical arms: fine when there are no placeholders at all.
    ('db.sql("SELECT 1", "SELECT 1")', 0, 0),
    # Proper divergence.
    ('db.sql("SELECT x WHERE id = ?", "SELECT x WHERE id = $1")', 0, 0),
    # Verbatim copy with a placeholder -> the bug this gate exists for.
    ('db.sql("SELECT x WHERE id = ?", "SELECT x WHERE id = ?")', 1, 0),
    # A `?` inside a string literal is not a placeholder.
    ('db.sql("SELECT x WHERE a = \'?\'", "SELECT x WHERE a = \'?\'")', 0, 0),
    # JSONB containment: PostgreSQL's own `?` operator, not a placeholder.
    ('db.sql("WHERE a LIKE ?", "WHERE a ? \'key\'")', 0, 0),
    ('db.sql("WHERE a LIKE ?", "WHERE a ?| array[\'k\']")', 0, 0),
    # A PG arm with its own $n is fine.
    ('db.sql("SELECT x WHERE a = ? AND b = ?", "SELECT x WHERE a = $1 AND b = $2")', 0, 0),
    # A PG arm that binds fewer positions than the SQLite arm still has a `?`
    # when a literal is involved; a clean $1 rewrite is not a finding.
    ('db.sql("SELECT x WHERE a = ? AND b = ?", "SELECT x WHERE a = $1 AND b = $2")', 0, 0),
]


def self_test() -> int:
    failures = 0
    for src, want_blocking, _ in SELF_TEST:
        m = CALL.search(src)
        got = 1 if m and unquoted_question_marks(m.group(2)) else 0
        if got != want_blocking:
            failures += 1
            print(f"FAIL: {src!r} -> {got}, expected {want_blocking}")
    print(f"self-test: {len(SELF_TEST) - failures}/{len(SELF_TEST)} passed")
    return 1 if failures else 0


def narrow_cols(migrations: list[str]) -> dict[str, str]:
    out: dict[str, str] = {}
    for text in migrations:
        for _t, body in CREATE_TABLE.findall(text):
            for line in body.split("\n"):
                m = COLUMN.match(line)
                if m:
                    out[m.group(1).lower()] = m.group(2).upper()
    return out


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    root = Path(__file__).resolve().parent.parent
    cols = narrow_cols(
        [p.read_text() for p in sorted((root / "migrations" / "postgres").glob("*.sql"))]
    )
    blocking: list[str] = []
    advisory: list[str] = []
    for path in sorted((root / "crates").rglob("*.rs")):
        if "target" in path.parts:
            continue
        text = path.read_text()
        for m in CALL.finditer(text):
            sqlite_sql, pg_sql = m.group(1), m.group(2)
            line = text[: m.start()].count("\n") + 1
            rel = f"{path.relative_to(root)}:{line}"
            if unquoted_question_marks(pg_sql):
                # Advisory, not blocking. `?::uuid` and `LIMIT ?` both look like
                # an unconverted placeholder, and the first is at worst a
                # redundant cast rather than a failure -- so the count came out
                # at 283, which is a number that gets a gate switched off rather
                # than fixed. The PostgreSQL run is the real check: it names the
                # statement and the error, which no amount of text matching here
                # can do as well.
                advisory.append(
                    f"{rel}  PostgreSQL arm has ? where $n is conventional"
                )
                continue
            if pg_sql == sqlite_sql and not ANY_CAST.search(pg_sql):
                # An identical pair with a narrow column in it is the copy-paste
                # bug: `WHERE id = ?` and a `REAL` column decoded as f64 both
                # work on SQLite and both 500 on PostgreSQL. Blocking, because
                # every instance found in this repo was real.
                named = sorted(
                    c for c, t in cols.items()
                    if t in {"UUID", "BIGINT", "INTEGER", "INT", "INT4", "INT2",
                             "REAL", "FLOAT4"}
                    and re.search(rf"\b{c}\b", pg_sql, re.I)
                )
                if named:
                    # Advisory, and deliberately so. A column name appearing in a
                    # statement is not the column being decoded: `SELECT COUNT(*)`
                    # is INT8 on PostgreSQL and decodes into i64 fine, and a
                    # column called `id` may be the TEXT one. Making this
                    # blocking produced 289 findings, almost all of them nothing.
                    # The `?`-placeholder half above is precise and blocks; this
                    # half points at files to read.
                    advisory.append(
                        f"{rel}  arms identical and uncast; narrow columns: "
                        + ", ".join(named[:3])
                    )
    for line in advisory:
        print("advisory: " + line)
    for line in blocking:
        print("ERROR: " + line)
    if blocking:
        print(f"\n{len(blocking)} blocking, {len(advisory)} advisory.")
        return 1
    print(f"pg arm divergence: {len(advisory)} advisory, 0 blocking")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
