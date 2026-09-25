#!/usr/bin/env python3
"""Find PostgreSQL statements that bind text to a column the dialect types.

`Database::sql` rewrites `?` to `$n` but never casts, and sqlx sends a bound
`&str` as `text`. So a statement that reads

    SELECT dimension_key FROM arena_weights WHERE account_id = $1

works on SQLite and fails on PostgreSQL with

    42804 column "account_id" is of type uuid but expression is of type text

or, where the statement does cast the placeholder and the *column* is read into
a String instead, with a decode error:

    ColumnDecode { source: "mismatched types; Rust type `String`
                   (as SQL type `TEXT`) is not compatible with SQL type `UUID` }

Both are the same underlying mistake, and both are invisible on SQLite -- which
is how 62 of them shipped. The gate that catches this is running the suite on
both dialects; this script is the cheap pre-flight that says where to look.

The verdict comes from the migrations, not from a hand-kept list of tables. An
earlier version carried a `UUID_KEYED_TABLES` set that had already drifted: it
claimed `admin_actions` and `listings` were uuid-keyed when both are TEXT in
`migrations/postgres/`, so it reported statements that were never wrong. A
list of tables cannot stay right; parsing the schema can. Types are read out of
`migrations/postgres/*.sql`, so a new migration that retypes a column is picked
up on the next run with no edit here.

A column is reported when it is bound or compared to a placeholder and its
PostgreSQL type is not one that a bound `&str` can satisfy: that is, not text,
varchar, char, or any other string-typed column.

Usage:  check-uncast-pg-placeholders.py [crates/...]

Options: --migrations DIR   where the PostgreSQL DDL lives
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys
from collections import defaultdict

# Column types a bound `&str` cannot satisfy by accident, and which therefore
# have to be named in the statement. A TIMESTAMPTZ column is here because the
# codebase stores timestamps as RFC 3339 strings and binds them as text, which
# PostgreSQL will not coerce: the statement has to say `::timestamptz`. Numeric
# and boolean types are deliberately absent -- a test may well be binding a real
# `i64`/`bool` there, and the type alone cannot tell, so those are settled by
# running the suite on both dialects rather than by guessing.
NEEDS_EXPLICIT_CAST = {
    "uuid", "timestamptz", "timestamp", "timestamp with time zone",
    "timestamp without time zone", "date", "json", "jsonb", "uuid[]",
}

# `CREATE TABLE x (` ... `);` in a migration, then `col  TYPE ...` lines.
CREATE_TABLE = re.compile(
    r"CREATE\s+TABLE\s+(?:IF\s+NOT\s+EXISTS\s+)?([a-z_][a-z0-9_]*)\s*\((.*?)\n\);",
    re.IGNORECASE | re.DOTALL,
)
COLUMN = re.compile(
    r"^\s*([a-z_][a-z0-9_]*)\s+([A-Za-z][A-Za-z0-9_ ]*?(?:\([^)]*\))?)"
    r"(?:\s+(?:NOT\s+NULL|NULL|DEFAULT\b[^,]*|PRIMARY\s+KEY|UNIQUE|REFERENCES\b.*?))*\s*,",
    re.IGNORECASE | re.MULTILINE,
)
# The final column of a CREATE TABLE has no trailing comma, so COLUMN cannot
# match it. Take it separately: same shape, comma optional only at the end, and
# anchored so it cannot re-read a line COLUMN already handled.
LAST_COLUMN = re.compile(
    r"^\s*([a-z_][a-z0-9_]*)\s+([A-Za-z][A-Za-z0-9_ ]*?(?:\([^)]*\))?)"
    r"(?:\s+(?:NOT\s+NULL|NULL|DEFAULT\b[^,]*|PRIMARY\s+KEY|UNIQUE|REFERENCES\b.*))?\s*$",
    re.IGNORECASE | re.MULTILINE,
)


def load_schema(migrations: pathlib.Path) -> dict[str, dict[str, str]]:
    """table -> {column: postgres type}, read from the PostgreSQL migrations."""
    schema: dict[str, dict[str, str]] = {}
    for path in sorted(migrations.glob("*.sql")):
        text = path.read_text(encoding="utf-8", errors="replace")
        for match in CREATE_TABLE.finditer(text):
            table, body = match.group(1).lower(), match.group(2)
            columns = schema.setdefault(table, {})
            found = list(COLUMN.finditer(body)) + list(LAST_COLUMN.finditer(body))
            for col in found:
                # The trailing constraint group is non-greedy, so on
                # `uuid NOT NULL` the captured type can still carry it.
                coltype = " ".join(col.group(2).split()).lower()
                for noise in ("not null", "null", "primary key", "unique"):
                    coltype = re.sub(rf"\b{noise}\b.*", "", coltype).strip()
                # Last writer wins. A table is often declared once and then
                # redeclared by a later migration that retypes columns, and the
                # schema the database actually has is the last one applied.
                columns[col.group(1).lower()] = coltype
    return schema


def tables_in(sql: str) -> set[str]:
    lowered = sql.lower()
    return {
        t for t in re.findall(r"\b(?:from|join|into|update)\s+([a-z_][a-z0-9_]*)", lowered)
    }


# `col = $n` / `col IN ($n...)` / `col <op> $n` -- the comparison form.
# A column compared to a placeholder. The trailing negative lookahead is the
# point: `$1::uuid` is already cast and cannot fail, and without this the check
# fired on every statement that spelled a cast column in the SELECT list --
# `SELECT id::text ... WHERE id = $1::uuid` is correct and was reported anyway.
COMPARED = re.compile(
    r"\b([a-z_][a-z0-9_.]*)\s*(?:=|<>|!=|>|<|like|ilike|in)\s*\$\d+(?!\s*::)",
    re.IGNORECASE,
)
# The INSERT form. `\s*` spans the newline, because these statements are often
# written with the column list on its own lines and a naive `VALUES (` match
# misses the wrapped ones.
VALUES_START = re.compile(r"VALUES\s*\(", re.IGNORECASE)
# Anchored at a comma or the start, not at `^` with MULTILINE: `^` under
# MULTILINE matches only a line start, so this returned the first column
# and nothing else, which silently disabled the whole INSERT branch.
INSERT_COLUMN = re.compile(
    r"(?:^|,)\s*([a-z_][a-z0-9_]*)\s*(?:\([^)]*\))?\s*(?=,|$)", re.IGNORECASE
)

STRING_LIT = re.compile(r'"((?:[^"\\]|\\.)*)"', re.DOTALL)


def referenced_columns(sql: str) -> set[str]:
    """Every column the statement names, so we can ask the schema about it."""
    cols = {m.group(1).split(".")[-1].lower() for m in COMPARED.finditer(sql)}
    cols |= {c.lower() for c in re.findall(r"\bSELECT\b(.*?)\bFROM\b", sql, re.IGNORECASE | re.DOTALL)
             for c in re.findall(r"[a-z_][a-z0-9_.]*", c)}
    return cols


def insert_column_order(sql: str) -> list[str]:
    """The column list of an INSERT, in order, so binds line up positionally.

    Finds the *last* top-level `(` before the first top-level `VALUES`, not the
    first `(` in the statement: `INSERT INTO t (a, b) SELECT ...` and a
    function call in the target list both put an earlier paren there. The
    original used `split("VALUES", 1)` and `rindex("(")`, which returned one
    column for every INSERT -- so the INSERT branch of this script never fired,
    and a statement like
    `INSERT INTO user_devices (..., last_seen_at, created_at, updated_at)
     VALUES (?::uuid, ?::uuid, ?, ?, ?, ?, ?)`
    passed while binding three text values into TIMESTAMPTZ columns.
    """
    vm = VALUES_START.search(sql)
    if not vm:
        return []
    before = sql[: vm.start()]
    if "(" not in before:
        return []
    inner = before[before.rindex("(") + 1 :]
    # Cut at the list's own closing paren. Left in place, the trailing `)`
    # defeats the `(?=,|$)` lookahead and the last column -- usually
    # `updated_at` -- was silently dropped.
    inner = inner[: inner.rindex(")")] if ")" in inner else inner
    return [m.group(1).lower() for m in INSERT_COLUMN.finditer(inner)]


def offending_lines(sql: str, schema: dict[str, dict[str, str]]) -> bool:
    """True when this statement hands text to a column the dialect types.

    Note there is no `if "::" in sql: return False` guard here any more. It read
    as reasonable -- "this statement already casts, so it is not a bare site" --
    and it hid a real defect: `sessions::create_session` casts three UUID binds
    and left three TIMESTAMPTZ binds bare in the same statement, so the whole
    statement was skipped and the check reported OK. One cast anywhere in a
    statement said nothing about the others. The per-placeholder check below is
    what the guard was standing in for, badly.
    """
    # A SUM over an integer column is wrong with or without a bind, so it is
    # checked before the placeholder test rather than after it.
    if sum_sites(sql, schema):
        return True
    if int4_sites(sql, schema):
        return True

    if not re.search(r"\$\d+", sql):
        # `?` is the SQLite spelling. The PostgreSQL arm is the one rewritten to
        # `$n`, so without a placeholder this is a SQLite arm and cannot fail.
        return False

    tables = tables_in(sql)
    known = {t: schema[t] for t in tables if t in schema}
    if not known:
        return False

    # Comparison form: a non-textual column compared to a placeholder.
    for match in COMPARED.finditer(sql):
        bare = match.group(1).split(".")[-1].lower()
        for columns in known.values():
            coltype = columns.get(bare)
            if coltype and not _accepts_text(coltype):
                return True

    # INSERT form: a non-textual column whose VALUES position is a bare bind.
    order = insert_column_order(sql)
    values = values_expressions(sql)
    if order and values and len(order) == len(values):
        for columns in known.values():
            for col, expr in zip(order, values):
                coltype = columns.get(col)
                if coltype and not _accepts_text(coltype) and is_bare_bind(expr):
                    return True

    # SET form: `SET col = $n`, which neither branch above covers.
    for match in re.finditer(
        r"(?:^|,|\bSET\s)\s*([a-z_][a-z0-9_]*)\s*=\s*\$(\d+)(?!\s*::)", sql, re.IGNORECASE
    ):
        col = match.group(1).lower()
        for columns in known.values():
            coltype = columns.get(col)
            if coltype and not _accepts_text(coltype):
                return True

    # `SUM` over an integer column is NUMERIC in PostgreSQL and an integer in
    # SQLite. Reported here as well as in `scan` so the self-test and the gate
    # cannot disagree about what counts -- the self-test is worth nothing if it
    # exercises a different predicate than the gate does.
    if sum_sites(sql, known):
        return True

    return False


def is_bare_bind(expr: str) -> bool:
    """True when a VALUES expression is a plain placeholder needing a cast."""
    return bool(re.fullmatch(r"\$\d+", expr.strip()))


def values_expressions(sql: str) -> list[str]:
    """The top-level expressions in the INSERT's VALUES list, in order.

    Split on commas at depth zero so a function call like `date_trunc('day', ?)`
    stays one expression -- which matters, because a comma inside it is not a
    column boundary and treating it as one shifts every later position.
    """
    vm = VALUES_START.search(sql)
    if not vm:
        return []
    i = vm.end()  # just past the opening paren
    depth = 1
    out: list[str] = []
    start = i
    quote: str | None = None
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
                out.append(sql[start:i])
                return out
        elif ch == "," and depth == 1:
            out.append(sql[start:i])
            start = i + 1
        i += 1
    return out


def add_missing_casts(sql: str, known: dict[str, dict[str, str]]) -> str:
    """Return `sql` with a cast added to every placeholder that needs one.

    Split out from the reporting so the fix and the check read the same rules.
    Only placeholders proven bare get a cast, and each is cast to the type its
    own column was declared with -- an `::text` here would be the defect the
    checker exists to catch, not a fix for it.

    Edits are collected and applied last, right to left. Mutating the string
    while iterating its matches shifts every later offset, and the result was
    `work$2::uuidd` -- a column name cut in half by a cast landing inside it.
    """
    edits: list[tuple[int, int, str]] = []
    for match in re.finditer(
        r"([a-z_][a-z0-9_]*)\s*(?:=|<>|!=|>|<|like|ilike|in)\s*(\$\d+)(?!\s*::)",
        sql,
        re.IGNORECASE,
    ):
        col, placeholder = match.group(1).lower(), match.group(2)
        for columns in known.values():
            cast = _cast_needed(columns.get(col))
            if cast:
                edits.append((match.start(2), match.end(2), f"{placeholder}::{cast}"))
                break

    # INSERT: pair each column with its VALUES position and cast the bare ones.
    # Walks the real expression spans so a comma inside `date_trunc('day', ?)`
    # is not mistaken for a column boundary.
    order = insert_column_order(sql)
    values = values_expressions(sql)
    if order and values and len(order) == len(values):
        vm = VALUES_START.search(sql)
        assert vm is not None
        for col, expr in zip(order, values):
            if not is_bare_bind(expr):
                continue
            cast = None
            for columns in known.values():
                cast = _cast_needed(columns.get(col))
                if cast:
                    break
            if cast:
                bind = expr.strip()
                at = sql.index(bind, vm.end()) if bind in sql[vm.end():] else -1
                if at >= 0:
                    edits.append((at, at + len(bind), f"{bind}::{cast}"))

    # A SUM over an integer column needs widening, and no placeholder is involved.
    # `CAST(SUM(x) AS BIGINT)` rather than `SUM(x)::bigint`: the `::` form is
    # PostgreSQL only, and many of these statements are spelled once and run on
    # both backends. `CAST` is the one form both dialects accept.
    #
    # The whole `SUM(...)` call is wrapped, matched to its closing paren. An
    # earlier attempt appended after the call and produced
    # `SUM(x CAST(... AS BIGINT))`, which is not valid SQL in either dialect.
    for col in sum_sites(sql, known):
        for match in re.finditer(
            rf"SUM\(\s*(?:[a-z_][\w]*\s*\.\s*)?{re.escape(col)}\b", sql, re.IGNORECASE
        ):
            start = match.start()
            # Scan from the `(` itself. Starting one character earlier -- at the
            # last letter of the column name -- never reaches depth zero, so the
            # fix silently did nothing while looking like it had run.
            open_paren = sql.index("(", match.start())
            depth = 0
            end = None
            for i in range(open_paren, len(sql)):
                if sql[i] == "(":
                    depth += 1
                elif sql[i] == ")":
                    depth -= 1
                    if depth == 0:
                        end = i + 1
                        break
            if end is not None and "::" not in sql[end:end + 2]:
                edits.append((start, end, f"CAST({sql[start:end]} AS BIGINT)"))

    # Widen a bare INT4/INT2 column in the SELECT list the same way. CAST, so
    # the result is valid on SQLite too and the statement need not be split.
    head_end = re.search(r"\bFROM\b", sql, re.IGNORECASE)
    for col in int4_sites(sql, known):
        for match in re.finditer(
            rf"(?<![:\w])((?:[a-z_][\w]*\s*\.\s*)?){re.escape(col)}\b(?!\s*::|\s*\()",
            sql[: head_end.start()] if head_end else sql,
            re.IGNORECASE,
        ):
            edits.append((match.start(1), match.end(), f"CAST({match.group(0)} AS BIGINT)"))

    out = sql
    for start, end, replacement in sorted(edits, reverse=True):
        out = out[:start] + replacement + out[end:]
    return out


def _cast_needed(coltype: str | None) -> str | None:
    """The cast a column of this declared type needs, or None if it needs none."""
    if not coltype or _accepts_text(coltype):
        return None
    return PG_CAST.get(coltype.strip().lower())


# The PostgreSQL type to cast a bind to, keyed by the type the schema declares.
PG_CAST = {
    "uuid": "uuid",
    "integer": "int4",
    "bigint": "int8",
    "smallint": "int2",
    "double precision": "float8",
    "real": "float4",
    "boolean": "bool",
    "numeric": "numeric",
    "date": "date",
    "timestamptz": "timestamptz",
    "timestamp": "timestamp",
    "bytea": "bytea",
    "jsonb": "jsonb",
    "json": "json",
}


def _accepts_text(coltype: str) -> bool:
    base = coltype.split("(")[0].strip()
    return base not in NEEDS_EXPLICIT_CAST


def scan(root: pathlib.Path, schema: dict[str, dict[str, str]]) -> list[tuple[pathlib.Path, int, str]]:
    hits: list[tuple[pathlib.Path, int, str]] = []
    # Accept a file as well as a directory, so a single module can be checked
    # while working on it.
    paths = [root] if root.is_file() else sorted(root.rglob("*.rs"))
    for path in paths:
        if path.suffix != ".rs" or "/target/" in str(path):
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        for match in STRING_LIT.finditer(text):
            sql = match.group(1)
            if not re.search(r"\b(?:SELECT|INSERT|UPDATE|DELETE)\b", sql, re.IGNORECASE):
                continue
            tables = tables_in(sql)
            known = {t: schema[t] for t in tables if t in schema}
            reasons: list[str] = []
            if offending_lines(sql, known):
                reasons.append("uncast placeholder")
            summed = sum_sites(sql, known) if known else []
            if summed:
                reasons.append("SUM(" + ", SUM(".join(summed) + ") is NUMERIC")
            narrow = int4_sites(sql, known) if known else []
            if narrow:
                reasons.append("INT4 column into i64: " + ", ".join(narrow))
            if not reasons:
                continue
            line = text[: match.start()].count("\n") + 1
            flat = " ".join(sql.split())[:78]
            hits.append((path, line, f"{'; '.join(reasons)}: {flat}"))
    return hits


SCHEMA = {
    "works": {"id": "uuid", "updated_at": "text", "title": "text"},
    "roadmap_cards": {"id": "text", "elo_rating": "double precision",
                      "updated_at": "timestamptz"},
    "progress": {"account": "uuid", "work_id": "uuid", "last_chapter": "integer",
                 "updated_at": "text"},
}

# Each case is (SQL, should_report, note). The negative cases are the point:
# a checker that flags correct SQL gets muted, and then it flags nothing.
SELF_TEST_CASES: list[tuple[str, bool, str]] = [
    ("SELECT title FROM works WHERE id = $1", True,
     "a UUID column with a bare bind"),
    ("SELECT title FROM works WHERE id = $1::uuid", False,
     "already cast"),
    ("UPDATE works SET updated_at = $1 WHERE id = $2", True,
     "the timestamp needs a cast but the id is bare"),
    ("UPDATE works SET updated_at = $1 WHERE id = $2::uuid", False,
     "both handled"),
    ("SELECT title FROM works WHERE id = $1::uuid -- ::", False,
     "a cast elsewhere must not decide this statement"),
    ("UPDATE roadmap_cards SET elo_rating = $1, updated_at = $2 WHERE id = $3",
     True, "float8 and timestamptz both need casts"),
    ("INSERT INTO progress (account, work_id, last_chapter, updated_at) "
     "VALUES ($1, $2, $3, $4)", True,
     "INSERT: only the typed columns get a cast"),
    ("INSERT INTO progress (account, work_id, last_chapter, updated_at) "
     "VALUES ($1::uuid, $2::uuid, $3, $4)", False,
     "INSERT: already cast"),
    ("SELECT COALESCE(SUM(last_chapter), 0) AS total FROM progress", True,
     "SUM over INTEGER is NUMERIC in PostgreSQL"),
    ("SELECT COALESCE(SUM(last_chapter), 0)::bigint AS total FROM progress", False,
     "SUM: already widened with ::"),
    ("SELECT CAST(SUM(last_chapter) AS BIGINT) AS total FROM progress", False,
     "SUM: already widened with CAST, which SQLite also accepts"),
    ("SELECT SUM(last_chapter) AS total FROM progress WHERE account = $1::uuid",
     True, "SUM is its own finding, independent of any bind"),
    ("SELECT SUM(last_chapter)::bigint AS total FROM progress WHERE account = $1::uuid",
     False, "both settled"),
    ("SELECT last_chapter FROM progress WHERE account = $1::uuid", True,
     "INT4 column read into an i64"),
    ("SELECT last_chapter::bigint FROM progress WHERE account = $1::uuid", False,
     "INT4: already widened"),
    ("SELECT COUNT(*) AS n FROM progress", False,
     "COUNT is int8 already, no widening wanted"),
    ("INSERT INTO progress (account, work_id, last_chapter, updated_at) "
     "VALUES ($1::uuid, $2::uuid, $3, $4)", False,
     "an INT4 bind is not a decode, so an INSERT is not a finding"),
]


# `SUM` over an integer column is NUMERIC in PostgreSQL and an integer in
# SQLite, so a row type of `i64` cannot decode it. The error names neither the
# aggregate nor the pool: "mismatched types; Rust type `i64` (as SQL type `INT8`)
# is not compatible with SQL type `NUMERIC`". `::bigint` in the PostgreSQL arm
# settles it, and is what events.rs, imports.rs and rating_integrity.rs already
# do -- the rest of the tree had not caught up.
INTEGER_TYPES = ("integer", "bigint", "smallint")
# `SUM(t.cost)` is as common as `SUM(cost)`; the pattern has to see through the
# qualifier or it reports nothing for half the tree.
# `SUM(stars * COALESCE(tl.level, 1))` sums an expression that is still an
# integer, so the first identifier in the argument is enough to judge it --
# `SUM(t.cost)` and `SUM(word_count)` both reduce to the same shape.
SUM_COLUMN = re.compile(r"SUM\(\s*([a-z_][\w]*\s*\.\s*)?([a-z_][\w]*)", re.IGNORECASE)


def sum_sites(sql: str, known: dict[str, dict[str, str]]) -> list[str]:
    """Columns this statement sums whose sum will not decode as an integer."""
    lowered = sql.lower()
    if "::bigint" in lowered or "::int" in lowered:
        return []  # already widened
    if re.search(r"cast\s*\([^)]*sum\s*\(", lowered):
        return []  # CAST(SUM(x) AS BIGINT) -- the form SQLite also accepts
    if "sum" not in sql.lower():
        return []
    out: list[str] = []
    for match in SUM_COLUMN.finditer(sql):
        col = match.group(2).lower()
        for columns in known.values():
            coltype = columns.get(col)
            if coltype in INTEGER_TYPES:
                out.append(col)
                break
    return out


# A column declared INTEGER (INT4) will not decode into an i64 either. It reads
# as the same family as the NUMERIC case and was found the same way -- one test
# failure at a time, each in a different module. "mismatched types; Rust type
# `i64` (as SQL type `INT8`) is not compatible with SQL type `INT4`".
#
# The SELECT list is the place to widen, because a cast on the column also fixes
# a bind compared against it in the same statement. The check is deliberately
# narrow: it only fires on a bare column reference in a SELECT list, because a
# `COUNT(*)` or an already-cast `col::bigint` is fine and must not be reported.
INT4_COLUMNS = ("integer", "smallint")
SELECT_ITEM = re.compile(
    r"(?:^|\s|,)((?:[a-z_][\w]*\s*\.\s*)?)([a-z_][\w]*)(?=\s*(?:,|FROM|AS|$))",
    re.IGNORECASE | re.MULTILINE,
)


def int4_sites(sql: str, known: dict[str, dict[str, str]]) -> list[str]:
    """INT4/INT2 columns in the SELECT list with no widening cast.

    Only for a statement that is actually the PostgreSQL arm. An INT4 column in
    the SQLite string needs no cast -- SQLite has no INT4 -- so without this
    guard the rule reports both halves of every `db.sql(a, b)` pair and roughly
    doubles the output for no new information.
    """
    if not re.search(r"\$\d+", sql):
        return []
    if re.match(r"\s*INSERT\b", sql, re.IGNORECASE):
        # A bind is not a decode. sqlx sends an `i32` for an INTEGER column and
        # PostgreSQL accepts it; nothing fails, so there is nothing to report.
        # Only reading an INT4 column *into an i64* is a real fault.
        return []
    head = re.split(r"\bFROM\b", sql, maxsplit=1, flags=re.IGNORECASE)[0]
    if "::" in head:
        # A cast anywhere in the list is the established fix; do not second-guess
        # which column it was meant for.
        return []
    out: list[str] = []
    for match in SELECT_ITEM.finditer(head):
        col = match.group(2).lower()
        if col in ("count", "sum", "cast", "coalesce"):
            continue
        for columns in known.values():
            if columns.get(col) in INT4_COLUMNS:
                out.append(col)
                break
    return out


def self_test() -> int:
    """Prove the rules on statements whose correct answer is known.

    Run in CI. The guard this file used to carry -- skip any statement with a
    `::` anywhere -- was wrong and the suite could not say so, because there was
    no suite.
    """
    failures = []
    for sql, should_report, note in SELF_TEST_CASES:
        known = {t: SCHEMA[t] for t in tables_in(sql) if t in SCHEMA}
        reported = offending_lines(sql, known)
        if reported != should_report:
            failures.append(
                f"{'false positive' if reported else 'false negative'}: {note}\n  {sql}")
        # A fix must both clear the report and survive a second pass.
        if should_report:
            fixed = add_missing_casts(sql, known)
            if offending_lines(fixed, known):
                failures.append(f"fix left a site behind: {sql}\n  {fixed}")
            if add_missing_casts(fixed, known) != fixed:
                failures.append(f"fix is not idempotent: {sql}\n  {fixed}")

    for failure in failures:
        print(f"FAIL {failure}")
    if failures:
        print(f"\n{len(failures)} of {len(SELF_TEST_CASES)} self-test cases failed")
        return 1
    print(f"self-test: {len(SELF_TEST_CASES)} cases passed")
    return 0


def main(argv: list[str]) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("paths", nargs="*", type=pathlib.Path,
                        default=[pathlib.Path("crates")])
    parser.add_argument("--migrations", type=pathlib.Path,
                        default=pathlib.Path("migrations/postgres"),
                        help="directory holding the PostgreSQL DDL")
    parser.add_argument("--self-test", action="store_true",
                        help="check the checker's own rules and exit")
    args = parser.parse_args(argv[1:])

    if args.self_test:
        return self_test()

    schema = load_schema(args.migrations)
    if not schema:
        print(f"no tables parsed from {args.migrations}", file=sys.stderr)
        return 2

    hits: list[tuple[pathlib.Path, int, str]] = []
    for root in (args.paths or [pathlib.Path("crates")]):
        hits.extend(scan(root, schema))

    if not hits:
        print(f"OK: no text bound to a typed column "
              f"({len(schema)} tables read from {args.migrations})")
        return 0

    by_file: dict[pathlib.Path, list[tuple[int, str]]] = defaultdict(list)
    for path, line, sql in hits:
        by_file[path].append((line, sql))

    total = sum(len(v) for v in by_file.values())
    print(f"{total} site(s) binding text to a typed column, in {len(by_file)} file(s):\n")
    for path, entries in sorted(by_file.items(), key=lambda kv: -len(kv[1])):
        print(f"  {path}  ({len(entries)})")
        for line, sql in entries:
            print(f"    {line:5}  {sql}")
    print(
        "\nEach of these passes on SQLite and fails on PostgreSQL. Fix by casting\n"
        "the placeholder in the PostgreSQL arm ($1::uuid), or -- when the column is\n"
        "read into a String -- casting the column in the SELECT ($1::text). The\n"
        "SQLite arm is unchanged. Column types come from the migrations, so if a\n"
        "report turns out to be wrong the migration is what needs fixing, not this\n"
        "script."
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
