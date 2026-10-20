#!/usr/bin/env python3
"""Split the Darkhold specification into `docs/spec/`, one file per section.

WHY THIS IS A SCRIPT AND NOT A ONE-OFF

The split is a **copy**, and a copy that is done by hand is a copy that is done
once. The vault file is the master until the split is complete, so every run of
this script re-derives `docs/spec/` from it — and a section that changed in the
vault shows up as a diff in the split rather than as a divergence nobody notices.

The number space is **reserved before it is used**: slots 01 to 27 hold the
specification's own sections, and 28 to 40 are reserved and unallocated. A
section number is never reused and never renumbered (spec §24.2).

USAGE

    python3 scripts/split-spec.py                    # write docs/spec/
    python3 scripts/split-spec.py --check            # refuse to write; report the drift
    python3 scripts/split-spec.py --source <path>    # a different master
"""

from __future__ import annotations

import argparse
import pathlib
import re
import sys

#: Where the master lives. The vault file stays the master until the split is
#: complete; after that this file becomes the index.
DEFAULT_SOURCE = pathlib.Path.home() / "secondbrain/10-Projects/ultimate-fanfiction-site-spec.md"

#: Where the split lands.
DEFAULT_TARGET = pathlib.Path("docs/spec")

#: The slot every section of the master occupies: the number, the slug, the
#: title, and the milestone that owns it.
#:
#: The title is the specification's own section title, and the milestone is the
#: one that ships it. A section with no milestone is a register rather than a
#: slice, and says so.
SLOTS: list[tuple[int, str, str, str]] = [
    (1, "thesis", "The thesis", "M0"),
    (2, "evidence", "Evidence base", "M0"),
    (3, "inheritance-and-decisions", "Inheritance", "M0"),
    (4, "non-goals", "Non-goals — the hard refusals", "M0"),
    (5, "vocabulary", "The canonical vocabulary", "M1"),
    (6, "domain-model", "Domain model", "M1"),
    (7, "data-model", "Data model", "M1"),
    (8, "architecture", "Architecture", "M1"),
    (9, "ingestion", "Ingestion", "M8"),
    (10, "reading", "Reading", "M5"),
    (11, "search-and-discovery", "Search and discovery", "M7"),
    (12, "taste-themes-moods-channels", "Taste, themes, moods and channels", "M13, M14, M17, M18"),
    (13, "community", "Community", "M12"),
    (14, "governance", "Governance — guests, trust, jury", "M3, M20, M21"),
    (15, "privacy-and-safety", "Privacy and safety", "M22"),
    (16, "extensions", "Extensions and the marketplace", "M23"),
    (17, "federation", "Federation and interoperability", "M25"),
    (18, "the-supporter-tier", "The Supporter tier — compute, not rank", "M24"),
    (19, "configuration", "The configuration contract", "M1"),
    (20, "api-conventions", "The API conventions", "M19"),
    (21, "frontend-and-design", "Frontend and design", "M26"),
    (22, "operations-and-budgets", "Operations, deployment and budgets", "M32"),
    (23, "verification", "The verification discipline", "M1"),
    (24, "the-ledger-and-the-milestones", "The ledger and the milestone plan", "M0"),
    (25, "risks", "The risk register", "—"),
    (26, "open-questions", "Open questions", "—"),
    (27, "appendix", "Appendix — the measured numbers this spec inherits", "M0"),
]

#: The slots beyond the specification's own sections: reserved, and unallocated.
RESERVED_FROM = 28
RESERVED_TO = 40

#: The four rules of spec §24.2, which the index states.
RULES = [
    "**A section number is a citation target, and it is never renumbered.** The number space is "
    "reserved to §40 in blocks of ten, the block titles are fixed when the block opens, and the "
    "directory numbers and the document's section numbers are the same numbers — so the split does "
    "not renumber anything.",
    "**A retired section keeps its number** and gets a withdrawn line. A number is never reused, "
    "so a citation to a retired section still resolves to the thing it cited.",
    "**A new section takes the next free number,** and the index is updated in the same commit as "
    "the section.",
    "**The vault file remains the master until the split is complete.** After that it becomes the "
    "index, and this directory is the specification.",
]


def read_source(path: pathlib.Path) -> str:
    if not path.exists():
        raise SystemExit(f"the master specification is not there: {path}")
    return path.read_text(encoding="utf-8")


def split_sections(text: str) -> dict[int, tuple[str, str]]:
    """The master's sections: number -> (title, body).

    A body runs from its `## N. Title` heading to the next `## ` heading, and the
    heading itself is *not* part of the body — the split file writes its own.
    """
    lines = text.split("\n")
    starts: list[tuple[int, int, str]] = []
    for index, line in enumerate(lines):
        match = re.match(r"^## (\d+)\. (.*)$", line)
        if match:
            starts.append((int(match.group(1)), index, match.group(2).strip()))
    sections: dict[int, tuple[str, str]] = {}
    for position, (number, index, title) in enumerate(starts):
        end = starts[position + 1][1] if position + 1 < len(starts) else len(lines)
        body = "\n".join(lines[index + 1 : end]).strip("\n")
        sections[number] = (title, body)
    return sections


