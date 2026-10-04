#!/usr/bin/env python3
"""Fail if a `Backend::Postgres` arm reaches for the SQLite pool.

`Database::sqlite_pool()` returns `None` for a PostgreSQL handle, so the
`expect("sqlite")` that virtually every call site carries fires. The panic
message is the bare string "sqlite", which does not name the real problem --
it reads like "the fixture was wired to the wrong database", when in fact
production code asked for the wrong pool.

That is exactly how `crates/db/src/thread_modes.rs` shipped: `create_forum_topic`
and `add_schedule_section` each had a `Backend::Postgres` arm whose statement
was written for PostgreSQL (`$1::uuid`) and whose executor was
`db.sqlite_pool().expect("sqlite")`. Under SQLite both call sites were never
taken, so the tests passed; under PostgreSQL 27 forum tests died with "sqlite",
which is most of what was failing.

The check is a brace-depth scan rather than a regex over a lookback window,
because a `sqlite_pool()` is only wrong when it sits inside a *Postgres* arm --
roughly 800 of the 824 uses in `crates/*/src` are inside `Backend::Sqlite` arms
and are correct.

Usage:  python3 scripts/check-pg-arm-uses-sqlite-pool.py [paths...]
Exit 0 when clean, 1 with a report when not.
"""

from __future__ import annotations

import pathlib
import re
import sys

# `sqlite_pool()` and the arm header forms that can enclose it.
POOL = re.compile(r"sqlite_pool\(\)")
# Backend::Postgres / Backend::Sqlite, however the arm is spelled.
POSTGRES = re.compile(r"Postgres")



# A function that reaches for the pool without ever naming the backend has no
# PostgreSQL arm at all -- it will run, and it will run against SQLite, and the
# failure will read as a decoding bug in whatever column it touched. `list_bounties`
# shipped that way and reported "Rust type `i64` (as SQL type `INT8`) is not
# compatible with SQL type `INT4`", which names neither the pool nor the port.
FN = re.compile(r"^\s*(?:pub(?:\([\w:]+\))?\s+)?(?:async\s+)?fn\s+(\w+)", re.M)


def _body_end(text: str, start: int, limit: int) -> int:
    """Offset of the closing brace of the function body opening at/after `start`.

    Falls back to `limit` when the brace cannot be found, so a signature spanning
    several lines or a body the scanner cannot balance degrades to the old
    behaviour instead of reporting nothing.
    """
    i = start
    depth = 0
    opened = False
    in_str: str | None = None
    prev = ""
    while i < limit:
        ch = text[i]
        if in_str is not None:
            if ch == in_str and prev != "\\":
                in_str = None
        elif ch in "\"'":
            # Rust has NO single-quoted string literal: `'a'` is a char, and in this
            # codebase it is always a lifetime like `&'a SqlitePool`.
            #
            # Tracking `'` as a string delimiter was a two-character bug with a large
            # blast radius. An apostrophe inside a `//` comment -- and this tree is
            # full of prose like "the previous statement's numbering" -- opened a
            # phantom string that swallowed every brace after it, so brace depth never
            # returned to zero and the depth -> header map went stale for the REST OF
            # THE FILE.
            in_str = ch
        elif ch == "{":
            depth += 1
            opened = True
        elif ch == "}":
            if opened:
                depth -= 1
                if depth == 0:
                    return i
        prev = ch
        i += 1
    return limit


