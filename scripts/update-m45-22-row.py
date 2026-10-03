#!/usr/bin/env python3
"""Update M45-22's tracker row in place, preserving CRLF.

The row's status and evidence change as the work lands, but the file is CRLF and
rewriting it through a CSV library turns a one-line edit into 698. So each row is
edited as text: the line is located by its id, and the fields that change are
replaced by index within that line only.

Idempotent: a second run finds the evidence already present and changes nothing.
"""

from __future__ import annotations

import sys
from pathlib import Path

CSV = Path("docs/requirements.csv")

# M45-22: step 1 of the plan shipped (the seen-exclusion fix); steps 2-8 remain.
NEW_STATUS = "partially-implemented"
NEW_EVIDENCE = (
    '"crates/db/tests/seen_exclusion.rs (6 tests, SQLite + PostgreSQL); '
    'crates/db/src/reading.rs::seen_work_ids; crates/db/src/rec_strategy.rs '
    'exclusion; spec §54; docs/plans/m45-22-concierge.md"'
)
NEW_NOTES = (
    '"Gaps review B4. STEP 1 DONE: two dead-code defects fixed so already-read '
    'works are excluded from the RRF blend (rec_engine passed seen: vec![]; '
    'generate_traced built _seen_set and never read it). Steps 2-8 remain: '
    'migration 0113, domain selectors, store, routes, WIP notify, frontend. '
    'Not in production."'
)


def split_fields(line: str) -> list[str]:
    """Split a CSV line, honouring double-quoted fields.

    Written out rather than pulled from the `csv` module because that module's
    reader normalises line endings on the way in, which is the exact damage this
    script exists to avoid.
    """
    fields, buf, in_quotes, i = [], [], False, 0
    while i < len(line):
        ch = line[i]
        if in_quotes:
            if ch == '"':
                if i + 1 < len(line) and line[i + 1] == '"':
                    buf.append('"')
                    i += 1
                else:
                    in_quotes = False
            else:
                buf.append(ch)
        else:
            if ch == '"':
                in_quotes = True
            elif ch == ",":
                fields.append("".join(buf))
                buf = []
            else:
                buf.append(ch)
        i += 1
    fields.append("".join(buf))
    return fields


def quote(value: str) -> str:
    """Quote a field only when it needs it, matching the file's own style."""
    if any(c in value for c in ',"\r\n'):
        return '"' + value.replace('"', '""') + '"'
    return value


def main() -> int:
    raw = CSV.read_bytes().decode("utf-8")
    crlf = "\r\n" in raw
    nl = "\r\n" if crlf else "\n"
    lines = raw.split(nl)

    for index, line in enumerate(lines):
        if not line.startswith("M45-22,"):
            continue

        fields = split_fields(line)
        if len(fields) < 7:
            print(f"M45-22 has {len(fields)} fields, expected 7: {line[:120]}", file=sys.stderr)
            return 1

        # Header order: id, area, description, milestone, status, evidence, notes.
        if fields[4] == NEW_STATUS and NEW_EVIDENCE in fields[5]:
            print("M45-22 already up to date; no change")
            return 0

        fields[4] = NEW_STATUS
        fields[5] = NEW_EVIDENCE
        fields[6] = NEW_NOTES
        lines[index] = ",".join(quote(f) for f in fields)
        break
    else:
        print("M45-22 not found", file=sys.stderr)
        return 1

    CSV.write_bytes(nl.join(lines).encode("utf-8"))
    print(f"M45-22 updated (CRLF={crlf})")
    return 0


if __name__ == "__main__":
    sys.exit(main())
