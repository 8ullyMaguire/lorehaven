#!/usr/bin/env python3
"""Find test files that claim dual-backend coverage but pin SQLite.

A test file that documents "runs on both backends" and then hardcodes a SQLite
URL is worse than one that never claimed it: the claim is what stops anyone
looking. `m29_transparency.rs` did exactly this, and four real PostgreSQL-only
defects sat behind it -- a tag merge that never moved a tag, and an explanation
route that returned a redacted 500.

Two things are checked, because either alone is a false negative:

  1. does the file's own header advertise both backends?
  2. does it build a Database from a hardcoded sqlite:// URL instead of going
     through the selector?

A file that is honestly SQLite-only is not flagged; it is renamed by this
report as a one-line note so the gap is on the record.

Usage: python3 scripts/find-single-backend-suites.py [--verbose]
"""

from __future__ import annotations

import re
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# Phrases that mean "this file runs on both engines".
DUAL_CLAIM = re.compile(
    r"(both backends|both dialects|on either backend|whichever backend|"
    r"whichever dialect|either engine|both engines|on both SQLite and Postgres)",
    re.IGNORECASE,
)

# A Database built straight from a config that was pinned to a sqlite:// URL.
# The distinguishing feature is the shape, not the mere presence of a "sqlite://"
# string: a `match std::env::var("LOREHAVEN_TEST_PG_URL")` with SQLite as the
# Err arm is a CORRECT harness -- the sqlite path is the fallback, and that is
# what m29_transparency.rs looks like now that it is fixed. Flagging it would be
# the same false positive this script exists to end, one level up.
SELECTOR = re.compile(r'LOREHAVEN_TEST_PG_URL')
HARD_SQLITE = re.compile(
    r'Database::connect\(&?\s*(?:config|self\.config|\w*config)\s*\.database\s*\)'
    r'|config\.database\s*=\s*DatabaseConfig::new\(\s*format!\(\s*\n?\s*"sqlite://'
    r'|Database::connect\(&DatabaseConfig::new\(\s*\n?\s*format!\(\s*\n?\s*"sqlite://',
    re.S,
)


# Words that make a dual-backend sentence a claim about the SCHEMA rather than
# about this suite. milestone_22.rs says "migration 0024 creates the media entity
# tables on BOTH dialects" -- true, and about the migration, while every test in
# the file runs on SQLite. Reading that as a false claim about the suite is this
# script making the same mistake one level up, so the sentence is judged whole.
ABOUT_THE_SCHEMA = re.compile(r"\bmigration\b|\btables?\b|\bcolumns?\b", re.IGNORECASE)


def strip_comments(text: str) -> str:
    """Drop `//` and `/* */` so a prose mention is not mistaken for a call.

    Crude on purpose: it does not understand string literals, so a `//` inside
    a string is dropped too. That is the safe direction -- this check exists to
    avoid false negatives, and losing a query that happens to contain "//" only
    makes the file look MORE suspicious, which is a re-run and a look, not a
    missed bug.
    """
    text = re.sub(r"/\*.*?\*/", " ", text, flags=re.S)
    return re.sub(r"//[^\n]*", " ", text)


def claims_dual_backend(header: str) -> bool:
    """True when the header claims THIS SUITE runs on both backends.

    Sentence-scoped, because the claim lives in a sentence and so does the
    exemption. A file that says its migration is cross-dialect has not claimed
    its tests are.
    """
    for sentence in re.split(r"(?<=[.!?])\s+|\n[\s>*+-]", header):
        if DUAL_CLAIM.search(sentence) and not ABOUT_THE_SCHEMA.search(sentence):
            return True
    return False


def main(argv: list[str]) -> int:
    verbose = "--verbose" in argv
    tests = sorted((REPO / "crates").rglob("tests/*.rs"))
    claimed = 0
    honest = 0
    bad: list[tuple[str, str]] = []

    for path in tests:
        if "target" in path.parts:
            continue
        text = path.read_text()
        header = text[:3000]  # the claim belongs in the header
        rel = str(path.relative_to(REPO))

        if not claims_dual_backend(header):
            honest += 1
            continue
        claimed += 1
                # A file that routes through the selector is a dual-backend file even
        # if it also names a sqlite:// fallback. The match must be on CODE, not
        # on a mention: a comment saying "honour the backend selector" is not
        # wiring, and treating prose as wiring is how this check would pass a
        # file that only ever opens SQLite.
        selector_in_code = SELECTOR.search(strip_comments(text))
        if selector_in_code:
            if verbose:
                print(f"  ok  {rel} (routes through the selector)")
            continue
        if HARD_SQLITE.search(text):
            bad.append((rel, "hardcoded sqlite:// in a file that claims both backends"))
        else:
            uses_testdb = "TestDb" in strip_comments(text)
            if not uses_testdb:
                bad.append((rel, "claims both backends, never mentions the selector"))
            elif verbose:
                print(f"  ok  {rel}")

    print(f"test files claiming dual-backend coverage: {claimed}")
    print(f"test files making no such claim:            {honest}")
    if bad:
        print(f"\n{len(bad)} file(s) claim both backends without running on both:")
        for rel, why in bad:
            print(f"  {rel}\n      {why}")
        return 1
    print("\nno file claims dual-backend coverage it does not have")
    return 0


if __name__ == "__main__":
    raise SystemExit(main(sys.argv[1:]))
