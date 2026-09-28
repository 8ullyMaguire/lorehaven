#!/usr/bin/env python3
"""Import feature ideas from a brainstorm markdown file into the consensus arena.

WHY THIS EXISTS. The arena's cards live in `roadmap_cards`, and
`scripts/seed_roadmap.py` already populates that table from
`docs/requirements.csv`. An earlier draft of this script therefore appended
the brainstorm ideas to requirements.csv as a *third* way in, and that was
wrong for two reasons:

  1. It corrupted the file. It hand-assembled CSV rows with string
     concatenation and no quoting, so every idea whose prose contained a
     comma split into extra columns. One row ended up with 1817 columns and
     `seed_roadmap.py` then refused to run at all, because a fragment of a
     title had landed in the `status` column. 668 good rows became
     unparseable.
  2. The CSV is the requirements tracker, not the idea backlog. A brainstorm
     is not a requirement: it has no milestone, no verification evidence and
     no status in the tracker's vocabulary. Forcing it in forces a fake
     `status=planned` and a blank id, which is what the tracker stops being
     trustworthy about.

So this writes to `roadmap_cards` directly, which is the table the arena
actually reads, and leaves requirements.csv alone.

IDEMPOTENCE. A brainstorm is re-read and re-run as it grows, so the same
idea will be seen many times. Deduplication is by NORMALIZED TITLE — the
same key `seed_roadmap.py` uses and the same key
`lorehaven_db::roadmap::find_card_by_title_normalized` uses — so an idea
reworded in the markdown is not added twice, and a card created by the
seeder is not shadowed by one created here. The normalizer is
lowercase, punctuation stripped, whitespace collapsed; it is deliberately
the same rule in all three places, because a second rule is how a duplicate
gets in.

STAGES. An idea marked with a checkmark in the source is `up_next` — the
user's stated meaning of "planned", and the stage the arena actually shows
work in. Unmarked ideas are `idea`. The checkmark is stripped from the
title: it is a marker in the markdown, not part of a feature's name.

BODIES. A card's body is the idea's own prose (§44.1), so a card imported
from a brainstorm arrives readable rather than title-only. It is stripped of
markdown heading/bold syntax and hard-wrapped into paragraphs, because a
line-wrapped markdown file read as prose in the UI looks like a mistake.

Usage:
  python3 scripts/import_brainstorm.py --db-url sqlite:///data/lorehaven.sqlite
  python3 scripts/import_brainstorm.py --dry-run          # plan only
  python3 scripts/import_brainstorm.py --file ~/other.md --lines 227
  python3 scripts/import_brainstorm.py --plan-file docs/ideas/planned.json

`--lines` defaults to ALL of the file. The user reads a portion of their own
brainstorm and may not have read the rest; importing ideas they have never
looked at is a decision for them, not for this script, so the default is
everything and `--lines` is how you say otherwise.
"""

from __future__ import annotations

import argparse
import json
import os
import re
import sqlite3
import sys
import uuid
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path

# Stages a card can occupy. Mirrors the CHECK constraint the migrations
# declare; `up_next` is why a checkmarked idea is not merely `idea`.
STAGE_PLANNED = "up_next"
STAGE_IDEA = "idea"

IDEA_RE = re.compile(r"^\s*[-*]\s+(?P<mark>[✓✔☑]|\[[ xX]✓?\]|\[[ xX]\])\s*(?P<rest>.*)$")
BULLET_RE = re.compile(r"^\s*[-*]\s+(?P<rest>.*)$")
CONT_RE = re.compile(r"^\s{2,}\S")  # an indented continuation line
HEADING_RE = re.compile(r"^\s*#{1,6}\s")
RULE_RE = re.compile(r"^\s*(-{3,}|_{3,}|\*{3,})\s*$")
CHECK_CHARS = "✓✔☑"


def normalize_title(title: str) -> str:
    """lowercase, punctuation stripped, whitespace collapsed.

    Deliberately identical to `seed_roadmap.normalize_title` and to
    `find_card_by_title_normalized` in the DB layer. If you change this,
    change those two with it, or the same idea lands on the board twice.
    """
    lowered = title.lower()
    stripped = re.sub(r"[^\w\s]", "", lowered, flags=re.UNICODE)
    return re.sub(r"\s+", " ", stripped).strip()


def now_iso() -> str:
    return datetime.now(timezone.utc).isoformat()