def unported(text: str) -> list[tuple[str, int]]:
    """Functions that use the pool but branch on no backend."""
    out: list[tuple[str, int]] = []
    functions = list(FN.finditer(text))
    for i, m in enumerate(functions):
        limit = functions[i + 1].start() if i + 1 < len(functions) else len(text)
        # The span ends at the function's own closing brace, not at the next `fn`.
        #
        # It used to run to the next `fn`, which swept in everything between --
        # including the DOC COMMENT of the following function. `body_audience.rs`'s
        # `set_audience` is a one-line wrapper around `set_audience_raw`, and it was
        # reported for a `sqlite_pool()` that is not in its body at all but in the
        # next function, whose own (correct, `is_postgres`-branched) implementation
        # was the very fix the comment describes.
        #
        # A wrapper that reaches for the pool only through a helper it delegates to
        # is correct as it stands, which is the same exemption the `_sqlite` naming
        # convention below encodes.
        end = _body_end(text, m.end(), limit)
        body = text[m.end() : end]
        uses_pool = "sqlite_pool()" in body
        # Three spellings of "which backend am I", and all three count as branching.
        #
        # `Backend::` / `.backend()` is the production idiom -- 1039 `match
        # db.backend()` arms in `crates/*/src`. `is_postgres()` is what the TEST
        # harness branches on: `TestDb::is_postgres` exists precisely so a suite can
        # ask without importing `Backend`, and 17 files use it.
        #
        # The rule only knew the first two, so `body_audience.rs`'s
        # `set_audience_raw` was reported for reaching for `sqlite_pool()` "with no
        # backend arm at all" while sitting in the `else` of
        # `if tdb.is_postgres() { ...postgres_pool()... }`. That file is the
        # best-documented port in the tree: its header explains that calling
        # `sqlite_pool()` directly "made seven of the eight tests here fail with a
        # bare `sqlite` panic under LOREHAVEN_TEST_PG_URL". The port is exactly
        # what the gate was asking for; it just could not see it.
        names_backend = (
            "Backend::" in body
            or ".backend()" in body
            or "is_postgres()" in body
        )
        # A file that opens its own `Database` from a hardcoded `sqlite://` URL
        # cannot be a single-backend SUITE: it never consults the backend selector,
        # so there is no arm to port and no way for it to have quietly passed on one
        # engine and failed on the other. It is SQLite-only by construction and by
        # declaration -- `m52_08_rec_shadow.rs` and `m57_metadata_exchange.rs` both say
        # so in their own panic messages ("this file's harness is sqlite"), and
        # neither claims dual-backend coverage anywhere.
        #
        # The finding this rule exists for is a suite that DOES honour the selector
        # and then hardcodes a pool inside it -- which is the `thread_modes.rs` bug in
        # this gate's own header. A file that cannot take the PostgreSQL path has
        # not made that mistake; it has declared what it is.
        if "sqlite://" in text and "TestDb::connect" not in text:
            continue
        # A helper that exists only to be called from one backend's arm is
        # correct as it stands. So is a test. `sqlite_pool()` in the name is the
        # convention this repo already uses for the former.
        if "_sqlite" in m.group(1):
            continue
        if uses_pool and not names_backend:
            out.append((m.group(1), text[: m.start()].count("\n") + 1))
    return out


def scan(text: str) -> list[tuple[int, str]]:
    """Every `sqlite_pool()` in `text`, with the header of the block it sits in.

    Walks the source once maintaining brace depth and a depth -> opening-line
    map, then re-derives that map at each match. String literals are skipped so
    a brace inside SQL text does not shift the depth.
    """
    out: list[tuple[int, str]] = []
    for m in POOL.finditer(text):
        depth = 0
        opened: dict[int, str] = {}
        in_str: str | None = None
        prev = ""
        i = 0
        while i < m.start():
            ch = text[i]
            if in_str is not None:
                if ch == in_str and prev != "\\":
                    in_str = None
            # Rust has NO single-quoted string literal: `'a'` is a char, and in
            # this codebase it is always a lifetime like `&'a SqlitePool`.
            #
            # Tracking `'` as a string delimiter was a two-character bug with a
            # large blast radius. An apostrophe inside a `//` comment -- and this
            # tree is full of prose like "the previous statement's numbering" --
            # opened a phantom string that swallowed every brace after it, so
            # brace depth never returned to zero and the depth -> header map went
            # stale for the REST OF THE FILE.
            #
            # In `category_governance.rs` that made all 18 findings wrong: every
            # `sqlite_pool()` was inside a `Backend::Sqlite` arm and reported as
            # `Backend::Postgres`. A gate that reports 18 correct functions as 18
            # defects is one nobody reads -- which is how the single real defect it
            # was written for (`thread_modes.rs`, in the same commit as this script)
            # ended up sharing a report with 18 false ones.
            elif ch == '"':
                in_str = ch
            elif ch == "{":
                depth += 1
                head = text[:i].rstrip()
                opened[depth] = head[head.rfind("\n") + 1 :].strip()
            elif ch == "}":
                opened.pop(depth, None)
                depth -= 1
            prev = ch
            i += 1
        out.append((m.start(), opened.get(depth, "")))
    return out


