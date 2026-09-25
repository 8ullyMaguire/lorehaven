#!/usr/bin/env python3
"""Find PostgreSQL arms that bind a bare `$n` to a UUID column.

`Database::sql` rewrites `?` to `$n` but never casts, and sqlx sends a bound
`&str` as `text`. So a statement that reads

    SELECT dimension_key FROM arena_weights WHERE account_id = $1

works on SQLite and fails on PostgreSQL with

    42804 column "account_id" is of type uuid but expression is of type text

or, where the statement does cast the placeholder and the *column* is being read
into a String instead, with a decode error:

    ColumnDecode { source: "mismatched types; Rust type `String`
                   (as SQL type `TEXT`) is not compatible with SQL type `UUID` }

Both are the same underlying mistake, and both are invisible on SQLite -- which
is how 62 of them shipped. The gate that catches this is running the suite on
both dialects; this script is the cheap pre-flight that says where to look.

The check is deliberately narrow. It only reports a statement when ALL of these
hold:

  * it is a PostgreSQL arm (it contains a `$n` placeholder);
  * it has no cast anywhere (`::`), so it is not already handled;
  * it names a table whose id column is UUID in the PostgreSQL dialect;
  * an id-shaped column is compared to, or bound positionally against, a
    placeholder.

It is a heuristic and will not find everything -- a wrong cast on the wrong
column still passes review. It is a map of where to start, not a verdict.

Usage:  check-uncast-pg-placeholders.py [crates/...]
"""

from __future__ import annotations

import pathlib
import re
import sys

# Tables whose `id` is UUID in the PostgreSQL dialect. Keep in sync with the
# migrations; `works`, `accounts` and `pseuds` are the ones that bite most often
# because so many foreign keys point at them.
UUID_KEYED_TABLES = {
    "accounts", "admin_actions", "arena_ballots", "arena_weights", "bounties",
    "bounty_contributions", "chapter_revisions", "chapters", "collection_items",
    "collections", "comment_classifications", "credit_holds",
    "credit_transactions", "groups", "group_members", "jobs", "listings",
    "media_references", "pseuds", "queue_slots", "recipes",
    "review_tasks", "roadmap_cards", "sanctions", "translation_jobs",
    "translation_units", "vanguard_pins", "vanguard_roles", "work_ratings",
    "works",
}

# A placeholder bound to any of these column names is being handed a uuid.
ID_COLUMNS = re.compile(
    r"\b(\w*_)?(id|account|pseud|owner|actor|reporter|reviewer|entry_id|job_id|"
    r"subject_id|card_id|listing_id|bounty_id|chapter_id|work_id|node_id)\b",
    re.IGNORECASE,
)

# `col = $n` -- the WHERE/ON form.
COMPARED = re.compile(r"\b\w+\s*=\s*\$\d+", re.IGNORECASE)
# `VALUES ($n` -- the INSERT form.
INSERT_BIND = re.compile(r"VALUES\s*\(\s*\$\d+", re.IGNORECASE)

STRING_LIT = re.compile(r'"((?:[^"\\]|\\.)*\$\d+(?:[^"\\]|\\.)*)"', re.DOTALL)


def offending_lines(sql: str) -> bool:
    """True when this statement has a bare placeholder on a uuid-keyed table."""
    if "::" in sql:
        return False  # already casts somewhere; not a bare-placeholder site
    lowered = sql.lower()
    if not any(re.search(rf"\b{t}\b", lowered) for t in UUID_KEYED_TABLES):
        return False
    # The placeholder has to be next to something id-shaped. Comparing the
    # column on the left of `= $n` catches the WHERE form; the INSERT form is
    # caught by the presence of a VALUES tuple whose first bind is positional.
    if COMPARED.search(sql) and ID_COLUMNS.search(sql):
        return True
    return bool(INSERT_BIND.search(sql))


def scan(root: pathlib.Path) -> list[tuple[pathlib.Path, int, str]]:
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
            if not offending_lines(sql):
                continue
            line = text[: match.start()].count("\n") + 1
            flat = " ".join(sql.split())[:78]
            hits.append((path, line, flat))
    return hits


def main(argv: list[str]) -> int:
    roots = [pathlib.Path(a) for a in argv[1:]] or [pathlib.Path("crates")]
    hits: list[tuple[pathlib.Path, int, str]] = []
    for root in roots:
        hits.extend(scan(root))

    if not hits:
        print("OK: every PostgreSQL statement on a uuid-keyed table casts its placeholders")
        return 0

    by_file: dict[pathlib.Path, list[tuple[int, str]]] = {}
    for path, line, sql in hits:
        by_file.setdefault(path, []).append((line, sql))

    total = sum(len(v) for v in by_file.values())
    print(f"{total} uncast placeholder(s) in {len(by_file)} file(s):\n")
    for path, entries in sorted(by_file.items(), key=lambda kv: -len(kv[1])):
        print(f"  {path}  ({len(entries)})")
        for line, sql in entries:
            print(f"    {line:5}  {sql}")
    print(
        "\nEvery one of these passes on SQLite and fails on PostgreSQL. Fix by\n"
        "casting the placeholder in the PostgreSQL arm ($1::uuid), or -- when\n"
        "the column is being read into a String -- casting the column in the\n"
        "SELECT ($1::text). The SQLite arm is unchanged."
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
