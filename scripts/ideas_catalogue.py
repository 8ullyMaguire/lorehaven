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


# The coverage gate (§59.7's "must not silently lose items", made structural).
#
# The generator reads a NAMED list of sources. A document the list forgot
# contributes nothing, and nothing reports the omission — silence reads as "no
# ideas there", which is exactly how a roadmap stops being a roadmap. The gate
# therefore enumerates every candidate document the repository itself offers
# (tracked markdown whose name says brainstorm/ideas/roadmap, plus everything
# under docs/ideas/), extracts its idea titles with the SAME rules the catalogue
# uses, and requires each title to have a home: a row in the idea catalogue, a
# row the catalogue retired because it was built, or a module in the registry.
#
# The gate reads content, not mtimes: every candidate on this host is older than
# the JSON, so an mtime gate would pass three unread documents silently.

# Documents the gate must not read, each with the rule that makes its ideas
# already counted. The rule names WHERE the ideas live, never "trust me".
GATE_IGNORE = {
    # A summary OF the spec: every heading restates a section the spec already
    # declares, and the spec is not an idea source (its amendments are).
    "docs/spec-summary-brainstorm.md": "titles restate spec sections; spec amendments are the idea source",
    # An eight-line pointer: the network plan lives in ascend's tree, and this
    # file says so in its first sentence. Its title ("The network plan lives in
    # Ascend") is a statement about where work is scheduled, not a proposal.
    # The ideas it points at are scheduled in that other repo's plan file.
    "docs/ideas/NETWORK-PLAN.md": "a pointer to ~/code-local/rust/ascend/docs/PLAN-network.md, not an idea document",
}


def candidate_documents() -> list[Path]:
    """Every tracked markdown document the repository offers as an idea source."""
    tracked = subprocess.run(
        ["git", "ls-files", "docs/*.md", "docs/**/*.md"],
        cwd=REPO, capture_output=True, text=True, check=True,
    ).stdout.split()
    out: list[Path] = []
    for name in tracked:
        path = REPO / name
        if not path.is_file():
            continue
        stem = path.stem.lower()
        in_ideas_dir = path.parent.name == "ideas"
        name_says = any(w in stem for w in ("brainstorm", "ideas", "roadmap"))
        if in_ideas_dir or name_says:
            if path.name == "README.md":
                continue  # a directory index, not a brainstorm
            out.append(path)
    return sorted(set(out))


def roadmap_items(path: Path) -> list[str]:
    """A roadmap's adopted items, as idea titles.

    The guest-trust roadmap names its items "GS-1 … GS-5" inside its "What this
    site adopts" section. A bare "GS-3" is a label, so each item is expanded
    with the roadmap's own words for it — the section that defines it. The
    expansion is written here rather than scraped, because scraping prose for
    "the item GS-3 is about" is how a catalogue invents ideas nobody proposed.
    """
    text = path.read_text(encoding="utf-8", errors="replace")
    out: list[str] = []
    for m in re.finditer(r"\bGS-([1-5])\b", text):
        n = m.group(1)
        titles = {
            "1": "guest access with consent",
            "2": "the guest fold: reading progress and bookmarks move with the guest",
            "3": "guest identity as a first-class pseudonym",
            "4": "elastic trust from karma, not a parallel score",
            "5": "a jury drawn from the eligible pool",
        }
        t = titles.get(n)
        if t and t not in out:
            out.append(t)
    return out


