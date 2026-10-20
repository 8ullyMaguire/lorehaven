#!/usr/bin/env python3
"""The ledger writer's test.

Run: `python3 scripts/ledger_test.py`

The property under test is not "does it write a CSV". It is **"does it write the same bytes
twice"** and **"does it keep CRLF"** — because those are the two things that have broken this
file before. A test that only checked the round-trip would pass on a writer that converted
every line ending to `\n`, and that writer is exactly the one that turned a one-line edit into
a 698-line diff.
"""

from __future__ import annotations

import io
import os
import shutil
import sys
import tempfile

sys.path.insert(0, os.path.dirname(os.path.abspath(__file__)))

import ledger  # noqa: E402


FAILURES = []


def check(condition, message):
    if condition:
        print("  ok   %s" % message)
    else:
        print("  FAIL %s" % message)
        FAILURES.append(message)


def row(id_, area="platform", requirement="a thing", milestone="M0",
        status="planned", evidence="", notes="", verified_at="", gates=""):
    return {
        "id": id_,
        "area": area,
        "requirement": requirement,
        "milestone": milestone,
        "status": status,
        "evidence": evidence,
        "notes": notes,
        "verified_at": verified_at,
        "gates": gates,
    }


def test_round_trip(tmp):
    path = os.path.join(tmp, "requirements.csv")
    ledger.write_rows(path, [row("M0-01"), row("M0-02", area="db/search")])
    rows = ledger.read_rows(path)
    check(len(rows) == 2, "two rows round-trip")
    check(rows[0]["id"] == "M0-01", "the first row keeps its id")
    check(rows[1]["area"] == "db/search", "the second row keeps its area")


def test_crlf_is_preserved(tmp):
    path = os.path.join(tmp, "requirements.csv")
    ledger.write_rows(path, [row("M0-01"), row("M0-02")])
    raw = open(path, "rb").read()
    crlf = raw.count(b"\r\n")
    lone = raw.count(b"\n") - crlf
    check(crlf == 3, "the header and two rows are three CRLFs (got %d)" % crlf)
    check(lone == 0, "there is not one lone LF (got %d)" % lone)
    check(raw.endswith(b"\r\n"), "the file ends with CRLF")


def test_upsert_is_idempotent(tmp):
    path = os.path.join(tmp, "requirements.csv")
    ledger.write_rows(path, [row("M0-01")])
    ledger.upsert_row(path, row("M0-01", status="implemented-fully-tested", evidence="cargo test"))
    first = open(path, "rb").read()
    ledger.upsert_row(path, row("M0-01", status="implemented-fully-tested", evidence="cargo test"))
    second = open(path, "rb").read()
    check(first == second, "upserting the same row twice is byte-identical")
    rows = ledger.read_rows(path)
    check(len(rows) == 1, "the row was replaced, not appended")
    check(rows[0]["status"] == "implemented-fully-tested", "the status was updated")
    check(rows[0]["evidence"] == "cargo test", "the evidence was updated")


def test_upsert_appends_a_new_id(tmp):
    path = os.path.join(tmp, "requirements.csv")
    ledger.write_rows(path, [row("M0-01")])
    ledger.upsert_row(path, row("M0-02"))
    rows = ledger.read_rows(path)
    check(len(rows) == 2, "a new id is appended")


def test_header_is_preserved(tmp):
    """A ledger with seven columns must not silently grow to nine.

    This test's name is the rule, and the rule is the *only* thing it asserts. An earlier
    version of it asserted the opposite — that the canonical columns were appended to an
    existing header — which is the exact defect the module's own header warns about:

        "a ledger that has not yet gained a column does not gain one because somebody
         edited a row."

    The test passed against a writer that grew the ledger from 703 rows of 7 columns to 703
    rows of 9 columns, with two empty fields on every single one, and the diff was 704 lines
    removed and 711 added. **A test whose assertion contradicts its own name is worse than no
    test, because it reads like coverage.**
    """
    path = os.path.join(tmp, "requirements.csv")
    legacy = "id,area,requirement,milestone,status,evidence,notes\r\nM0-01,platform,a thing,M0,planned,,\r\n"
    with io.open(path, "w", encoding="utf-8", newline="") as handle:
        handle.write(legacy)
    ledger.upsert_row(path, row("M0-01", status="implemented-fully-tested"))
    header = open(path, encoding="utf-8", newline="").read().split("\r\n")[0]
    check(
        header == "id,area,requirement,milestone,status,evidence,notes",
        "the header is preserved verbatim (got %r)" % header,
    )
    rows = ledger.read_rows(path)
    check(len(rows) == 1, "the legacy row survived the write")
    check(rows[0]["status"] == "implemented-fully-tested", "the legacy row was updated")


def test_an_edit_does_not_change_the_header_or_the_field_count(tmp):
    """The real ledger's shape, and the shape it must keep.

    The 703 inherited rows have seven fields. An edit that added two empty fields to every
    one of them would be a 1,411-line diff for a one-line change — the same trap, at the same
    size, as the 698-line diff that made this module necessary in the first place.
    """
    path = os.path.join(tmp, "requirements.csv")
    legacy_header = "id,area,requirement,milestone,status,evidence,notes"
    body = "".join(
        "M0-%02d,platform,thing %d,M0,planned,,notes\r\n" % (n, n) for n in range(1, 6)
    )
    with io.open(path, "w", encoding="utf-8", newline="") as handle:
        handle.write(legacy_header + "\r\n" + body)
    before = open(path, "rb").read()

    ledger.upsert_row(path, row("M0-03", status="implemented-fully-tested"))
    after = open(path, "rb").read()

    lines = after.split(b"\r\n")
    check(
        lines[0].decode() == legacy_header,
        "the header is untouched (got %r)" % lines[0].decode(),
    )
    for index, line in enumerate(lines[1:6], start=1):
        check(
            line.count(b",") == 6,
            "row %d has seven fields, not nine (got %r)" % (index, line),
        )
    # Only the one edited row differs. Everything else is byte-identical.
    before_lines = before.split(b"\r\n")
    changed = [
        i
        for i, (a, b) in enumerate(zip(before_lines, lines))
        if a != b
    ]
    check(
        changed == [3],
        "exactly one line changed, and it is the edited row (got %r)" % changed,
    )


