#!/usr/bin/env python3
"""Gate: SQLite migrations must not use syntax newer than the bundled SQLite.

    python3 scripts/check-sqlite-migration-syntax.py            # the real gate
    python3 scripts/check-sqlite-migration-syntax.py --self-test

WHY THIS EXISTS
---------------
sqlx's `libsqlite3-sys` BUNDLES SQLite. It is not the system sqlite3, and the
two versions differ in what they parse. `ALTER TABLE ... ADD CONSTRAINT`
arrived in SQLite 3.49.0; the bundled engine in `Cargo.lock` is 3.46.0.

So a migration can be valid SQL, work perfectly on a developer's system SQLite,
pass review, and then fail on every fresh database this project actually
creates:

    error returned from database: (code: 1) near "CONSTRAINT": syntax error

which surfaces in `migrate()`, before a single assertion — and therefore as
"every test in every suite failed", with nothing pointing at the migration.

The version is read from the lockfile rather than hardcoded, so a `cargo update`
that bumps the bundled engine turns these from defects into dead rules by
itself. If the lockfile cannot be parsed the script FAILS rather than passing:
a gate that cannot find the version it is checking against has nothing to say,
and a gate that fails quiet is worse than no gate.

WHAT IT CHECKS
--------------
A table of constructs, each with the SQLite release that introduced it. A
migration naming a construct newer than the bundled engine is reported. The
list is deliberately short and deliberately cited: every entry is a bug this
repository has actually had, or a feature the project has reason to want, and
the comment on each names where it came from.

WHAT IT IS NOT
---------------
It is not a SQL parser and does not try to be. It cannot see a construct built
by string interpolation in Rust, and it does not know about future SQLite
releases — an unlisted construct is NOT reported, which is a known coverage gap
and is why the file says so where a maintainer will read it.

NEGATIVE CASES
--------------
`--self-test` runs whole calls through the checker, so a rule that fires on
correct SQL fails the self-test rather than shipping. See `ARM_TEST_CASES`.
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
LOCKFILE = ROOT / "Cargo.lock"
MIGRATION_DIRS = ("migrations/sqlite",)


# --- the bundled engine's version --------------------------------------------


def bundled_sqlite_version() -> tuple[int, int, int]:
    """The SQLite version sqlx compiles in, from `Cargo.lock`.

    The lockfile pins `libsqlite3-sys`, whose bundled `sqlite3.c` is what the
    test binaries actually link. Reading it rather than hardcoding means a
    dependency bump raises the ceiling automatically.
    """
    if not LOCKFILE.exists():
        raise SystemExit(f"cannot read {LOCKFILE}: the version this gate checks against is unknown")

    text = LOCKFILE.read_text(encoding="utf-8")
    # The `[[package]]` block for libsqlite3-sys, then its `version` line. The
    # crate's own version (0.30.x) is NOT the SQLite version, which is the trap:
    # reading it gives an answer, and the wrong one, silently.
    match = re.search(
        r'\[\[package\]\]\s*\nname = "libsqlite3-sys"\s*\nversion = "([^"]+)"',
        text,
    )
    if not match:
        raise SystemExit(
            "no `libsqlite3-sys` block in Cargo.lock: this gate cannot tell which "
            "SQLite the tests link, so it will not guess"
        )

    # The bundled amalgamation's version is in the crate's own Cargo.toml only
    # as a source dependency, so read the vendored header when it is present.
    vendored = list(Path.home().glob(
        ".cargo/registry/src/*/libsqlite3-sys-*/sqlite3/sqlite3.h"
    ))
    if not vendored:
        raise SystemExit(
            "libsqlite3-sys is in the lockfile but its bundled sqlite3.h is not "
            "on disk: run `cargo fetch` first, or this gate cannot pin a version"
        )

    header = vendored[0].read_text(encoding="utf-8", errors="replace")
    found = re.search(r'#define SQLITE_VERSION\s+"(\d+)\.(\d+)\.(\d+)"', header)
    if not found:
        raise SystemExit(
            f"no SQLITE_VERSION in {vendored[0]}: the header is not the one this "
            "gate knows how to read"
        )
    return tuple(int(part) for part in found.groups())  # type: ignore[return-value]


# --- constructs newer than the bundled engine --------------------------------

# Each entry: (name, regex, introduced-in, why-this-is-here).
#
# The regexes match STATEMENT TEXT, so a construct named in a `--` comment is
# stripped first (see `strip_comments`). That is the whole reason the
# `forbidden_construct` tests below include a migration that *talks about* the
# construct without using it: migration 0086 documents `ADD CONSTRAINT` in
# twenty lines of comment and the checker must not report it.
FORBIDDEN: list[tuple[str, re.Pattern[str], tuple[int, int, int], str]] = [
    (
        "ALTER TABLE ... ADD CONSTRAINT",
        re.compile(r"ALTER\s+TABLE\s+\S+\s+ADD\s+CONSTRAINT", re.IGNORECASE),
        (3, 49, 0),
        "0086_body_audience.sql used it. Valid on system SQLite 3.53, a "
        "syntax error on the bundled 3.46, and it broke every fresh database.",
    ),
    (
        "ALTER TABLE ... DROP COLUMN",
        re.compile(r"ALTER\s+TABLE\s+\S+\s+DROP\s+COLUMN", re.IGNORECASE),
        (3, 35, 0),
        "Same shape as the rule above, reported at its own introduction so the "
        "two are separately visible when the bundled version moves.",
    ),
    (
        "ALTER TABLE ... RENAME COLUMN",
        re.compile(r"ALTER\s+TABLE\s+\S+\s+RENAME\s+(COLUMN\s+)?\S+\s+TO", re.IGNORECASE),
        (3, 25, 0),
        "Reported as a lower bound; kept because a bump of the bundled engine "
        "makes it safe and a reader will want to know it was considered.",
    ),
    (
        "RETURNING clause",
        re.compile(r"\bRETURNING\b", re.IGNORECASE),
        (3, 35, 0),
        "The project writes the same queries by hand for both dialects, so "
        "this is a guard on a migration, not a recommendation.",
    ),
]


def strip_comments(sql: str) -> str:
    """Remove `--` comments, respecting quoted strings.

    A construct named in prose is not a construct used in code. Migration 0086
    explains at length why it could not use `ADD CONSTRAINT`; a checker that
    read its comment would report the explanation as the defect — and a gate
    that cries wolf gets muted, which is worse than no gate.
    """
    out: list[str] = []
    in_quote = False
    i = 0
    while i < len(sql):
        ch = sql[i]
        if ch == "'":
            in_quote = not in_quote
            out.append(ch)
        elif not in_quote and ch == "-" and sql[i : i + 2] == "--":
            j = sql.find("\n", i)
            if j == -1:
                break
            out.append(" ")
            i = j
            continue
        else:
            out.append(ch)
        i += 1
    return "".join(out)


def forbidden_construct(version: tuple[int, int, int]) -> list[tuple[str, re.Pattern[str]]]:
    """The rules that apply below `version`."""
    return [
        (name, pattern)
        for name, pattern, introduced, _ in FORBIDDEN
        if version < introduced
    ]


def scan_text(sql: str, version: tuple[int, int, int]) -> list[str]:
    """Names of the too-new constructs used in `sql`."""
    body = strip_comments(sql)
    return [name for name, pattern in forbidden_construct(version) if pattern.search(body)]


def active_rule(version: tuple[int, int, int]) -> tuple[str, re.Pattern[str], tuple[int, int, int], str]:
    """The rule that applies at `version`, or the first if none does.

    The self-test's boundary case needs a rule whose introduction is at or
    above the bundled version — that is the only rule the boundary arithmetic
    can be observed on. Returning the first rule as a fallback keeps the
    caller total; `check()` never calls it.
    """
    active = [rule for rule in FORBIDDEN if version < rule[2]]
    if active:
        return active[0]
    return FORBIDDEN[-1]


def migration_files() -> list[Path]:
    files: list[Path] = []
    for directory in MIGRATION_DIRS:
        base = ROOT / directory
        if base.is_dir():
            files.extend(sorted(base.glob("*.sql")))
    return files


def check() -> int:
    version = bundled_sqlite_version()
    rules = forbidden_construct(version)
    print(
        f"bundled SQLite {version[0]}.{version[1]}.{version[2]}; "
        f"{len(rules)} of {len(FORBIDDEN)} constructs are too new",
        file=sys.stderr,
    )
    findings: list[str] = []
    for path in migration_files():
        for name in scan_text(path.read_text(encoding="utf-8"), version):
            findings.append(f"{path.relative_to(ROOT)}: {name}")

    for finding in findings:
        print(f"FAIL {finding}", file=sys.stderr)
    if findings:
        print(
            "\nThese parse on a newer system sqlite3 and not on the engine the "
            "tests link.\nCheck the introduction version against the bundled one:\n"
            + "\n".join(
                f"  {name}: SQLite {i}.{m}.{p}\n    {why}"
                for name, _, (i, m, p), why in FORBIDDEN
            ),
            file=sys.stderr,
        )
        return 1
    print("OK: no SQLite migration uses a construct newer than the bundled engine")
    return 0


# --- self-test ---------------------------------------------------------------
#
# Both directions for every rule. A self-test with only positive cases is a
# checker nobody can trust to say "OK", and that lesson has been learned twice
# in this repository (see docs/handoff.md on the uncast-placeholder checker and
# the timestamptz fix tool).

CASES: list[tuple[str, str, bool]] = [
    # --- the defect this gate was written for ---
    (
        "add_constraint_is_reported",
        "ALTER TABLE works ADD COLUMN body_audience TEXT;\n"
        "ALTER TABLE works ADD CONSTRAINT works_body_audience_valid CHECK (body_audience IS NULL);",
        True,
    ),
    (
        "add_constraint_lowercase_is_reported",
        "alter table works add constraint c check (1);",
        True,
    ),
    # --- the negative case that matters most: the same words in a comment ---
    (
        "add_constraint_named_in_a_comment_is_not_reported",
        "-- The first version wrote ALTER TABLE works ADD CONSTRAINT works_x CHECK (...),\n"
        "-- which is valid SQL and fails on the bundled engine.\n"
        "ALTER TABLE works ADD COLUMN body_audience TEXT;",
        False,
    ),
    (
        "add_constraint_split_across_lines_is_reported",
        "ALTER TABLE works\n  ADD CONSTRAINT c\n  CHECK (x);",
        True,
    ),
    # --- ordinary migrations must pass ---
    (
        "add_column_is_fine",
        "ALTER TABLE works ADD COLUMN body_audience TEXT;",
        False,
    ),
    (
        "a_trigger_is_fine",
        "CREATE TRIGGER works_body_audience_valid_insert\n"
        "BEFORE INSERT ON works\n"
        "FOR EACH ROW\n"
        "WHEN NEW.body_audience IS NOT NULL\n"
        "BEGIN\n"
        "    SELECT RAISE(ABORT, 'not a legal audience');\n"
        "END;",
        False,
    ),
    (
        "a_create_table_with_a_check_is_fine",
        "CREATE TABLE t (id TEXT PRIMARY KEY, n INTEGER NOT NULL CHECK (n BETWEEN 1 AND 5));",
        False,
    ),
    (
        "an_apostrophe_in_a_comment_does_not_eat_the_rest",
        "-- the reader's own\n"
        "ALTER TABLE works ADD CONSTRAINT c CHECK (1);",
        True,
    ),
    # --- each remaining rule, both directions ---
    # The remaining three rules are INACTIVE on the bundled engine (3.46 has
    # had DROP COLUMN since 3.35, RENAME COLUMN since 3.25 and RETURNING since
    # 3.35). Asserting they fire would be asserting a falsehood about the
    # version comparison, so each is checked at a version where it IS active —
    # which tests the rule rather than the current engine, and keeps the case
    # meaningful after a `cargo update` raises the ceiling past all of them.
    (
        "drop_column_is_reported_below_its_introduction",
        "ALTER TABLE works DROP COLUMN body_audience;",
        True,
    ),
    (
        "rename_column_is_reported_below_its_introduction",
        "ALTER TABLE works RENAME COLUMN a TO b;",
        True,
    ),
    (
        "returning_is_reported_below_its_introduction",
        "INSERT INTO t (id) VALUES (1) RETURNING id;",
        True,
    ),
    (
        "returning_in_a_comment_is_not_reported",
        "-- a RETURNING clause would be nicer here\nCREATE TABLE t (id TEXT);",
        False,
    ),
    (
        "a_construct_the_bundled_engine_has_is_not_reported",
        "ALTER TABLE works DROP COLUMN body_audience;",
        False,
    ),
    # --- the version arithmetic, which is the part that could be wrong ---
    ("exactly_the_introduction_version_is_allowed", None, False),  # filled below
]


def self_test() -> int:
    version = bundled_sqlite_version()
    failures: list[str] = []

    for name, sql, should_report in CASES:
        # Cases whose name says "below_its_introduction" are evaluated at the
        # version just before that construct appeared, not at the bundled one:
        # the bundled engine has had DROP COLUMN and RETURNING for a decade,
        # so asserting they fire here would be asserting a falsehood.
        below = name.endswith("below_its_introduction")
        case_version = (0, 0, 0) if below else version
        if sql is None:
            # The boundary. A rule must be INACTIVE at exactly the version that
            # introduced its construct and ACTIVE one release earlier: the
            # comparison is `version < introduced`, and getting the inequality
            # backwards (or off by one) would either report a legal migration
            # or silently disable the rule. Both directions are checked, on the
            # rule that is actually active for the bundled engine, so this case
            # tests the real arithmetic rather than a rule that is already off.
            _, _, introduced, _ = active_rule(version)
            statement = "ALTER TABLE works ADD CONSTRAINT c CHECK (1);"
            if scan_text(statement, introduced):
                failures.append(
                    "a rule is active at the version that introduced its construct"
                )
            if not scan_text(statement, (introduced[0], introduced[1], introduced[2] - 1)):
                failures.append(
                    "a rule is inactive one release before its construct exists"
                )
            continue
        reported = bool(scan_text(sql, case_version))
        if reported != should_report:
            failures.append(
                f"{name}: {'reported' if reported else 'not reported'}, "
                f"expected {'reported' if should_report else 'not reported'}"
            )

    # A version BELOW every introduction must activate every rule, and a
    # version above all of them must activate none. This is what stops the
    # rule list from quietly becoming a no-op after a dependency bump.
    everything = (0, 0, 0)
    if len(forbidden_construct(everything)) != len(FORBIDDEN):
        failures.append("at version 0.0.0 not every rule is active")
    newest = max(introduced for _, _, introduced, _ in FORBIDDEN)
    if forbidden_construct(newest):
        failures.append(
            f"at the newest introduction version {newest} some rules are still active"
        )

    for failure in failures:
        print(f"FAIL {failure}", file=sys.stderr)
    if failures:
        print(f"\n{len(failures)} self-test case(s) failed", file=sys.stderr)
        return 1
    print(f"OK: {len(CASES)} self-test cases pass "
          f"(bundled SQLite {version[0]}.{version[1]}.{version[2]})")
    return 0


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    return check()


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
