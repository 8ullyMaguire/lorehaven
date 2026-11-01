#!/usr/bin/env python3
"""Generate the module registry: the 74 compiled surfaces, and every idea.

Why a script rather than a hand-kept list: §59.7 says the catalogue must not
silently lose items, and the ideas live in eight places (requirements.csv, four
brainstorms, three designs, thirteen spec amendments, the 100-idea audit, the
vault's growth notes, and concord's repo entry). A hand-kept list of those is a
list somebody stops updating the week they are busy.

Two outputs, both committed:

  crates/domain/src/modules/registry.rs   the compiled catalogue
  docs/ideas-catalogue.json               the same rows, for the drift test

The drift test (`every_idea_in_the_json_is_in_the_registry`) compares the JSON's
ids against the registry's, so regenerating is not required to notice a hand edit
that dropped a row — but running this script is what updates both.

Relevance, which decides the default (§59.10)
---------------------------------------------
A row's `relevance` is 0..10 and `default_on` follows it:

    built surface                        +6
    named in docs/spec.md by title       +2
    has a requirements.csv row           +1
    appears in two or more sources       +1
    needs a third party / external client -2   (bot, mobile, desktop, OPDS, extension)
    needs heavy compute or a model        -1   (embedding, transcript, narration)
    the source says maybe/someday/later   -1

A built surface therefore cannot score below 6, and the four opt-in-by-design
surfaces are pushed under the line by the external-client and compute terms
(arena, quiz, tasting, concierge, narration, vanguard). Nothing load-bearing is
allowed near the threshold: `works`, `search`, `catalogue` and `library` score 9
or 10, and the test asserts it, because a formula that can switch off the
catalogue is a formula with a bug, not a policy.
"""
from __future__ import annotations

import csv
import json
import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
REGISTRY = REPO / "crates/domain/src/modules/registry.rs"
JSON_OUT = REPO / "docs/ideas-catalogue.json"
VAULT = Path("/home/alvaro/secondbrain/10-Projects")

# Surfaces that exist and are opt-in by design. Each needs a reason: a surface
# switched off by default is a promise the operator did not make, so the list
# stays short and every entry argues itself.
OPT_IN = {
    "arena": "a calibration game; engaging with it is a choice, not a default",
    "quiz": "onboarding only; a reader who skips it is not missing a surface",
    "tasting": "an experimental sampler, superseded in spirit by the draw",
    "concierge": "an assistant that answers on the instance's behalf; opt in",
    "narration": "speech synthesis needs an engine on the host; opt in",
    "vanguard": "a role that must be granted before it means anything",
}

EXTERNAL = re.compile(
    r"\b(bot|telegram|discord|mastodon|bluesky|tumblr|browser extension|firefox|chrome|"
    r"mobile|desktop|electron|tauri|opds|calibre|email|newsletter|webhook|sms)\b", re.I)
COMPUTE = re.compile(r"\b(embedding|embeddings|llm|model|transcrib|ocr|narration|speech|vector)\b", re.I)
HEDGE = re.compile(r"\b(maybe|perhaps|someday|sometime|later|wishlist|speculative|unclear|unsure)\b", re.I)

# Titles too generic to be an idea; they are document furniture, not surfaces.
SKIP_TITLE = re.compile(
    r"^(context|the |what |why |how |notes?|method|status|summary|introduction|overview|"
    r"open questions|risks?|appendix|references?|tiers?|phase\b|part\b|table of|"
    r"decision|trade-?offs?|implementation|acceptance|design|background|motivation|"
    r"data model|schema|api|routes?|costs?|alternatives?|consequences?|migration|"
    r"measur|test|rollout|open|non-goals?|goals?|problem|solution|questions?)",
    re.I)


def slug(text: str) -> str:
    text = text.lower().replace("&", " and ").replace("/", " ")
    text = re.sub(r"[^a-z0-9]+", "-", text).strip("-")
    return re.sub(r"-{2,}", "-", text)[:44].strip("-")


def headings(path: Path, levels: tuple[int, ...] = (2, 3)) -> list[str]:
    """Markdown headings at the requested levels, minus fences and furniture."""
    out: list[str] = []
    in_fence = False
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        if line.startswith("```"):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        m = re.match(r"^(#{1,6})\s+(.+?)\s*$", line)
        if not m:
            continue
        if len(m.group(1)) not in levels:
            continue
        title = re.sub(r"[*_`]", "", m.group(2)).strip()
        title = re.sub(r"^\d+[.)]\s*", "", title)
        title = re.sub(r"\(.*?\)$", "", title).strip()
        if len(title) < 4 or len(title) > 70:
            continue
        # Registers that quote their own sections, and headings whose only job is
        # navigation, are not ideas.
        if SKIP_TITLE.match(title):
            continue
        if title.lower() in {"the rule", "also", "when to use"}:
            continue
        out.append(title)
    return out


