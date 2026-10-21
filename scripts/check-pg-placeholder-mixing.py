#!/usr/bin/env python3
"""Find PostgreSQL statements whose placeholder numbering is ambiguous.

`Database::sql` (and `sql_owned`) hand the PostgreSQL half of a statement to
`rewrite_placeholders`, which walks the string and renumbers **every** `?` to
`$1`, `$2`, `$3`, ... in the order they appear. A hand-written `$n` in the same
statement is left exactly where it is.

So a statement that mixes the two spellings has two numbering systems in it,
and the rewrite reconciles them only by coincidence. When they do not agree,
the statement compiles, every SQLite test passes, and PostgreSQL binds the
wrong values to the wrong columns -- or refuses the statement outright.

    VALUES (?::uuid, ?::uuid, ?, '', 'en', 'general', ?, 'draft', ..., $4, $5, $6)

The four `?` become `$1 $2 $3 $4` and the hand-written `$4 $5 $6` stays put, so
`$4` names two different columns and the last two columns are numbered from a
sequence that has already been used. This is not hypothetical: this exact shape
shipped in `crates/db/src/content.rs` (the `create_work` insert) and was found
by the PostgreSQL arm, where the work's `discussion_mode` came back as
`comments_only` instead of the instance's `thread_only`.

## The two rules

1. **The PostgreSQL half carries one numbering scheme.** Either every bind is a
   `?` -- the rewrite numbers them -- or every bind is a `$n` -- the rewrite
   leaves them alone. A half that mixes the two has two numbering systems in
   it, and whether they agree is a count a reader cannot check by eye and a
   refactor cannot preserve. The estate already writes it this way: of the
   statements in the tree, all but a handful use one scheme or the other.

2. **A PostgreSQL half may not use `?1`-style numbered binds.** `?` is
   rewritten and the digit survives it, so `?1::uuid` becomes `$11::uuid` -- a
   parameter eleven, in a statement that supplies one. SQLite's numbered form
   belongs in the SQLite half only.

Two spellings are legal and are not reported:

  * a PostgreSQL half that uses **only** `$n` -- the rewrite is a no-op on it;
  * a SQLite half that uses **only** `?` -- including the `?::uuid` cast
    spelling, which SQLite accepts.

The JSONB containment operators (`?`, `?|`, `?&`) are PostgreSQL *operators*,
not binds, and are excluded -- a `?` followed by a quote, a pipe or an
ampersand is not a placeholder. A `?` inside a string literal is data.

Exit 0 when clean, 1 with the findings otherwise, so it works as a CI gate.
"""

from __future__ import annotations

import pathlib
import re
import sys

# A Rust string literal. `\\.` with DOTALL would swallow a newline after a
# trailing backslash and end the literal early.
STRING_LIT = re.compile(r'"((?:[^"\\]|\\.)*)"', re.DOTALL)

# The calls whose PostgreSQL half is rewritten: `db.sql(sqlite, postgres)` and
# `sql_owned(db, sqlite, postgres)`.
SQL_PAIR = re.compile(r"(?:\.\s*sql\s*\(|\bsql_owned\s*\()")


def mask_string_literals(sql: str) -> str:
    """Blank out the *contents* of single-quoted literals, keeping the length.

    A `?` or a `$n` inside a string literal is data, not a bind: `WHERE label =
    'why?'` carries no placeholder, and `SELECT '$1'` names no parameter. The
    quotes themselves are kept, so a JSONB operator (`tags ? 'genre'`) is still
    visible to the operator rule -- the operator's `?` sits *outside* the quotes.
    """
    out = list(sql)
    i = 0
    while i < len(sql):
        if sql[i] != "'":
            i += 1
            continue
        i += 1
        while i < len(sql):
            if sql[i] == "'":
                if i + 1 < len(sql) and sql[i + 1] == "'":
                    i += 2  # an escaped quote inside the literal
                    continue
                break
            out[i] = " "
            i += 1
        i += 1
    return "".join(out)


