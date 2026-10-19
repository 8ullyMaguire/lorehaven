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
    """A ledger with seven columns must not silently grow to nine."""
    path = os.path.join(tmp, "requirements.csv")
    legacy = "id,area,requirement,milestone,status,evidence,notes\r\nM0-01,platform,a thing,M0,planned,,\r\n"
    with io.open(path, "w", encoding="utf-8", newline="") as handle:
        handle.write(legacy)
    ledger.upsert_row(path, row("M0-01", status="implemented-fully-tested"))
    header = open(path, encoding="utf-8", newline="").read().split("\r\n")[0]
    check(
        header == "id,area,requirement,milestone,status,evidence,notes,verified_at,gates",
        "the canonical columns are appended to an existing header (got %r)" % header,
    )
    rows = ledger.read_rows(path)
    check(len(rows) == 1, "the legacy row survived the write")
    check(rows[0]["status"] == "implemented-fully-tested", "the legacy row was updated")


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
