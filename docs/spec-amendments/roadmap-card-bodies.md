# Roadmap card bodies — the arena shows a title, the detail view shows a page

- Status: proposed
- Date: 2026-09-28
- Amends: spec §44 (Roadmap consensus), ADR 0023
- Plan: `docs/plans/roadmap-card-bodies.md`

## Context

The consensus arena (spec §44, ADR 0023) stores one row per feature idea in
`roadmap_cards`. The board renders the title, the Elo rating and the category,
and nothing else. That was sufficient when every card was a one-sentence
requirement lifted straight out of `docs/requirements.csv`, whose median
`requirement` cell is 47 characters.

It is no longer sufficient, for two reasons that arrived together.

**The seed source is changing.** The preservation brainstorm (162 ideas across
13 sections) is a *title plus a paragraph of rationale* per idea. Seeded as
they are, 162 paragraphs of careful argument collapse into 162 bare titles,
and the arena asks people to rank a list of labels whose reasoning nobody can
read. A MaxDiff ballot is a judgement about a thing, not about a phrase; a
voter who cannot see what a card proposes is not voting on the feature, they
are voting on how the phrase reads.

**An arena of titles is not a self-explaining surface.** The board is
anonymously readable, which is the point of it (§44.5). A stranger who
arrives at the board, has never read `docs/requirements.csv`, and wants to know
whether "Author's mirror declaration" is something the project intends to build
has exactly one place to find out: the card. Right now that place shows a
title, a number, and a category.

Writing the feature means the voter can judge it. §44.3 already assumes they
can — MaxDiff assumes a comparable judgement is possible per ballot, and
title-only cards make that judgement about phrasing.

## Decision

A card gains a **body**: a longer statement of what the feature is and why it
exists, on the order of a page. The board still lists title only. The detail
view shows title and body.

Four decisions follow, and each one is a choice rather than a consequence.

### 1. The body is a column, not a join

`roadmap_cards.body` — `TEXT NOT NULL DEFAULT ''`.

Not a separate `roadmap_card_bodies` table, and not a row in
`roadmap_suggestions`. A card's body is a property of the card: it is edited
with the card, travels with the card through a stage move, and is read on
every detail view. A side table would make every read a join and put the
question of what happens when a card has a row in one and not the other
permanently on the table. The column is `NOT NULL DEFAULT ''` so the 667
existing cards — and any card created by the suggest endpoint, which is
title-only — are valid without a backfill.

Empty is a real, supported state, not a defect. A card with no body renders a
board-quality placeholder, not an error and not a blank area.

### 2. Bodies live in `docs/requirements.csv`, and every row gets one

`docs/requirements.csv` becomes a 7-column file:

```
id,area,requirement,milestone,status,evidence,notes
```

becomes

```
id,area,requirement,body,milestone,status,evidence,notes
```

`body` is inserted directly after `requirement` so the file still reads as
"the requirement, then the elaboration of it" when opened in a spreadsheet.

Filling all 667 existing rows is a real cost and is stated as such here
rather than waved through: it is a day of work that produces no behaviour
change on its own. It is justified because the arena is a public surface and
a board of 667 title-only cards is a board where the argument for most of the
claims is invisible. The `notes` column is not a substitute and is explicitly
ruled out as one: `notes` is provenance (why a row claims the status it
claims), it is median 204 characters of build and test evidence, and it already
carries 2,671-character entries. It describes how the row was verified, not
what the feature is.

The CSV is the only source of truth for a body. The suggest endpoint (§44.5)
accepts a `body` on creation and stores it; it does not edit bodies of existing
cards, because community suggestion and operator curation are different acts
(§44.1: "A card's stage is the operator's decision").

### 3. The detail view is a route, and it is public

New route `GET /api/v1/roadmap/cards/:id`, public, no session. Returns the full
card including `body`.

The board keeps returning every card it returns today, and gains exactly one
field per card, `body`, so that clicking through never costs a second request
for data the board already loaded. The rationale is written into the API block
in §44.5 rather than left implicit, because a 667-card board with a page of
text per card is the first payload in this project large enough for that to
matter.

The alternative — board omits `body`, detail view fetches it — is strictly
worse and is recorded as considered-and-rejected: it trades one fat list
request for a fat list request *plus* a request per click, and it introduces
a visible loading state on the thing users are being asked to read.

The frontend adds `frontend/src/routes/RoadmapCard.svelte` with path
`/roadmap/:id`, registered in `frontend/src/lib/router.ts`, reachable from
each board card. The card element in `Roadmap.svelte` becomes a link rather
than a static article.

The arena ballot cards (§44.5) also gain `body`, on the same reasoning: a
voter choosing best of four is making the same judgement as a voter on the
board, and a ballot that hides the body is the ballot where it matters most.
A 4-card ballot is 4 pages, which is not a payload problem.

### 4. What is not changing

- Elo, stages, trust gating, the MaxDiff mechanic, and §44.6's
  never-downgrade rule are untouched. This is a presentation and storage
  change.
- The normalized-title dedup key is untouched. Bodies are prose and prose does
  not dedup; the ADR's "normalized-text matching only" stands.
- `roadmap_suggestions` is untouched. It is a queue for human triage
  (§44.1), not a body store.
- No Markdown rendering. `body` is stored and served as plain text and
  rendered as plain text. Bodies are written by the operator in the CSV, not
  by members, so the `{@html}` question (README §2.5) does not arise — but the
  next person to add member-authored bodies inherits that question, and the
  spec says so here so it is answered once.

## Consequences

- Two migrations (SQLite + PostgreSQL), both `0090_roadmap_card_body.sql`. Both
  dialects must carry identical ids: `the_two_dialects_define_the_same_migration_ids`
  fails otherwise.
- The `Card` struct, every `SELECT` of `roadmap_cards`, and `upsert_card` all
  grow a field. `SELECT *` is not used anywhere in `crates/db/src/roadmap.rs`,
  so each of the six queries is edited by hand.
- `scripts/seed_roadmap.py` gains the column in its insert and its update
  path, and its `--self-test` gains a check that every row's body is non-empty
  — the same class of check as the status-vocabulary check that already
  exists, for the same reason.
- `GET /api/v1/roadmap` grows from ~60 KB to ~600 KB on a 667-card board. This
  is accepted, not mitigated, and is recorded in §44.5.
- 667 bodies is a writing task that gates the arena's usefulness. The plan
  sequences it so that the *code* can land before the *prose*, and states the
  gate as a command.

## The unanswered question this defers

The preservation brainstorm is 162 ideas, of which 41 carry a checkmark. The
plan proposes seeding all 162 as `idea` cards and treating the checkmark as
"planned" — which is a claim about the project, not about the file, and is the
one thing in this amendment that the author of the brainstorm must decide
rather than have inferred.
