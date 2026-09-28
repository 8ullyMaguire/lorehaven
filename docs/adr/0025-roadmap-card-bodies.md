# ADR 0025: A roadmap card carries a body; the board lists titles only

- Status: proposed
- Date: 2026-09-28
- Amends: ADR 0023 (Roadmap consensus), spec §44
- Plan: `docs/plans/roadmap-card-bodies.md`

## Context

ADR 0023 ported FicHub's Elo/MaxDiff consensus engine and made
`docs/requirements.csv` the canonical feature inventory: every requirement row
is a card. At the time that was the right inventory, because a requirement row
*is* a card — one idea, stated in one sentence, median 47 characters in the
`requirement` cell. `roadmap_cards` therefore carries a `title` and nothing
else, and the board renders title, Elo and category.

Two things have changed since.

**The seed source is no longer sentences.** The preservation brainstorm
(2026-09-28) is 162 ideas, each a title plus a paragraph of argument, across
13 sections. Seeded into the existing shape, the argument has nowhere to go.
The arena would ask people to rank 162 bare labels, and a MaxDiff ballot
presupposes a comparable judgement is possible per ballot — "which of these
four is most valuable" is a question about features, and it becomes a question
about phrasing.

**The board is a public surface with an audience that has no other source.**
§44.5 makes the board anonymously readable, and the amendment's rationale is
transparency. A stranger who has never read `docs/requirements.csv` and wants
to know whether "Author's mirror declaration" is something the project intends
to build has exactly one place to look. Today that place shows a title, a
number, and a category.

The arena is the worst possible place for this gap, because the arena is
where the ranking happens. Elo is only as meaningful as the comprehension
behind it.

## Decision

A card gains a **`body`**: a longer statement of what the feature is and why it
exists, on the order of a page. The board lists titles only; the detail view
shows title and body.

Four choices, each of which was available to be made differently.

### 1. Column, not a side table

`roadmap_cards.body TEXT NOT NULL DEFAULT ''`. Not a
`roadmap_card_bodies` table, not a row in `roadmap_suggestions`.

A card's body is a property of the card. It is edited with the card, travels
with it through a stage move, and is read on every detail view. A side table
makes every read a join and leaves "a card with a body row and no card row, or
the reverse" permanently on the table — a state that is cheap to forbid and
easy to introduce.

`NOT NULL DEFAULT ''` is the whole compatibility story: 667 cards exist with
no body, and none of them may break. A `NULL` would force every reader to
branch and every writer to supply a value, in exchange for an absent/empty
distinction no surface renders differently. Empty is a real state and renders
as a placeholder.

### 2. Bodies in `docs/requirements.csv`, and every row gets one

`body` is inserted directly after `requirement` in the existing seven-column
file, so the file still reads as requirement-then-elaboration in a
spreadsheet.

Filling 667 bodies is a day of work that changes no behaviour on its own. It
is justified because the arena is public and a board where the argument for
most claims is invisible is a board that is not doing its job.

`notes` is explicitly **not** the column. `notes` is provenance — why a row
claims the status it claims — and already runs to 2,671 characters of build
and test evidence. It describes how a row was verified, not what the feature
is. Reusing it would bury 667 feature descriptions inside a provenance column
that readers have no reason to open.

### 3. Board carries the body; the detail route exists anyway

`GET /api/v1/roadmap` returns each card's body, and a new
`GET /api/v1/roadmap/cards/:id` (public) returns one card in full. The board
grows from ~60 KB to ~600 KB on 667 cards. Accepted, not mitigated.

The alternative — lean board, fetch the body on click — is strictly worse: one
fat request plus a request per click, and a visible loading state on the thing
the reader is being asked to read. At ~3,000 cards, pagination becomes the
answer; that is a number, and this is the milestone that should notice it.

Arena ballots carry the body too, on the same reasoning: the ballot is where
the ranking is actually made, and a 4-card payload is not a payload problem.

### 4. A checkmark is not a stage

The brainstorm marks 41 of its 162 ideas with a checkmark. Those seed as
`idea` — arena-eligible — and are recommended to the operator for promotion
through the existing audited `POST /api/v1/admin/roadmap/move` path.

`up_next` is frozen from the community's vote (§44.2). Seeding a checkmark
directly into it would remove 41 ideas from public ranking on the strength of
a character in a text file, and would make the arena's coverage of the
project's stated intentions a matter of file format. §44.1's division —
"Elo is the community's, stage is the operator's" — is the whole reason this
goes through the move endpoint and not the importer.

## Consequences

- Migration `0090_roadmap_card_body.sql` in both dialects. 0084 is **not**
  free: 0083 reserves it for `bot_registrations.token_id` (gap D5), and
  `the_two_dialects_define_the_same_migration_ids` fails on a mismatch. 0086 is
  also taken — by the in-flight M59 `body_audience` work — and 0087/0088 are
  claimed by the M59 plan, so the number is 0090.
- `the_two_dialects_define_the_same_migration_ids` compares the dialects to
  *each other*. It cannot see that two different changes both claim 0086; that
  surfaces later as a checksum collision in the migration ledger. Check the
  free number at the moment you start, not from a document.
- `the_two_dialects_declare_the_same_columns_and_indexes` does not parse
  `ALTER TABLE ... ADD COLUMN`. This migration is invisible to it. Recorded
  because a check that cannot see a class of change is a check whose silence
  is not evidence.
- The `Card` struct and every hand-written column list in
  `crates/db/src/roadmap.rs` grow a field. There is no `SELECT *` in that
  file, so each query is edited by hand, and every positional `row.get` below
  the new column shifts. `category` and `body` are both `String`, so a missed
  shift compiles and returns one column's value under the other's name.
- `find_card_by_id` is new. `find_card_by_title_normalized` loads every card
  and filters in Rust; at 667 cards with bodies that is a 600 KB table scan to
  answer a one-card question, and the detail route must not be built on it.
- `scripts/seed_roadmap.py` learns the column, and `--self-test` gains a
  non-empty-body check beside the status-vocabulary check it already carries.
- Non-CSV idea sources are imported by generator
  (`scripts/import_brainstorm.py` → `docs/ideas/preservation-brainstorm.csv`),
  never by hand-editing `requirements.csv`. The generator **imports**
  `normalize_title` from the seeder. Two implementations of the dedup key are
  two answers to "is this the same idea", and ADR 0023 names normalized-title
  matching as the single key the whole idempotence rests on.
- Bodies are plain text. No Markdown, no `{@html}`. They are operator-authored
  today; the rule is written down so that making them member-editable is a
  change to this ADR rather than an accident.