def _fixture(name: str) -> str:
    """Read a source fixture from `scripts/fixtures/`."""
    return (pathlib.Path(__file__).parent / "fixtures" / name).read_text(
        encoding="utf-8"
    )


SELF_TEST_CASES: list[tuple[str, str, int, object]] = [
    # (name, source, expect_postgres_arm_hits, expect_unported_hits)
    (
        "an apostrophe in a comment does not swallow the rest of the file",
        # Read from scripts/fixtures/quote_in_comment.rs, a verbatim prefix of
        # crates/db/src/category_governance.rs. It is a fixture FILE rather than an
        # inline string for a reason worth recording: the bug needs an apostrophe in
        # # a comment opening a phantom string that swallows the SQLite arm's braces
        # # and closes on the 'active' inside a SQL literal 250 lines later. Three
        # # hand-written miniatures of that shape all came back CORRECT under the old
        # # scanner -- a fixture that does not fail against the bug is worse than none.
        _fixture("quote_in_comment.rs"),
        0,
        0,
    ),
    (
        # Asserts the NAME, not just the count. The span bug did not change HOW MANY
        # functions were reported, it reported the wrong one: `wrapper` is a one-line
        # delegate, and the old span ran to the next `fn` -- sweeping in the following
        # function's DOC COMMENT and body -- so a correct wrapper was reported for a
        # pool it never touched. `body_audience.rs`'s `set_audience` was that finding.
        "a wrapper is not blamed for the next function's pool use",
        """
async fn wrapper(tdb: &TestDb) {
    helper(tdb).await;
}

/// Dialect-aware through `TestDb::sql`. The first version of this file called
/// `tdb.db().sqlite_pool().expect("sqlite")` directly, which passed on SQLite.
async fn helper(tdb: &TestDb) {
    sqlx::query("UPDATE t SET a = ?")
        .execute(tdb.db().sqlite_pool().unwrap())
        .await;
}
""",
        0,
        ["helper"],
    ),
    (
        "a SQLite pool inside a Postgres arm IS reported",
        """
fn broken(db: &Database) -> Result<bool> {
    let sql = db.sql("UPDATE t SET s = ?", "UPDATE t SET s = $1");
    match db.backend() {
        Backend::Sqlite => {
            let r = sqlx::query(&sql).execute(db.sqlite_pool().expect("sqlite")).await?;
            Ok(r.rows_affected() > 0)
        }
        Backend::Postgres => {
            // The defect this gate exists for: this panics with a bare "sqlite".
            let r = sqlx::query(&sql).execute(db.sqlite_pool().expect("sqlite")).await?;
            Ok(r.rows_affected() > 0)
        }
    }
}
""",
        1,
        0,
    ),
    (
        "a pool with no backend branch at all IS reported",
        """
fn unported(db: &Database) -> Result<bool> {
    let sql = db.sql("UPDATE t SET s = ?", "UPDATE t SET s = $1");
    let r = sqlx::query(&sql).execute(db.sqlite_pool().expect("sqlite")).await?;
    Ok(r.rows_affected() > 0)
}
""",
        0,
        1,
    ),
    (
        "is_postgres() counts as a backend branch",
        """
async fn set_audience(tdb: &TestDb) {
    if tdb.is_postgres() {
        sqlx::query("UPDATE works SET a = ?").execute(tdb.db().postgres_pool().unwrap()).await;
    } else {
        sqlx::query("UPDATE works SET a = ?").execute(tdb.db().sqlite_pool().unwrap()).await;
    }
}
""",
        0,
        0,
    ),
    (
        "a function whose pool use is in the NEXT function is not this one's",
        """
async fn wrapper(tdb: &TestDb) {
    helper(tdb).await;
}

/// The pool use lives HERE, not in `wrapper`.
async fn helper(tdb: &TestDb) {
    sqlx::query("UPDATE t SET a = ?").execute(tdb.db().sqlite_pool().unwrap()).await;
}
""",
        0,
        1,
    ),
]