def is_bind_question(sql: str, index: int) -> bool:
    """True when the `?` at `index` is a bind parameter.

    PostgreSQL's JSONB containment operators are also spelled `?`, `?|` and `?&`
    -- `WHERE tags ? 'genre'` -- so a `?` followed by a quote, a pipe or an
    ampersand is an operator rather than a bind. A `?` that ends the literal or
    the line is an ordinary placeholder.
    """
    j = index + 1
    while j < len(sql) and sql[j] == " ":
        j += 1
    if j < len(sql) and sql[j] in ("'", "|", "&"):
        return False
    return True


def question_binds(sql: str) -> list[int]:
    """The offsets of the `?` placeholders the statement carries."""
    return [i for i, ch in enumerate(sql) if ch == "?" and is_bind_question(sql, i)]


def numbered_binds(sql: str) -> list[int]:
    """The `$n` indices the statement names."""
    return [int(m.group(1)) for m in re.finditer(r"\$(\d+)", sql)]


def rewrite(sql: str) -> str:
    """`rewrite_placeholders`, reproduced so the gate can report what it does."""
    out: list[str] = []
    index = 1
    for ch in sql:
        if ch == "?":
            out.append(f"${index}")
            index += 1
        else:
            out.append(ch)
    return "".join(out)


def in_line_comment(text: str, pos: int) -> bool:
    """Is `pos` inside a `//` comment? Cheap: scan back to the line start."""
    start = text.rfind("\n", 0, pos) + 1
    return "//" in text[start:pos]


def sql_comment_stripped(sql: str) -> str:
    """Drop `--` comments, so a fix may explain itself in place."""
    return re.sub(r"--[^\n]*", " ", sql)


def balanced(text: str, open_idx: int) -> int | None:
    """The index of the `)` closing the `(` at `open_idx`, or None."""
    depth = 0
    quote: str | None = None
    i = open_idx
    while i < len(text):
        ch = text[i]
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


def postgres_halves(text: str) -> list[tuple[int, str]]:
    """(line, sql) for the PostgreSQL half of every `db.sql(a, b)` pair.

    The pair is found by the call's name and its arguments. A pair whose
    arguments are named constants is resolved through the constants' own
    definitions, because a statement assembled from a const is the same
    statement and the gate must judge it the same way.

    The PostgreSQL half is the **last** argument of the call: `db.sql(sqlite,
    postgres)` has two and `sql_owned(db, sqlite, postgres)` has three, and the
    database handle is not a string. An argument that is neither a literal nor
    a resolvable constant is kept as the empty string, so an unresolved
    argument never displaces the real PostgreSQL half.
    """
    consts: dict[str, str] = {}
    for m in re.finditer(
        r"(?:const|let)\s+(?:mut\s+)?([A-Za-z_][A-Za-z0-9_]*)\s*(?::\s*&\s*str\s*)?=\s*"
        r'"((?:[^"\\]|\\.)*)"',
        text,
        re.DOTALL,
    ):
        consts[m.group(1)] = m.group(2)

    out: list[tuple[int, str]] = []
    for call in SQL_PAIR.finditer(text):
        open_idx = text.find("(", call.start())
        if open_idx == -1:
            continue
        close = balanced(text, open_idx)
        if close is None:
            continue
        body = text[open_idx + 1 : close]
        args: list[str] = []
        for arg in re.finditer(
            r'"((?:[^"\\]|\\.)*)"|\b([A-Za-z_][A-Za-z0-9_]*)\b', body, re.DOTALL
        ):
            if arg.group(1) is not None:
                args.append(arg.group(1))
            else:
                args.append(consts.get(arg.group(2), ""))
        if len(args) < 2:
            continue
        pg = args[-1]
        if not pg:
            continue
        line = text[: call.start()].count("\n") + 1
        out.append((line, pg))
    return out