def document_titles(path: Path) -> list[str] | None:
    """A document's idea titles, by the kind of document it is — or None when the
    gate must not read it at all.

    The rules mirror the catalogue's own extraction, one per kind:

    - a **sweep** (a brainstorm under docs/ideas/ or the vault) enumerates its
      `###` headings — the unit of an idea in a sweep is a heading;
    - a **named document** (an amendment, a design note, a gaps file, a roadmap)
      IS one idea: its title. Reading its internal headings would read its
      sections, which is the 375-row mistake;
    - **narration** (a handoff, a plan that records what happened, a session
      note) contributes nothing BY RULE — its headings are narration, and a gate
      that demanded rows from it would fail forever on correct behaviour. The
      exception is a plan whose name says it is an idea list (`100-ideas-*`):
      those are sweeps filed under the wrong directory, and the gate reads their
      headings.
    """
    name = path.name.lower()
    parent = path.parent.name
    # Narration, by directory kind. A handoff records what happened; a plan
    # records what to build — except the plans that ARE idea lists.
    if parent == "handoffs":
        return None
    if parent == "plans" and not any(w in name for w in ("ideas", "brainstorm")):
        return None
    if parent == "plans" and any(w in name for w in ("ideas", "brainstorm")):
        # An idea list filed under plans/: read its item headings, like a sweep.
        return headings(path, levels=(2, 3))
    if parent == "adr":
        return None  # a decision record argues ONE decision; it is not a backlog
    # Sweeps read their headings.
    if "brainstorm" in name or (parent == "ideas" and "sweep" in name):
        return headings(path, levels=(3,))
    # An audit's tables are its only idea-bearing part.
    if "audit" in name:
        return audit_absent() if name.startswith("audit-") else []
    # A roadmap's headings are sections; its ADOPTED items are the ideas. The
    # guest-trust roadmap names them "GS-1 … GS-5" in its adopts section, and
    # each is work nobody has registered. A roadmap that adopted nothing would
    # contribute its title alone, which is the named-document rule.
    if "roadmap" in name or parent == "roadmaps":
        titles = roadmap_items(path)
        return titles or [path.stem.replace("-", " ").title()]
    # Everything else is a named document: one idea, its title.
    return [path.stem.replace("-", " ").title()]


def coverage_gate(catalogue_slugs: set[str], retired_slugs: set[str],
                  registry_titles: set[str]) -> list[str]:
    """Titles with no home anywhere. Empty means the catalogue reads everything."""
    homeless: list[str] = []
    for path in candidate_documents():
        rel = str(path.relative_to(REPO))
        if rel in GATE_IGNORE:
            continue
        titles = document_titles(path)
        if titles is None:
            continue  # narration by kind; the gate does not demand rows from it
        for title in titles:
            if not words(title, 2):
                continue  # a label, not a proposal; the catalogue's own rule
            key = slug(title)
            if not key:
                continue
            if key in catalogue_slugs or key in retired_slugs:
                continue
            if title.lower() in registry_titles:
                continue
            homeless.append(f"{rel}: {title}")
    return homeless