def clean_prose(text: str) -> str:
    """Turn markdown source text into readable card prose.

    Strips emphasis and heading markers, collapses the hard wrapping a
    markdown file has, and blank-lines paragraphs. Leaves the words alone —
    an idea's description is the author's, and rewriting it is not this
    script's call.
    """
    text = HEADING_RE.sub("", text)
    text = re.sub(r"`([^`]*)`", r"\1", text)             # code spans
    text = re.sub(r"\*\*([^*]+)\*\*", r"\1", text)      # bold
    text = re.sub(r"(?<!\*)\*([^*]+)\*(?!\*)", r"\1", text)  # italic
    text = re.sub(r"^\s*[-*]\s+", "", text, flags=re.M)  # stray bullets
    text = text.replace("→", "->").replace("—", "-")   # normalize dashes
    text = re.sub(r"[ \t]+", " ", text)
    text = re.sub(r"\n{3,}", "\n\n", text)
    return text.strip()


def split_title_and_body(raw: str) -> tuple[str, str]:
    """`Title - description` -> (title, description); no dash -> (raw, '')."""
    # An em/en dash or a spaced hyphen separates the name from the prose.
    m = re.match(r"^(?P<t>[^—–]{2,120}?)\s+[—–]\s+(?P<d>.+)$", raw, re.S)
    if m:
        return clean_prose(m.group("t")), clean_prose(m.group("d"))
    m = re.match(r"^(?P<t>[^—–]{2,120}?)\s+-\s+(?P<d>.+)$", raw, re.S)
    if m:
        return clean_prose(m.group("t")), clean_prose(m.group("d"))
    return clean_prose(raw), ""


@dataclass
class Idea:
    title: str
    body: str
    planned: bool
    line: int
    section: str = ""
    extra: list[str] = field(default_factory=list)


def extract_ideas(path: Path, max_lines: int | None) -> list[Idea]:
    """Read the brainstorm and return its ideas in document order.

    An idea is a top-level bullet. Indented lines beneath it are its
    description and become the card body; a bullet that is a continuation of
    the previous idea's prose (not itself a bolded title) extends that
    body. Headings and `---` rules delimit sections and never produce ideas.
    """
    lines = path.read_text(encoding="utf-8").splitlines()
    if max_lines is not None:
        lines = lines[:max_lines]

    ideas: list[Idea] = []
    section = ""
    cur: Idea | None = None
    pending: list[str] = []

    def flush() -> None:
        nonlocal cur, pending
        if cur is None:
            return
        if pending:
            cur.extra = pending
        ideas.append(cur)
        cur = None
        pending = []

    for n, raw in enumerate(lines, 1):
        if RULE_RE.match(raw):
            continue
        if HEADING_RE.match(raw):
            flush()
            section = HEADING_RE.sub("", raw).strip()
            continue

        m = IDEA_RE.match(raw)
        if m:
            flush()
            rest = m.group("rest").strip()
            title, body = split_title_and_body(rest)
            if not title:
                continue
            cur = Idea(
                title=title,
                body=body,
                planned=m.group("mark") in CHECK_CHARS or "✓" in m.group("mark"),
                line=n,
                section=section,
            )
            continue

        bullet = BULLET_RE.match(raw)
        if bullet is not None and not CONT_RE.match(raw):
            # An unmarked top-level bullet is an ordinary idea.
            flush()
            title, body = split_title_and_body(bullet.group("rest"))
            if not title:
                continue
            cur = Idea(
                title=title,
                body=body,
                planned=False,
                line=n,
                section=section,
            )
            continue

        if cur is not None and (CONT_RE.match(raw) or raw.startswith("  ")):
            pending.append(raw.strip())
            continue
        if cur is not None and raw.strip():
            pending.append(raw.strip())
            continue
        flush()

    flush()

    # Attach collected continuation lines as body text.
    for idea in ideas:
        if idea.extra:
            extra = clean_prose(" ".join(idea.extra))
            if extra:
                idea.body = f"{idea.body}\n\n{extra}".strip() if idea.body else extra
    return ideas


def read_existing_titles(conn: sqlite3.Connection) -> set[str]:
    """Every normalized title already on the board, from any source."""
    try:
        rows = conn.execute("SELECT title FROM roadmap_cards").fetchall()
    except sqlite3.OperationalError:
        return set()
    return {normalize_title(r[0]) for r in rows if r and r[0]}


def read_plan_file(path: Path) -> set[str]:
    """Normalized titles an operator marked planned outside the markdown.

    Lets a review of the board feed back into a re-import: a card moved by
    hand to `up_next` stays there, and so does one listed here.
    """
    if not path.exists():
        return set()
    try:
        data = json.loads(path.read_text(encoding="utf-8"))
    except json.JSONDecodeError as e:
        sys.exit(f"error: {path} is not valid JSON: {e}")
    if isinstance(data, dict):
        data = data.get("planned", [])
    return {normalize_title(str(t)) for t in data}


