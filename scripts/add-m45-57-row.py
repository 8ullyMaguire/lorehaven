#!/usr/bin/env python3
"""Append M45-57 to docs/requirements.csv without disturbing the file.

That file is CRLF. Rewriting it with Python's `csv` module converts every line
ending and turns a one-line edit into 698 -- a trap this repo has hit before.
So: split on `\\r\\n`, append one row, rejoin with `\\r\\n`. Idempotent: if the row
is already present, nothing changes and the exit is 0.

Rewrites in place only when the content actually differs, so a re-run is a no-op
rather than a spurious diff.
"""

from __future__ import annotations

import sys
from pathlib import Path

CSV = Path("docs/requirements.csv")

ROW = (
    'M45-57,platform,"Curator-submitted source adapters: declarative manifests, '
    'with the WASM path specified and gated",M45,partially-implemented,'
    '"crates/scrapers/src/source_manifest.rs (14 tests); '
    'crates/scrapers/src/declarative.rs (17 tests); '
    'crates/app/tests/wasm_gate.rs (3 tests); scripts/check-wasm-gate.py",'
    '"Spec §55. PATH A (declarative YAML) SHIPPED: Category/Manifest types, the '
    'manifest schema, DeclarativeAdapter. PATH B (WASM) IS SPECIFIED AND NOT '
    'BUILT, gated by §55.6 -- the sandbox does not exist, so adopting a runtime '
    'is refused by scripts/check-wasm-gate.py in CI. Steps 5-8 of the plan '
    '(migration 0113, submissions store, routes, automated check) remain. '
    'Neither path is in production yet."'
)


def main() -> int:
    raw = CSV.read_bytes().decode("utf-8")
    crlf = "\r\n" in raw
    lines = raw.split("\r\n") if crlf else raw.split("\n")

    if any(line.startswith("M45-57,") for line in lines):
        print("M45-57 already present; no change")
        return 0

    # Keep the trailing newline the file already has.
    trailing = "\r\n" if crlf else "\n"
    if lines and lines[-1] == "":
        lines.pop()

    lines.append(ROW)
    out = (trailing.join(lines)) + trailing

    before = len(raw.splitlines())
    CSV.write_bytes(out.encode("utf-8"))
    print(f"appended M45-57: {before} -> {len(out.splitlines())} lines (CRLF={crlf})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