def audit_absent() -> list[str]:
    """The 100-idea audit's own list of what is not built.

    Its tables are the only part that names one idea per row; the prose above
    them is a method note. Reading the tables and nothing else is deliberate —
    a regex over the prose would invent items.
    """
    path = REPO / "docs/audit-100-reader-ideas-2026-10-04.md"
    if not path.exists():
        return []
    out: list[str] = []
    for line in path.read_text(encoding="utf-8", errors="replace").splitlines():
        m = re.match(r"^\|\s*(\d+)\s*\|\s*(.+?)\s*\|", line)
        if not m:
            continue
        title = re.sub(r"[*_`]", "", m.group(2)).strip().strip('"').replace('"', "")
        # One-word cells ("cover", "pace", "seeding") are table column values, not
        # ideas: the audit's table has cells that are attributes as well as items.
        if 4 <= len(title) <= 70 and words(title, 2):
            out.append(title)
    return out


def requirement_rows() -> list[tuple[str, str, str]]:
    """(id, requirement, milestone) for every planned row."""
    rows = list(csv.DictReader((REPO / "docs/requirements.csv").open(encoding="utf-8")))
    return [
        (r["id"], r["requirement"].strip(), r["milestone"].strip())
        for r in rows
        if r["status"].strip() == "planned"
    ]


def host_rows() -> list[dict]:
    """The compiled surfaces, with the toggles a human chose.

    Parsed from the current registry so the script does not carry a second copy
    of judgements that already exist; the generated file keeps them verbatim.
    """
    src = REGISTRY.read_text(encoding="utf-8")
    out = []
    for block in re.findall(r"Module \{(.*?)\n    \},", src, re.S):
        def field(name: str, default: str = "") -> str:
            m = re.search(rf'{name}:\s*"?([^",\n]*)"?', block)
            return m.group(1) if m else default
        out.append(
            {
                "id": field("id"),
                "name": field("name"),
                "summary": field("summary"),
                "default_on": "default_on: true" in block,
                "stability": field("stability").split("::")[-1].strip(),
                "toggle": field("toggle").split("::")[-1].strip(),
                "milestone": field("milestone"),
                "spec": field("spec"),
                "planned": "Stability::Planned" in block,
            }
        )
    return out


def words(text: str, least: int) -> bool:
    """At least `least` word-ish tokens of 2+ letters: a heading that is one word
    is usually a label, not a proposal."""
    return len(re.findall(r"[A-Za-z]{2,}", text)) >= least


def shorten(text: str, limit: int = 80) -> str:
    """Cut at a word boundary: a name truncated mid-word reads as a bug."""
    text = re.sub(r"\s+", " ", text).strip()
    if len(text) <= limit:
        return text
    cut = text[:limit].rsplit(" ", 1)[0]
    return cut.rstrip(",;:") + "…"


def escape(text: str) -> str:
    return text.replace("\\", "\\\\").replace('"', '\\"').replace("\n", " ")


def write_rust(rows: list[dict]) -> Path:
    """Emit `ideas.rs`: one `Module` literal per idea, planned and off.

    `Planned` is what makes the row unenablable, and that is the point: a flag
    cannot compile a feature, so the catalogue holds the idea without pretending
    the instance can switch it on (`validate` refuses it by name and source).
    """
    lines = [
        "//! The idea catalogue (§59.7) — every idea the project has had and not",
        "//! built, as a registry row.",
        "//!",
        "//! GENERATED by `scripts/ideas_catalogue.py`. Do not edit by hand: run the",
        "//! script, which also writes `docs/ideas-catalogue.json` and reads eight",
        "//! sources (requirements.csv, four brainstorms, three designs, thirteen",
        "//! spec amendments, the 100-idea audit, the vault's growth notes).",
        "//!",
        "//! Every row is `Stability::Planned` and `default_on: false`: an idea has no",
        "//! routes to switch, and `modules::validate` refuses enabling one by name,",
        "//! naming the documents it came from.",
        "",
        "use super::{Module, Stability, Toggle};",
        "",
        "/// Every registered idea, most relevant first.",
        "pub const IDEAS: &[Module] = &[",
    ]
    for row in rows:
        summary = escape(row.get("why_summary") or f"idea from {', '.join(row['sources'])}")[:200]
        sources = ", ".join(row["sources"])
        name = escape(shorten(row["title"], 80))
        lines.append("    Module {")
        lines.append(f'        id: "{row["id"]}",')
        lines.append(f'        name: "{name}",')
        lines.append(f'        summary: "{summary}",')
        lines.append("        default_on: false,")
        lines.append("        stability: Stability::Planned,")
        lines.append("        toggle: Toggle::Operator,")
        lines.append("        requires: &[],")
        lines.append(f'        milestone: "{escape(row["milestone"])[:40]}",')
        lines.append('        spec: "\u00a759.7",')
        lines.append("        nav: &[],")
        lines.append(f'        relevance: {row["relevance"]},')
        lines.append(f'        source: "{escape(sources)[:160]}",')
        lines.append("    },")
    lines.append("];")
    path = REPO / "crates/domain/src/modules/ideas.rs"
    path.write_text("\n".join(lines) + "\n")
    return path