def self_test() -> int:
    """The cases above, run against this module's own functions.

    This gate had three separate bugs that all presented as `correct code, reported
    wrong` -- a gate that cries wolf. A self-test is what turns the third one from a
    three-session mystery into a two-minute check, and it is the only thing here that
    can notice the scanner's own arithmetic drifting.
    """
    failures = 0
    for name, source, want_arm, want_unported in SELF_TEST_CASES:
        arm_hits = [
            header for _, header in scan(source) if POSTGRES.search(header)
        ]
        unported_hits = unported(source)
        problems = []
        if len(arm_hits) != want_arm:
            problems.append(f"Postgres-arm hits: got {len(arm_hits)}, want {want_arm}")
        if isinstance(want_unported, list):
            got_names = sorted(name for name, _ in unported_hits)
            if got_names != sorted(want_unported):
                problems.append(
                    f"unported names: got {got_names}, want {sorted(want_unported)}"
                )
        elif len(unported_hits) != want_unported:
            problems.append(
                f"unported hits: got {len(unported_hits)}, want {want_unported}"
            )
        if problems:
            failures += 1
            print(f"FAIL {name}: " + "; ".join(problems))
    if failures:
        print(f"{failures} self-test case(s) failed")
        return 1
    print(f"OK: {len(SELF_TEST_CASES)} self-test cases pass")
    return 0


def main(argv: list[str]) -> int:
    if "--self-test" in argv:
        return self_test()
    roots = [pathlib.Path(a) for a in argv[1:]] or [pathlib.Path("crates")]
    files: list[pathlib.Path] = []
    for root in roots:
        if root.is_dir():
            files += [
                p
                for p in sorted(root.rglob("*.rs"))
                if "/src/" in p.as_posix() or "/tests/" in p.as_posix()
            ]
        else:
            files.append(root)

    bad: list[tuple[pathlib.Path, int, str]] = []
    for path in files:
        text = path.read_text(encoding="utf-8", errors="replace")
        if "sqlite_pool()" not in text:
            continue
        for pos, header in scan(text):
            if POSTGRES.search(header):
                line = text[:pos].count("\n") + 1
                bad.append((path, line, header))

    missing: list[tuple[pathlib.Path, str, int]] = []
    for path in files:
        text = path.read_text(encoding="utf-8", errors="replace")
        if "sqlite_pool()" not in text:
            continue
        for name, line in unported(text):
            missing.append((path, name, line))

    if not bad and not missing:
        print("ok: no Backend::Postgres arm calls sqlite_pool(), "
              "and no function uses the pool without a backend arm")
        return 0

    if missing:
        print(f"{len(missing)} function(s) use the pool with no backend arm at all:\n")
        for path, name, line in missing:
            print(f"  {path}:{line}  {name}")
        print(
            "\nSuch a function cannot fail the first check -- there is no\n"
            "PostgreSQL arm to put a sqlite_pool() in. It runs on SQLite under\n"
            "every local test and misbehaves under LOREHAVEN_TEST_PG_URL, where\n"
            "the error names a column type rather than the missing port.\n"
        )
    if not bad:
        return 1

    print(f"{len(bad)} site(s) call sqlite_pool() inside a PostgreSQL arm:\n")
    for path, line, header in bad:
        print(f"  {path}:{line}")
        print(f"      inside: {header}")
    print(
        "\nThe executor for a PostgreSQL arm is\n"
        "    .execute(db.postgres_pool().expect(\"postgres\"))\n"
        "A `Backend::Postgres` arm writing `$1::uuid` while executing on the\n"
        "SQLite pool passes every SQLite test and panics with the message\n"
        "\"sqlite\" under LOREHAVEN_TEST_PG_URL."
    )
    return 1


if __name__ == "__main__":
    raise SystemExit(main(sys.argv))