def test_a_deliberate_migration_adds_a_column_without_touching_the_rows(tmp):
    """The migration is the ONLY way a column arrives.

    The writer preserves the file's header verbatim, so a ledger that has not yet
    grown `verified_at` and `gates` does not grow them because a row was edited.
    The column arrives here, in its own command, with its own commit — and the
    migration adds the column, empties it on every row, and **does not change the
    row count**. A migration that added or dropped a row would not be a migration.
    """
    path = os.path.join(tmp, "requirements.csv")
    legacy = "id,area,requirement,milestone,status,evidence,notes\r\nM0-01,platform,a thing,M0,planned,,\r\nM0-02,platform,another,M0,planned,,\r\n"
    with io.open(path, "w", encoding="utf-8", newline="") as handle:
        handle.write(legacy)

    assert ledger.add_columns(path, ["verified_at", "gates"]) is True, "the ledger changed"
    assert ledger.add_columns(path, ["verified_at"]) is False, "a second migration changes nothing"

    raw = open(path, "rb").read()
    check(
        raw.count(b"\r\n") == 3 and raw.count(b"\n") == 3,
        "the migrated file is still CRLF (got %d CRLF, %d LF)" % (raw.count(b"\r\n"), raw.count(b"\n")),
    )
    header = raw.split(b"\r\n")[0].decode()
    check(
        header == "id,area,requirement,milestone,status,evidence,notes,verified_at,gates",
        "the new columns are appended in order (got %r)" % header,
    )
    rows = ledger.read_rows(path)
    check(len(rows) == 2, "the row count did not change")
    for row in rows:
        check(row["verified_at"] == "", "the new column is empty on every row")
        check(row["gates"] == "", "the new column is empty on every row")
    check(rows[0]["id"] == "M0-01", "the first row survived")
    check(rows[1]["requirement"] == "another", "the second row survived")


def test_a_migration_refuses_a_column_that_is_not_one(tmp):
    path = os.path.join(tmp, "requirements.csv")
    ledger.write_rows(path, [row("M0-01")])
    before = open(path, "rb").read()
    raised = False
    try:
        ledger.add_columns(path, ["colour"])
    except ValueError:
        raised = True
    check(raised, "an unknown column is refused by name")
    check(
        open(path, "rb").read() == before,
        "the refused migration wrote nothing at all",
    )


def test_unknown_column_is_refused(tmp):
    path = os.path.join(tmp, "requirements.csv")
    ledger.write_rows(path, [row("M0-01")])
    bad = row("M0-02")
    bad["colour"] = "blue"
    raised = False
    try:
        ledger.upsert_row(path, bad)
    except ValueError:
        raised = True
    check(raised, "an unknown column is refused by name")
    check(len(ledger.read_rows(path)) == 1, "the refused row was not written")


def test_missing_id_is_refused(tmp):
    path = os.path.join(tmp, "requirements.csv")
    ledger.write_rows(path, [row("M0-01")])
    raised = False
    try:
        ledger.upsert_row(path, row(""))
    except ValueError:
        raised = True
    check(raised, "a row with no id is refused")


def test_a_corrupt_row_is_refused(tmp):
    """A row with more fields than the header is corruption, not a row."""
    path = os.path.join(tmp, "requirements.csv")
    with io.open(path, "w", encoding="utf-8", newline="") as handle:
        handle.write("id,area\r\nM0-01,platform,surprise\r\n")
    raised = False
    try:
        ledger.read_rows(path)
    except ValueError:
        raised = True
    check(raised, "a row with too many fields is refused")


def test_the_real_ledger_is_crlf():
    """The real file, if it is there. This is the trap the module exists for."""
    path = os.path.join(os.path.dirname(os.path.dirname(os.path.abspath(__file__))),
                        "docs", "requirements.csv")
    if not os.path.exists(path):
        print("  skip the real ledger is not here")
        return
    raw = open(path, "rb").read()
    crlf = raw.count(b"\r\n")
    lone = raw.count(b"\n") - crlf
    check(lone == 0, "the real ledger has no lone LF (got %d)" % lone)
    check(crlf > 0, "the real ledger has CRLF")
    rows = ledger.read_rows(path)
    check(len(rows) > 0, "the real ledger reads (%d rows)" % len(rows))
    check(all(r.get("id") for r in rows), "every real row has an id")


def main():
    tmp = tempfile.mkdtemp(prefix="ledger-test-")
    try:
        for name, fn in sorted(globals().items()):
            if name.startswith("test_") and callable(fn):
                print(name)
                if fn.__code__.co_argcount == 1:
                    fn(tmp)
                else:
                    fn()
    finally:
        shutil.rmtree(tmp, ignore_errors=True)
    print()
    if FAILURES:
        print("%d FAILURE(S)" % len(FAILURES))
        for f in FAILURES:
            print("  - %s" % f)
        return 1
    print("all ledger tests pass")
    return 0


if __name__ == "__main__":
    sys.exit(main())
