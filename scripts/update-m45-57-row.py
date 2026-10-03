#!/usr/bin/env python3
"""Update the M45-57 tracker row after steps 5-6 of docs/plans/m45-23-curator-adapters.md.

Idempotent: rewrites exactly one row, leaves every other line byte-identical.
Uses newline='' so the file's CRLF line endings survive, and asserts the row is
found exactly once so a duplicated id cannot be silently half-updated.

Run from the repo root:  python3 scripts/update-m45-57-row.py
"""
import csv
import io
import pathlib
import sys

ROW_ID = "M45-57"

NOTES = (
    "Spec §55. PATH A (declarative YAML) SHIPPED: Category/Manifest types, the manifest "
    "schema, DeclarativeAdapter, and the submission+review pipeline. Steps 1-6 of "
    "docs/plans/m45-23-curator-adapters.md done: migration 0113 (extension_submissions + "
    "adapter_reviews, both engines), and crates/db/src/source_adapters.rs with §55.2's TL3 "
    "gate (11 tests, SQLite AND PostgreSQL, 6 of 7 mutations proven red). Steps 7-8 "
    "remain: HTTP routes, and §55.5's automated check. PATH B (WASM) IS SPECIFIED AND NOT "
    "BUILT, gated by §55.6 -- the sandbox does not exist, so adopting a runtime is refused "
    "by scripts/check-wasm-gate.py in CI. Neither path is in production yet."
)

EVIDENCE = (
    "crates/scrapers/src/source_manifest.rs (14 tests); "
    "crates/scrapers/src/declarative.rs (17 tests); "
    "crates/app/tests/wasm_gate.rs (3 tests); "
    "scripts/check-wasm-gate.py; "
    "crates/db/src/source_adapters.rs; "
    "crates/db/tests/source_adapters.rs (11 tests, both engines)"
)

STATUS = "partially-implemented"


def main() -> int:
    root = pathlib.Path(__file__).resolve().parent.parent
    path = root / "docs" / "requirements.csv"
    if not path.is_file():
        print(f"error: {path} not found", file=sys.stderr)
        return 1

    # newline='' -> no translation; the reader sees the file's own line endings
    # and csv can then write them back unchanged.
    with path.open(newline="", encoding="utf-8") as fh:
        raw = fh.read()
    reader = csv.reader(io.StringIO(raw, newline=""))
    rows = list(reader)
    if not rows:
        print("error: requirements.csv is empty", file=sys.stderr)
        return 1

    try:
        id_col = rows[0].index("id")
        status_col = rows[0].index("status")
        evidence_col = rows[0].index("evidence")
        notes_col = rows[0].index("notes")
    except ValueError as exc:
        print(f"error: expected column missing from header: {exc}", file=sys.stderr)
        return 1

    hits = [r for r in rows[1:] if len(r) > id_col and r[id_col] == ROW_ID]
    if len(hits) != 1:
        print(f"error: expected exactly one {ROW_ID} row, found {len(hits)}", file=sys.stderr)
        return 1
    row = hits[0]

    before = list(row)
    row[status_col] = STATUS
    row[evidence_col] = EVIDENCE
    row[notes_col] = NOTES

    if row == before:
        print(f"{ROW_ID}: already up to date, nothing written")
        return 0

    buf = io.StringIO(newline="")
    csv.writer(buf, lineterminator="\r\n").writerows(rows)
    with path.open("w", newline="", encoding="utf-8") as fh:
        fh.write(buf.getvalue())
    print(f"{ROW_ID}: updated status={STATUS} and notes ({len(before)} cols kept)")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())