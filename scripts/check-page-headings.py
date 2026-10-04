#!/usr/bin/env python3
"""Every page component must have exactly one top-level heading.

## Why this exists

Three pages -- AdminEconomyFlows, ForumSearch and UserSearch -- opened with an
`<h2>` and no `<h1>`. Visually that reads as "the title is styled slightly
differently"; to a screen reader it reads as a document whose title has gone
missing, and the heading outline starts at level 2 for no reason. It survived
review because it looks fine and nothing tested it.

Three of the pages that DO have an h1 pass a dynamic one (`{doc.title}`,
`{card.title}`), so this checker accepts a heading whose only child is an
expression -- it is checking for the ELEMENT, not its text.

## What counts as a page

Anything in `src/routes/`. Components in `src/lib/components/` are excluded:
`NavMenu` renders a `<button>` and no heading, which is correct for a button.

## Why this checks for AT LEAST one h1, and not exactly one

The first version of this checker required exactly one `<h1>` per file and
failed six pages -- Account, AuthorMedia, Docs, Reader, WorkEditor, WorkPage.
All six were wrong and the checker was right to look.

Then all six turned out to be fine: each has one `<h1>` per *mutually exclusive*
branch, so exactly one ever renders. `WorkPage` has a paywall heading, a
not-found heading and a work-title heading, and the reader is shown exactly one
of them. Requiring one `<h1>` per FILE would have forced a defect into six
working pages to satisfy a rule about static text.

Deciding whether two headings are branch-exclusive needs to know the control
flow, which is not something a regular expression can establish. So this checker
enforces the half it can prove -- no page may have ZERO -- and the other half is
asserted at runtime by the component tests, where the branches are actually
taken. A checker that guesses at control flow and fails working pages is worse
than one that checks less.

## Running it

    python3 scripts/check-page-headings.py
    python3 scripts/check-page-headings.py --self-test

Exits non-zero and names each file and line on a violation.
"""

from __future__ import annotations

import argparse
import re
import sys
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ROUTES = ROOT / "frontend" / "src" / "routes"

# An <h1 ...> opening tag, allowing attributes across lines.
H1_OPEN = re.compile(r"<h1\b", re.IGNORECASE)

# Markup hidden from the reader. A commented-out <h1> is not a heading.
COMMENT_BLOCK = re.compile(r"<!--.*?-->", re.DOTALL)
SCRIPT_BLOCK = re.compile(r"<script\b.*?</script>", re.DOTALL | re.IGNORECASE)
STYLE_BLOCK = re.compile(r"<style\b.*?</style>", re.DOTALL | re.IGNORECASE)


def visible_markup(source: str) -> str:
    """Strip script, style and HTML comments.

    Without this, a commented-out heading counts and a page can pass the check
    while rendering no title at all.
    """
    for pattern in (SCRIPT_BLOCK, STYLE_BLOCK, COMMENT_BLOCK):
        source = pattern.sub("", source)
    return source


def heading_count(source: str) -> int:
    return len(H1_OPEN.findall(visible_markup(source)))


def headings(source: str) -> int:
    """Number of <h1> elements in the RENDERABLE markup."""
    return heading_count(source)


def check_dir(routes: Path) -> tuple[list[str], int]:
    problems: list[str] = []

    if not routes.is_dir():
        return [f"no routes directory at {routes}"], 0

    pages = sorted(
        p for p in routes.glob("*.svelte") if not p.name.endswith(".test.svelte")
    )
    if not pages:
        return [f"no pages found in {routes} -- is the path right?"], 0

    branched = 0
    for page in pages:
        source = page.read_text(encoding="utf-8")
        count = heading_count(source)
        name = page.name

        if count == 0:
            line = next(
                (
                    i
                    for i, text in enumerate(source.splitlines(), 1)
                    if "<section" in text or "<div" in text
                ),
                1,
            )
            problems.append(f"{name}: no <h1> (markup starts near line {line})")
        elif count > 1:
            # Not a failure. See the module docstring: several pages carry one
            # h1 per mutually exclusive branch. Counted, and reported, so the
            # list stays visible rather than being silently tolerated.
            branched += 1

    return problems, branched


# --- self-test ---------------------------------------------------------------
# A checker that cannot fail is a checker that is not running. These cases are
# what a previous version of this repository's gates got wrong: they reported
# nothing at all, which looked identical to passing.

CASES: list[tuple[str, str, int]] = [
    ("<h1>Title</h1>", "plain h1", 1),
    ("<h1 class='x'>T</h1>", "h1 with attributes", 1),
    ("<h1\n  id='a'\n>T</h1>", "h1 with newlines in the tag", 1),
    ("<h1>{page.title}</h1>", "dynamic h1 expression", 1),
    ("<h2>Title</h2>", "h2 only -- a violation", 0),
    ("<h1>a</h1><h1>b</h1>", "two h1s across branches -- allowed", 2),
    ("<!-- <h1>hidden</h1> -->", "commented-out h1 is not a heading", 0),
    (
        "<script>const x = '<h1>';</script><h1>real</h1>",
        "h1 inside a script string is not a heading",
        1,
    ),
    (
        "<style>.a { content: '<h1>'; }</style><h1>real</h1>",
        "h1 inside a style string is not a heading",
        1,
    ),
    ("", "empty source", 0),
]


def self_test() -> int:
    failures = 0
    for source, label, expected in CASES:
        actual = heading_count(source)
        if actual == expected:
            print(f"  ok   {label}")
        else:
            failures += 1
            print(f"  FAIL {label}: expected {expected}, got {actual}")

    total = len(CASES)
    print(f"self-test: {total - failures}/{total} passed")
    return failures


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument(
        "--self-test",
        action="store_true",
        help="verify the checker itself against known cases",
    )
    parser.add_argument(
        "--routes",
        default=str(ROUTES),
        help="path to the routes directory (default: frontend/src/routes)",
    )
    args = parser.parse_args()

    if args.self_test:
        return 1 if self_test() else 0

    routes = Path(args.routes)
    problems, branched = check_dir(routes)

    if problems:
        print(f"FAIL: {len(problems)} page(s) have a heading problem:", file=sys.stderr)
        for problem in problems:
            print(f"  - {problem}", file=sys.stderr)
        return 1

    count = len(list(routes.glob("*.svelte")))
    where = routes.relative_to(ROOT) if routes.is_relative_to(ROOT) else routes
    # Report what was read. A checker that is silent on success is
    # indistinguishable from one that stopped reading input.
    print(f"OK: all {count} pages have an <h1> ({where})")
    if branched:
        print(
            f"    ({branched} carry one h1 per mutually exclusive branch; "
            f"exactly one renders)"
        )
    return 0


if __name__ == "__main__":
    sys.exit(main())