def scan(root: pathlib.Path) -> list[tuple[pathlib.Path, int, str]]:
    """Find every PostgreSQL half whose placeholder numbering is ambiguous."""
    hits: list[tuple[pathlib.Path, int, str]] = []
    paths = [root] if root.is_file() else sorted(root.rglob("*.rs"))
    for path in paths:
        if path.suffix != ".rs" or "/target/" in str(path):
            continue
        text = path.read_text(encoding="utf-8", errors="replace")
        for line, raw in postgres_halves(text):
            sql = mask_string_literals(sql_comment_stripped(raw))
            if not re.search(r"\b(?:SELECT|INSERT|UPDATE|DELETE)\b", sql, re.IGNORECASE):
                continue
            # Rule 2: a numbered `?1` in a PostgreSQL half. The rewrite turns
            # the `?` into `$1` and leaves the digit, so `?1` becomes `$11`.
            if re.search(r"\?\s*\d", sql):
                hits.append(
                    (
                        path,
                        line,
                        "the PostgreSQL half uses SQLite's numbered '?1' bind; "
                        "the rewrite turns it into '$11'",
                    )
                )
                continue
            questions = question_binds(sql)
            numbers = numbered_binds(sql)
            # Rule 1: the two spellings in one statement.
            if not questions or not numbers:
                continue
            taken = len(questions)
            colliding = sorted({n for n in numbers if n <= taken})
            rewritten = rewrite(sql)
            after = numbered_binds(rewritten)
            duplicated = sorted({n for n in after if after.count(n) > 1})
            if duplicated:
                detail = f"the rewrite makes ${duplicated[0]} name two columns"
            elif colliding:
                detail = f"${colliding[0]} is inside the range the '?'s take"
            else:
                detail = (
                    "the '?'s are renumbered and the '$n's are not, so the two "
                    "numbering schemes have to agree by hand"
                )
            hits.append(
                (
                    path,
                    line,
                    f"{len(questions)} '?' and {len(numbers)} '$n' in one statement; {detail}",
                )
            )
    return hits


SELF_TEST = [
    # The bug: the hand-written `$4` lands inside the range the `?`s take.
    (
        "INSERT INTO t VALUES (?::uuid, ?::uuid, ?, '', 'en', 'general', ?, 'draft', $4, $5, $6)",
        True,
        "mixed: the rewrite collides",
    ),
    # Mixed but agreeing. Still reported: the agreement is a count a reader
    # cannot check by eye.
    (
        "INSERT INTO t VALUES (?::uuid, ?::uuid, ?::uuid, $4, $5, $6, 1)",
        True,
        "mixed: two numbering schemes",
    ),
    # A pure PostgreSQL half is legal.
    ("INSERT INTO t VALUES ($1::uuid, $2, $3)", False, "all $n: correct"),
    # A pure SQLite half is legal.
    ("INSERT INTO t VALUES (?, ?, ?)", False, "all ?: correct"),
    # The SQLite cast spelling is still all `?`.
    ("INSERT INTO t VALUES (?::uuid, ?::uuid, ?)", False, "all ?: correct"),
    # A JSONB operator is not a bind.
    ("SELECT id FROM t WHERE tags ? 'genre' AND id = $1", False, "the ? is an operator"),
    ("SELECT id FROM t WHERE tags ?| ARRAY['a'] AND id = $1", False, "the ? is an operator"),
    # A `?` inside a string literal is data, not a bind.
    ("SELECT id FROM t WHERE label = 'why?' AND id = $1", False, "the ? is data"),
    # SQLite's numbered form in a PostgreSQL half is the second fault.
    ("SELECT id FROM t WHERE account_id = ?1::uuid LIMIT 1", True, "the numbered ?1"),
]


def self_test() -> int:
    """Prove the rule on statements whose correct answer is known."""
    failures = 0
    for sql, expected, why in SELF_TEST:
        masked = mask_string_literals(sql)
        questions = question_binds(masked)
        numbers = numbered_binds(masked)
        got = bool(re.search(r"\?\s*\d", masked)) or bool(questions and numbers)
        if got != expected:
            failures += 1
            print(
                f"self-test FAILED: {why}\n  sql:    {sql}\n"
                f"  wanted: {expected}\n  got:    {got}",
                file=sys.stderr,
            )
    return failures


def main(argv: list[str]) -> int:
    root = pathlib.Path(argv[1]) if len(argv) > 1 else pathlib.Path(".")
    failures = self_test()
    if failures:
        print(f"self-test: {failures} case(s) failed", file=sys.stderr)
        return 1
    hits = scan(root)
    if hits:
        print("pg placeholder numbering: findings")
        for path, line, detail in hits:
            print(f"  {path}:{line}: {detail}")
        return 1
    print("pg placeholder numbering: clean")
    return 0


if __name__ == "__main__":
    sys.exit(main(sys.argv))