def main() -> int:
    spec = spec_text()
    planned = requirement_rows()
    planned_titles = {t.lower() for _, t, _ in planned}
    planned_by_id = {i: m for i, _, m in planned}
    csv_rows = list(csv.DictReader((REPO / "docs/requirements.csv").open(encoding="utf-8")))
    rows_by_id = {r["id"]: r for r in csv_rows}
    planned_ids = {r["id"] for r in csv_rows if r["status"].strip() == "planned"}
    # Every requirement row, any status, keyed by its title. A row that left
    # `planned` was built — and an idea with the same title that another source
    # (a brainstorm, a design note) also mentions must not stay in the idea
    # catalogue as a planned row beside the module it became. Matching on the
    # title, because only the requirements pass carries the id and a built row
    # never reaches it.
    requirement_titles = {r["requirement"].split(":")[0].strip().lower(): r["status"].strip()
                          for r in csv_rows}

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
        if context and len(context) > len(entry["context"]):
            # Keep the longer context rather than concatenating: two documents
            # describing one idea say the same thing in different words, and
            # gluing them together produced summaries like "per-user module
            # overrides: …narrowing in the gate per-user module overrides: …".
            entry["context"] = context
        if row_id and not entry["row_id"]:
            # The merge must carry the row id onto a pre-existing entry: the
            # retirement check below matches on row_id, and an idea first seen
            # from a brainstorm keeps a stale `planned` row if its requirement row
            # never reaches it. M65-04 retired only because the requirements pass
            # created the entry first; the order of the sources decided it, which
            # is not a rule.
            entry["row_id"] = row_id
        if milestone and not entry["milestone"]:
            entry["milestone"] = milestone

    # 1. requirements.csv — the committed queue. Keep the row's id: re-slugging the
    #    sentence gave `idea.dh-watch-saved-query-exec-cmd-notify-ntfy-de`, which is
    #    unreadable and hides that this is M63-12 rather than a new idea.
    #
    # The context is the row's own evidence and notes fields — NOT the requirement
    # text, which `add` concatenates onto the title it already received. The first
    # version passed the requirement as context and every summary came out as the
    # same sentence twice, truncated: "per-user module overrides: module_user_state
    # table, PATCH /api/v1/modules/{id}, and the per-request narrowing in the gate
    # per-user module overrides: ...". A generated catalogue whose rows are
    # unreadable is one nobody reads.
    for rid, requirement, milestone in planned:
        row = rows_by_id.get(rid, {})
        context = " ".join(
            part.strip()
            for part in (row.get("evidence", ""), row.get("notes", ""))
            if part.strip()
        )
        add(requirement.split(":")[0], "docs/requirements.csv", context, milestone, row_id=rid)

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

    # The registry is the authority on what is built: a requirements row whose
    # milestone closed (M65-04 on this run) is a module now, and a planned row
    # beside the module it became is the second backlog §59.8 was written to end.
    # Such a row is dropped from the idea catalogue, and the drop is printed — a
    # row that leaves the queue silently is the same loss as one never read.
    built = {h["id"]: h for h in host}

    rows = []
    below = []
    retired = []
    for idea in ideas:
        committed = idea["title"].lower() in planned_titles
        value, why = score(idea, spec, committed, len(idea["sources"]))
        built_id = host_titles.get(idea["title"].lower())
        if built_id:
            idea["built"] = True
            value, why = score(idea, spec, committed, len(idea["sources"]))
        # A requirements row that has been built is retired from the idea
        # catalogue: it is a module row now, and §59.8's "no second backlog" is
        # about exactly this. Two matches: the row id (a requirement keeps its
        # own id into the registry), and the title, because a built row never
        # reaches the ideas list through the requirements pass — `planned` holds
        # only `status=planned` rows — and the same idea can arrive from a
        # brainstorm first. M65-04 retired on the title match: "per-user module
        # overrides" is `implemented-verified-e2e`, and the idea catalogue
        # listing it as planned was a second backlog in miniature.
        row_status = requirement_titles.get(idea["title"].strip().lower())
        if (idea.get("row_id") and idea["row_id"] not in planned_ids) or (
            row_status and row_status.startswith("implemented")
        ):
            retired.append((idea.get("row_id") or idea["title"][:40], idea["title"]))
            continue
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
                "why_summary": idea["context"][:180],
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

    # Coverage gate, before anything is written: a red gate produces no
    # artefacts, so a stale catalogue can never look current. The gate checks
    # that every idea title a candidate document offers has a home — a row here,
    # a row retired because it was built, or a module in the registry.
    cat_slugs = {slug(r["title"]) for r in rows}
    ret_slugs = {slug(t) for _, t in retired}
    reg_titles = {h["name"].lower() for h in host}
    homeless = coverage_gate(cat_slugs, ret_slugs, reg_titles)
    if homeless:
        print(f"coverage gate: {len(homeless)} idea title(s) with no home:", file=sys.stderr)
        for line in homeless[:40]:
            print(f"  - {line}", file=sys.stderr)
        if len(homeless) > 40:
            print(f"  ... and {len(homeless) - 40} more", file=sys.stderr)
        print("either the generator does not read that document (add its rule to "
              "candidate_documents/document_titles), or the document names an idea "
              "nobody has registered (add it to docs/ideas/ or requirements.csv). "
              "Silence here is how a roadmap stops being one.", file=sys.stderr)
        return 1
    JSON_OUT.write_text(json.dumps(
        {"generated_by": "scripts/ideas_catalogue.py", "count": len(rows), "ideas": rows},
        indent=1,
    ) + "\n")
    write_rust(rows)
    print(f"wrote {JSON_OUT.relative_to(REPO)}: {len(rows)} ideas")
    print(f"  ideas:         {len(rows)}")
    if retired:
        # The queue shrank, and a shrinking queue said nothing is how a milestone
        # closure quietly looks like an idea being abandoned.
        print(f"  retired (built): {len(retired)} — " + ", ".join(rid for rid, _ in retired))
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