def spec_text() -> str:
    return (REPO / "docs/spec.md").read_text(encoding="utf-8", errors="replace")


# What a source is worth, and why. The order is the operator's own commitment
# ladder: a requirements row is work already agreed; a spec amendment is a design
# already argued; a brainstorm is a proposal; the audit is a list somebody else
# wrote; the vault is where an idea was first typed.
SOURCE_WEIGHT = [
    ("docs/requirements.csv", 4, "a requirements row exists (+4)"),
    ("docs/spec-amendments/", 3, "specified by an amendment (+3)"),
    ("docs/ideas/", 2, "in an idea sweep (+2)"),
    ("docs/design/", 2, "in a design (+2)"),
    ("docs/audit-", 1, "in the reader audit (+1)"),
    ("vault:", 1, "in a vault note (+1)"),
]


def score(idea: dict, spec: str, committed: bool, sources: int) -> tuple[int, list[str]]:
    why: list[str] = []
    value = 0
    if idea.get("built"):
        value += 6
        why.append("built (+6)")
    best = 0
    best_why = ""
    for row in idea["sources"]:
        for prefix, weight, label in SOURCE_WEIGHT:
            if row.startswith(prefix) and weight > best:
                best, best_why = weight, label
    if best:
        value += best
        why.append(best_why)
    if idea["title"].lower() in spec.lower():
        value += 2
        why.append("named in the spec (+2)")
    if committed and best < 4:
        value += 1
        why.append("its words appear in requirements.csv (+1)")
    if sources >= 3:
        value += 1
        why.append(f"appears in {sources} sources (+1)")
    if EXTERNAL.search(idea["title"]) or EXTERNAL.search(idea.get("context", "")):
        value -= 2
        why.append("needs a third party or an external client (-2)")
    if COMPUTE.search(idea["title"]) or COMPUTE.search(idea.get("context", "")):
        value -= 1
        why.append("needs a model or heavy compute (-1)")
    if HEDGE.search(idea.get("context", "")):
        value -= 1
        why.append("the source hedges (-1)")
    return max(0, min(10, value)), why


