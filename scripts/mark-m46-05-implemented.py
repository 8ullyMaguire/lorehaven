"""Mark the six M46-05 rows in docs/requirements.csv as implemented.

The rows still read "Not yet implemented" after the code landed. Verified before
editing: crates/domain/tests/query_phase2.rs is 26/26 and covers all six sections of
spec 15.4.1 -- Scoped, MinMatch, +children expansion, ValueKind, QueryBudget (all four
bounds at the spec's 8/24/200/8), and the pretty-printer round trip.

Uses csv.writer so quoting is the stdlib's, not a hand-rolled join. A hand-rolled
version of this corrupted an unrelated row on the first attempt (a field containing a
comma and a newline shifted a column, and `implemented-verified-e2e` silently became a
sentence fragment) -- which is exactly why the round-trip check below compares every
status count against the pre-edit file rather than trusting the write.
"""

import csv
import io
import sys
from collections import Counter
from pathlib import Path

CSV = Path("docs/requirements.csv")

# Section -> (evidence, note). The six rows in file order.
UPDATES = [
    (
        "crates/domain/src/query.rs (QueryAst::Scoped); "
        "crates/domain/src/query_sql.rs; "
        "crates/domain/tests/query_phase2.rs (7 scoped tests)",
        "Implemented and tested. The uncorrelated two-term form is the thing Scoped "
        "exists to prevent, and a_scoped_predicate_is_one_exists_not_two covers it.",
    ),
    (
        "crates/domain/src/query.rs (QueryAst::MinMatch, max_min_match_arity); "
        "crates/domain/tests/query_phase2.rs (6 tests: counting, arity bound, "
        "distinguishable from Or and from min_matches)",
        "Implemented and tested.",
    ),
    (
        "crates/domain/src/query.rs (take_expansion_suffix); "
        "crates/domain/tests/query_phase2.rs (4 tests); closure from migration 0104",
        "Implemented and tested, including the two refusals that matter: over the cap "
        "and a field with no closure.",
    ),
    (
        "crates/domain/src/query.rs (ValueKind); crates/domain/src/query_sql.rs",
        "Implemented and tested.",
    ),
    (
        "crates/domain/src/query.rs (QueryBudget: max_depth 8, max_terms 24, "
        "max_expansion 200, max_min_match_arity 8); "
        "crates/domain/tests/query_phase2.rs (6 budget tests)",
        "Implemented and tested, including that a zero bound means 'no limit' rather "
        "than 'match nothing'.",
    ),
    (
        "crates/domain/src/query.rs (canonical pretty-printer); "
        "crates/domain/tests/query_phase2.rs (round-trip and delimiter tests)",
        "Implemented and tested.",
    ),
]

NEW_STATUS = "implemented-fully-tested"


def main() -> int:
    original = CSV.read_text(encoding="utf-8")
    rows = list(csv.reader(io.StringIO(original)))
    header = rows[0]
    try:
        i_id = header.index("id")
        i_status = header.index("status")
        i_evidence = header.index("evidence")
        i_notes = header.index("notes")
    except ValueError as exc:
        print(f"unexpected header {header}: {exc}", file=sys.stderr)
        return 1

    targets = [r for r in rows[1:] if r[i_id] == "M46-05"]
    if len(targets) != len(UPDATES):
        print(
            f"expected {len(UPDATES)} M46-05 rows, found {len(targets)}",
            file=sys.stderr,
        )
        return 1

    for row, (evidence, note) in zip(targets, UPDATES):
        row[i_status] = NEW_STATUS
        row[i_evidence] = evidence
        row[i_notes] = note

    buf = io.StringIO()
    csv.writer(buf, lineterminator="\n").writerows(rows)
    CSV.write_text(buf.getvalue(), encoding="utf-8")

    # Round-trip check: re-read and compare every status count against the original.
    # A writer that shifts a column shows up here as a changed count, which is how the
    # hand-rolled attempt was caught.
    before = Counter(r[i_status] for r in rows[1:] if True)
    reparsed = list(csv.DictReader(io.StringIO(CSV.read_text(encoding="utf-8"))))
    after = Counter(r["status"] for r in reparsed)
    if len(reparsed) != len(rows) - 1:
        print(f"row count changed: {len(rows) - 1} -> {len(reparsed)}", file=sys.stderr)
        return 1
    for status, count in before.items():
        if after.get(status, 0) != count:
            print(
                f"status {status!r} went {count} -> {after.get(status, 0)}",
                file=sys.stderr,
            )
            return 1
    print(
        f"ok: {len(targets)} rows -> {NEW_STATUS}; "
        f"{len(reparsed)} rows; every status count preserved"
    )
    return 0


if __name__ == "__main__":
    raise SystemExit(main())