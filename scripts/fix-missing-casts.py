#!/usr/bin/env python3
"""Add the missing cast to each uncast placeholder the checker reports.

The checker knows the column type from the PostgreSQL schema; this walks the
same sites and appends the cast that type needs. Kept separate from the checker
so a reviewer can read exactly what would change before running it, and so the
checker stays a pure report.
"""
from __future__ import annotations

import importlib.util
import pathlib
import re
import sys

REPO = pathlib.Path(__file__).resolve().parent.parent
CHECKER = REPO / "scripts" / "check-uncast-pg-placeholders.py"

spec = importlib.util.spec_from_file_location("chk", CHECKER)
chk = importlib.util.module_from_spec(spec)
spec.loader.exec_module(chk)  # type: ignore[attr-defined]

def main() -> int:
    schema = chk.load_schema(REPO / "migrations" / "postgres")
    changed_files = 0
    changed_sites = 0

    for path in sorted((REPO / "crates").rglob("*.rs")):
        text = original = path.read_text()

        # Work statement by statement: each SQL string literal is one place
        # where a cast may be missing, and a file has many. Collect the edits and
        # apply them right to left, so no replacement shifts a later offset.
        edits: list[tuple[int, int, str]] = []
        for match in chk.STRING_LIT.finditer(text):
            sql = match.group(1)
            # The same arm rules the checker uses, or the fixer edits the SQLite
            # half of a pair and the checker keeps reporting the PostgreSQL one.
            if chk.arm_at(text, match.start()) == "sqlite":
                continue
            if chk.in_sql_pair(text, match.start()) == "sqlite":
                continue
            # No placeholder test here. `arm_at` and `in_sql_pair` above have
            # already decided which arm this is, and a PostgreSQL arm that binds
            # nothing is still a PostgreSQL arm -- `list_flexible_bounties` had no
            # placeholder and still decoded an INT4 into an i64.
            known = {t: schema[t] for t in chk.tables_in(sql) if t in schema}
            if not known:
                continue
            fixed = chk.add_missing_casts(sql, known)
            if fixed != sql:
                edits.append((match.start(1), match.end(1), fixed))
                changed_sites += 1

        for start, end, replacement in sorted(edits, reverse=True):
            text = text[:start] + replacement + text[end:]

        if text != original:
            path.write_text(text)
            changed_files += 1
            print(f"  {path.relative_to(REPO)}")

    print(f"\n{changed_sites} literal(s) fixed in {changed_files} file(s)")
    return 0


if __name__ == "__main__":
    sys.exit(main())