def main() -> int:
    ap = argparse.ArgumentParser(description=__doc__.split("\n")[0])
    ap.add_argument("--file", type=Path, default=Path("/tmp/brainstorm.md"),
                    help="brainstorm markdown (default: /tmp/brainstorm.md)")
    ap.add_argument("--db-url", default=os.environ.get("DATABASE_URL", ""),
                    help="sqlite path, or sqlite:///path. Defaults to $DATABASE_URL.")
    ap.add_argument("--lines", type=int, default=None,
                    help="only read the first N lines (default: all of them)")
    ap.add_argument("--plan-file", type=Path, default=Path("docs/ideas/planned.json"),
                    help="JSON list of titles to also treat as planned")
    ap.add_argument("--category", default="brainstorm",
                    help="category for imported cards (default: brainstorm)")
    ap.add_argument("--dry-run", action="store_true",
                    help="print the plan, change nothing")
    args = ap.parse_args()

    if not args.file.exists():
        sys.exit(f"error: {args.file} does not exist")

    ideas = extract_ideas(args.file, args.lines)
    total_lines = len(args.file.read_text(encoding="utf-8").splitlines())
    scope = f"lines 1-{args.lines}" if args.lines else f"all {total_lines} lines"
    print(f"read {len(ideas)} ideas from {args.file} ({scope})")

    plan_titles = read_plan_file(args.plan_file) if args.plan_file else set()

    # SQLite only. The arena's PostgreSQL deployment is populated by running
    # this against that database with a --db-url pointing at it, or by the
    # seeder; a python script that quietly spoke one dialect would be worse
    # than one that says so.
    url = args.db_url
    if not url:
        sys.exit("error: no --db-url and no $DATABASE_URL")
    if url.startswith("sqlite:///"):
        url = url[len("sqlite:///"):]
    elif url.startswith("postgres"):
        sys.exit("error: this importer writes to SQLite. For the PostgreSQL "
                 "deployment, run it against a SQLite copy and move the rows, "
                 "or say so and a postgres backend will be added here.")
    elif "://" in url:
        sys.exit(f"error: unsupported db-url scheme: {url}")

    db = Path(url)
    if not db.exists() and not args.dry_run:
        sys.exit(f"error: database not found: {db}")

    conn = sqlite3.connect(db)
    existing = read_existing_titles(conn)
    print(f"{len(existing)} cards already on the board")

    fresh, dupes, promoted = [], [], []
    seen: set[str] = set()
    for idea in ideas:
        key = normalize_title(idea.title)
        if not key:
            continue
        planned = idea.planned or key in plan_titles
        if key in existing or key in seen:
            dupes.append(idea)
            continue
        seen.add(key)
        idea.planned = planned
        (promoted if planned else fresh).append(idea)

    for label, group in (("planned", promoted), ("idea", fresh)):
        print(f"  {len(group)} to add as {label}")

    if dupes:
        shown = ", ".join(f"{d.title[:40]!r}" for d in dupes[:3])
        print(f"  {len(dupes)} skipped as already on the board (e.g. {shown})")

    if args.dry_run:
        print("\ndry run, nothing written")
        return 0

    now = now_iso()
    with conn:
        for idea in promoted + fresh:
            conn.execute(
                "INSERT INTO roadmap_cards"
                " (id, title, body, category, stage, elo_rating,"
                "  matches_played, times_best, times_worst, created_at, updated_at)"
                " VALUES (?,?,?,?,?,?,0,0,0,?,?)"
                " ON CONFLICT(id) DO UPDATE SET"
                "  title=excluded.title, body=excluded.body,"
                "  category=excluded.category, stage=excluded.stage",
                (
                    str(uuid.uuid4()),
                    idea.title,
                    idea.body,
                    args.category,
                    STAGE_PLANNED if idea.planned else STAGE_IDEA,
                    1500.0,
                    now,
                    now,
                ),
            )

    total = conn.execute("SELECT COUNT(*) FROM roadmap_cards").fetchone()[0]
    print(f"\ninserted {len(promoted) + len(fresh)} cards; board now has {total}")
    print(f"  {len(promoted)} at stage {STAGE_PLANNED!r} (planned)")
    print(f"  {len(fresh)} at stage {STAGE_IDEA!r}")
    return 0


if __name__ == "__main__":
    sys.exit(main())