def promote(body: str) -> str:
    """Push a section's headings down one level, so the file reads as a document.

    The master's `### N.M` becomes the split file's `## N.M`, and `#### N.M.K`
    becomes `### N.M.K`. A heading that is already at the floor stays where it
    is — pushing `##` to `###` and `###` to `####` is the whole transform, and
    a heading deeper than that is left alone.
    """
    out = []
    for line in body.split("\n"):
        match = re.match(r"^(#{2,5}) (.*)$", line)
        if match:
            hashes, rest = match.groups()
            out.append(f"{hashes[:-1] if len(hashes) > 2 else '#'} {rest}")
        else:
            out.append(line)
    return "\n".join(out)


def render(number: int, title: str, milestone: str, body: str) -> str:
    return (
        f"# {number}. {title}\n"
        "\n"
        "**Status:** reserved\n"
        f"**Milestone:** {milestone}\n"
        "**Source:** the corresponding section of the Darkhold specification\n"
        "\n"
        f"{body}\n"
    )


def render_reserved(number: int) -> str:
    return (
        f"# {number}. Reserved\n"
        "\n"
        "**Status:** reserved, unallocated\n"
        "**Milestone:** —\n"
        "**Source:** —\n"
        "\n"
        "**This slot is reserved and holds nothing.** The number space is reserved before it is\n"
        "used, so that a section added later takes a number nobody has already cited.\n"
        "\n"
        "A new section takes the next free number, and the index is updated in the same commit\n"
        "as the section (spec §24.2).\n"
    )


def render_index(sections: dict[int, tuple[str, str]], preamble: str) -> str:
    rows = []
    for number, slug, title, milestone in SLOTS:
        rows.append(f"| **{number:02d}** | `{slug}.md` | {title} | {milestone} |")
    reserved = ", ".join(f"**{n:02d}**" for n in range(RESERVED_FROM, RESERVED_TO + 1))
    return f"""# 0. The Darkhold specification — the index

**This directory is the specification.** The vault file it was split from
(`~/secondbrain/10-Projects/ultimate-fanfiction-site-spec.md`) remains the master
until the split is complete; after that it becomes the index and this directory
is the specification.

**The four rules** (spec §24.2):

{chr(10).join(f"{index}. {rule}" for index, rule in enumerate(RULES, start=1))}

## The section map

| slot | file | title | milestone |
|---|---|---|---|
{chr(10).join(rows)}

**Slots {RESERVED_FROM:02d} to {RESERVED_TO:02d} are reserved and unallocated:** {reserved}.

## Which document governs

| | |
|---|---|
| **this directory** | the Darkhold specification, split into sections — **the governing document** |
| **the master** | `~/secondbrain/10-Projects/ultimate-fanfiction-site-spec.md` — the master until the split is complete, and the index afterwards |
| **the implementation plan** | `~/secondbrain/10-Projects/darkhold-implementation-plan.md` — 255 task cards, M0–M32, the build order |
| **the historical record** | `../spec.md` — the pre-conversion specification, kept and not converted |
| **the authority for what is built** | `../requirements.csv` — the requirement ledger, and ADR 0028 for how it is re-triaged |

## How this directory was made

`python3 scripts/split-spec.py` re-derives every file from the master, and
`python3 scripts/check-split-spec.py` asserts that each file's heading number
matches its filename and that the split is in step with the master. **A split
that is done by hand is a split that is done once**, and a section that changed
in the master would show up as a divergence nobody notices.

## How to read this

{preamble}
"""


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--source", type=pathlib.Path, default=DEFAULT_SOURCE)
    parser.add_argument("--target", type=pathlib.Path, default=DEFAULT_TARGET)
    parser.add_argument(
        "--check",
        action="store_true",
        help="do not write; report whether the split is in step with the master",
    )
    args = parser.parse_args(argv)

    text = read_source(args.source)
    sections = split_sections(text)

    missing = [number for number, _, _, _ in SLOTS if number not in sections]
    if missing:
        raise SystemExit(f"the master has no section(s) {missing}")

    # §0 is the master's own reading guide, and the index is the split's.
    preamble = sections.get(0, ("", ""))[1]
    if not preamble:
        preamble = "See the master specification."

    args.target.mkdir(parents=True, exist_ok=True)
    written: list[pathlib.Path] = []
    drift: list[str] = []

    def emit(path: pathlib.Path, content: str) -> None:
        written.append(path)
        if args.check:
            if not path.exists():
                drift.append(f"{path.name} is missing")
            elif path.read_text(encoding="utf-8") != content:
                drift.append(f"{path.name} differs from the master")
        else:
            path.write_text(content, encoding="utf-8")

    for number, slug, _, milestone in SLOTS:
        title, body = sections[number]
        emit(
            args.target / f"{number:02d}-{slug}.md",
            render(number, title, milestone, promote(body)),
        )

    for number in range(RESERVED_FROM, RESERVED_TO + 1):
        emit(args.target / f"{number:02d}-reserved.md", render_reserved(number))

    emit(args.target / "00-index.md", render_index(sections, preamble))

    if args.check:
        if drift:
            print("the split is NOT in step with the master:", file=sys.stderr)
            for line in drift:
                print(f"  {line}", file=sys.stderr)
            return 1
        print(f"the split is in step with the master: {len(written)} files")
        return 0

    print(f"wrote {len(written)} files to {args.target}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