def main() -> int:
    spec = spec_text()
    planned = requirement_rows()
    planned_titles = {t.lower() for _, t, _ in planned}
    planned_by_id = {i: m for i, _, m in planned}

    sources: dict[str, set[str]] = {}
    ideas: list[dict] = []

    def add(
        title: str,
        source: str,
        context: str = "",
        milestone: str = "",
        built: bool = False,
        row_id: str = "",
    ):
        title = title.replace('"', "").strip()
        if not words(title, 2):
            return
        key = slug(title)
        if not key:
            return
        entry = next(
            (e for e in ideas if e["slug"] == key or (row_id and e.get("row_id") == row_id)),
            None,
        )
        if entry is None:
            entry = {
                "slug": key,
                "title": title,
                "sources": [],
                "context": context,
                "milestone": milestone,
                "built": built,
                "row_id": row_id,
            }
            ideas.append(entry)
        if source not in entry["sources"]:
            entry["sources"].append(source)
        if context:
            entry["context"] = (entry["context"] + " " + context)[:400]
        if milestone and not entry["milestone"]:
            entry["milestone"] = milestone

    # 1. requirements.csv — the committed queue. Keep the row's id: re-slugging the
    #    sentence gave `idea.dh-watch-saved-query-exec-cmd-notify-ntfy-de`, which is
    #    unreadable and hides that this is M63-12 rather than a new idea.
    for rid, requirement, milestone in planned:
        add(requirement.split(":")[0], "docs/requirements.csv", requirement, milestone, row_id=rid)

    # 2. Brainstorms enumerate their ideas as `### Something` headings.
    for path in sorted((REPO / "docs/ideas").glob("*.md")):
        if path.name == "README.md":
            continue
        for title in headings(path, levels=(3,)):
            add(title, str(path.relative_to(REPO)))

    # 3. A spec amendment or a design IS the idea. Reading its internal headings
    #    instead yields its sections — "Acceptance", "Config", "4. Errors" — which
    #    is what the first run of this script produced: 375 rows, most of them
    #    furniture, and a relevance score that sorted "5 Acceptance" above the
    #    brainstorming it came from.
    for path in sorted([*(REPO / "docs/spec-amendments").glob("*.md"),
                        *(REPO / "docs/design").glob("*.md")]):
        title = path.stem
        title = re.sub(r"^\d{4}-\d{2}-\d{2}-", "", title)
        title = re.sub(r"^\d{4}-", "", title)
        title = title.replace("-", " ").strip()
        if title.startswith("20"):  # a dated design note is still an idea
            title = title[11:]
        add(title.title(), str(path.relative_to(REPO)))

    # 3. the audit's own absent list.
    for title in audit_absent():
        add(title, "docs/audit-100-reader-ideas-2026-10-04.md")

    # 4. the vault's *idea* notes. Named files, not a grep, and not the session
    #    notes: `ranking-substrate-47` and `companion-firefox-extension-48` record
    #    what happened, so their `###` headings are narration — "State at handoff",
    #    "Three decisions I made", "Lesson worth keeping" — and reading them put a
    #    changelog into a backlog. Two of the four named sources were that mistake.
    for name in [
        "2026-09-20T183349+0200-darkhold-growth-ideas.md",
        "2026-09-30-cross-project-feature-suggestions.md",
    ]:
        path = VAULT / name
        if not path.exists():
            print(f"warning: missing source {path}", file=sys.stderr)
            continue
        for title in headings(path, levels=(3,)):
            add(title, f"vault:{name}")

    host = host_rows()
    host_ids = {h["id"] for h in host}
    host_titles = {h["name"].lower(): h["id"] for h in host}

    rows = []
    below = []
    for idea in ideas:
        committed = idea["title"].lower() in planned_titles
        value, why = score(idea, spec, committed, len(idea["sources"]))
        built_id = host_titles.get(idea["title"].lower())
        if built_id:
            idea["built"] = True
            value, why = score(idea, spec, committed, len(idea["sources"]))
        # A built surface is judged by the toggle a human gave it, not by the
        # title heuristics: the heuristics are for ideas nobody has built.
        if built_id and built_id in OPT_IN and value >= 6:
            value = 4
            why.append("opt-in by design (§59.10)")
        default_on = bool(built_id) and value >= 6 and built_id not in OPT_IN
        if built_id and not default_on:
            below.append((built_id, value, why))
        rows.append(
            {
                "why_summary": escape(idea["context"])[:180],
                # A requirement already has an id, and re-slugging its sentence
                # gave `idea.dh-watch-saved-query-exec-cmd-notify-ntfy-de`:
                # unreadable, and it hides that this is M63-12.
                "id": built_id or idea.get("row_id") or f"idea.{idea['slug']}",
                "title": idea["title"],
                "sources": idea["sources"],
                "milestone": idea["milestone"] or "unassigned",
                "relevance": value,
                "because": "; ".join(why),
                "default_on": default_on,
                "built": bool(built_id),
            }
        )

    rows.sort(key=lambda r: (-r["relevance"], r["id"]))
    rows = [r for r in rows if not r["built"]]
    JSON_OUT.write_text(json.dumps(
        {"generated_by": "scripts/ideas_catalogue.py", "count": len(rows), "ideas": rows},
        indent=1,
    ) + "\n")
    write_rust(rows)
    print(f"wrote {JSON_OUT.relative_to(REPO)}: {len(rows)} ideas")
    print(f"  ideas:         {len(rows)}")
    on = [h["id"] for h in host if h["default_on"]]
    off = [h["id"] for h in host if not h["default_on"]]
    print(f"  registry rows: {len(host)} — {len(on)} on, {len(off)} off")
    print(f"  off by default: {', '.join(sorted(off))}")
    hist: dict[int, int] = {}
    for r in rows:
        hist[r["relevance"]] = hist.get(r["relevance"], 0) + 1
    print("  relevance:", dict(sorted(hist.items(), reverse=True)))
    print("  sample:")
    for row in rows[:8]:
        print(f"    {row['relevance']}  {row['id'][:44]:<44} {row['title'][:50]}")
    for built_id, value, why in below:
        print(f"  OFF: {built_id} (relevance {value}) — {'; '.join(why)}")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
