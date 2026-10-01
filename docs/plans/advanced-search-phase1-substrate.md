# Advanced search: Phase 1 — the character/relationship substrate

> Status: **adopted**, superseding nothing. `advanced-search-external-reference.md`
> is a pasted reference and explicitly non-normative until reconciled;
> `advanced-search-as-built.md` is a target design that "builds on what you have".
> This plan takes the reconciliation notes in the reference document as the
> starting point, because they had already done the hard part: separating what the
> spec already covers from what is genuinely missing.

## What the reconciliation established, and what I checked

Re-verified against the tree on 2026-10-01 rather than taken on trust:

| claim | verified how | result |
|---|---|---|
| §15.3 defines `ExistsCharacter` / `ExistsRelationship` | `grep docs/spec.md` | **true** — `docs/spec.md:2365` |
| neither is in the built AST | `crates/domain/src/query.rs:451` | **true** — `QueryAst` has only `Text \| Phrase \| Fielded \| Comparison \| And \| Or \| Not` |
| no character/relationship schema exists | `grep migrations/` | **true** — only `taxonomy_nodes` rows with `kind='character'`/`'ship'` |
| journey 12 is an acceptance criterion | `docs/spec.md` §25.1 item 12 | **true** — "Search bound character attribute → exclude ship → filter by mood" |
| `work_tags` has no prominence | `migrations/sqlite/0011_taxonomy.sql` | **true** — `work_tags(work_id, node_id, weight, added_at)` |
| §15.17 already governs `review_status` | `migrations/sqlite/0082_taxonomy_review_status.sql` | **true** — columns exist; must not be reinvented |

So the blocker is exactly as the reference document said: **journey 12 is not
implementable today**, because §15.3's two nodes have no schema behind them. This
plan builds that substrate. Everything else in the target design (facets,
planner, semantic, MMR) sits on top of it and is out of scope here.

## Decisions taken (the reference document asks these back; these are the answers)

**1. Scale → SQLite FTS5 stays. No Elasticsearch, no Vespa.**
`advanced-search-as-built.md` §7 says to stay on SQL and put pgvector/sqlite-vec
behind a `VectorIndex` trait, and the reference's open question ("expected scale")
is answered by the architecture the spec already commits to: two dialects,
`migrations/sqlite/` + `migrations/postgres/`, with parity tests that tax every
feature. Adding a third engine would spend that budget on something the spec has
deliberately avoided. Recorded as ADR 0025.

**2. Reader-applied tags → NO, and the reason is §49.2.**
The reference proposes reader tags as one of three tag sources. Spec §49.2 is
emphatic: *"Only confirmed tags count toward gravity"* and confirmed means
author-confirmed. A reader-tag layer becomes a griefing vector the moment it
influences anything. This plan therefore introduces **no** reader-vote table, and
`work_tags` gains no reader-facing write path.

**3. ML-inferred metadata → NO. This is a contradiction, not a gap.**
The reference proposes machine-inferred tropes/POV/tense as filterable. Spec
§47.10 refuses learning-to-rank and embeddings in the ranking path, and §49.9
reiterates: *"No learning-to-rank, no embeddings, no ML."* §49.3 — the section
M45-14 implements — makes the same refusal concrete: *"an embedding is a fifth
that costs the instance CPU it may not have."*
Adopting inferred tags would need an explicit spec amendment, not a merge. Not
adopted. Note this is consistent with the just-built work coordinates: they are
four deterministic arithmetic measures precisely *because* no model is permitted.

**4. Prominence is adopted** — it is a genuine gap and it serves journey 12
directly. "Exclude Major Character Death" means something different when the tag is
incidental.

**5. Ship identity is a participant set; the relationship type belongs to the
work.** The same pair can be romantic in one fic and platonic in another, so
"Spike/Spike" cannot be the unit of relationship truth.

## Schema (migration 0103, both dialects)

```sql
-- Character attributes, §15.1: prominence, roles, attributes
work_characters(work_id, character_node_id, prominence, is_pov)

-- A ship is a participant SET. Identity = the sorted set, so "A/B" and "B/A"
-- are one node and no writer has to pick an order.
ship_participants(ship_node_id, character_node_id)

-- The relationship TYPE belongs to the work, not the ship (§15.2).
work_relationships(id, work_id, ship_node_id, rel_type, prominence, dynamics)

-- Tag prominence + source, per the as-built doc's §2.
ALTER work_tags ADD prominence / source / confidence
```

### The correctness rule §15.3 states and this schema exists to satisfy

> "Compile bound predicates into SQL `EXISTS` clauses. **Never allow one
> character to satisfy another character's attributes.**"

That is a statement about **correlation**: every clause of an `ExistsCharacter`
must test the *same* `work_characters` row. A naive three-`EXISTS` compilation —
one for the character, one for each attribute — lets work A satisfy "character X"
while a *different* row satisfies "attribute vampire", which is a different work's
vampire. So the compiler emits a **single** `EXISTS` with the attributes as
correlated `AND`s on `work_characters` itself, and the test suite includes a
fixture that fails if that is ever split.

### Dialect notes (checked against the migrations, not assumed)

The rule in this repo is that a column's type follows its **foreign key**, never
its neighbour's spelling — and here the two point in opposite directions, which is
exactly the trap:

| column | SQLite | PostgreSQL | because |
|---|---|---|---|
| `taxonomy_nodes.id` | TEXT | **TEXT** | 0011 declares it TEXT on *both* |
| `works.id` | TEXT | **UUID** | 0003 |
| `work_tags.work_id` | TEXT | **UUID** | follows `works` |

So `character_node_id`, `ship_node_id` and any `node_id` take **no cast at all**
on either engine, while `work_id` takes `::uuid` on PostgreSQL. Guessing "nodes are
UUIDs because they are ids" would have been wrong in both directions, and only the
migration files settled it.

## Build order for this plan — and what actually shipped

All five steps are done. Commits `31e875d`, `f4f0455`, and the query-language one.

1. **Migration 0103, both dialects + parity gate.** Done. The parity test compares
   declared column *sets*, so it cannot tell whether PostgreSQL accepts the DDL;
   `scripts/probe-character-substrate-pg.sh` applies it to a real PostgreSQL 15 and
   exercises §15.3's rule.
2. **`work_characters` / `ship_participants` / `work_relationships` store.** Done,
   `crates/db/src/work_characters.rs`.
3. **`ExistsCharacter` / `ExistsRelationship` AST + compiler arms.** Done, plus the
   decision that work-scoped nodes are *refused* in forum and user rendering rather
   than ignored.
4. **Both-dialect tests including the correlation fixture.** Done.
   `one_character_does_not_satisfy_another_characters_attributes` runs the correct
   and the incorrect compilation side by side and asserts they differ — 0 and 1 —
   so it fails if the fixture ever stops exercising the trap.
5. **Reachable from the query language.** Done, but *not* as this plan originally
   proposed.

### Step 5 changed, and why

The first draft of this step said to add `character:(with:"X" attributes:"vampire")`.
That would have been a spec amendment. §15.4 fixes the surface as flat fields:

```text
title: author: fandom: character: relationship: tag: mood:
```

and §15.4's own examples are flat (`mood:comfort AND status:complete`). A nested
grammar also has a specific hazard here: `character:Alice attribute:vampire` as two
implicitly-conjuncted terms is *exactly* the uncorrelated compilation §15.3 forbids
— the reader would be able to write the bug the schema exists to prevent, and the
two characters would silently not have to be the same one.

So the fields were redirected instead, which turned out to matter more than the
syntax:

* `character:X` read `work_tags` with `kind = 'character'`. **Nothing in the tree
  ever wrote such a tag** — verified by grep; 0103 is the only migration that names
  that kind at all. So `character:Alice` matched nothing. It now reads
  `work_characters`, joined to `taxonomy_nodes` to resolve the reader's name to a
  node id inside the statement, which keeps the parser pure as §15.4 requires.
* `relationship:"Alice/Bob"` read a ship *tag*, and the same argument applied. It
  now compares the ship's participant set against the names given.

The parser is unchanged: `character:Alice` still produces `Fielded(Character,
"Alice")`, which is what §15.4's "the parser produces the same typed AST as the
visual filter builder" actually requires.

### `relationship:` needed three clauses, not two

`A/B` is the participant set, so the query asks whether some ship has *exactly*
those participants. Measured on real PostgreSQL 15, two clauses are not enough:

* (a) every participant of the ship is named in the value
* (b) no participant of the ship is unnamed in the value
* (c) **every name in the value is a participant of the ship**

With only (a) and (b), asking for the poly `{alice,bob,carol}` also matched the duo
`{alice,bob}`: alice and bob *are* named, so (a) holds, and the duo has no unnamed
participant, so (b) holds. Only (c) rejects it.

Clause (c) is per-name, so the names are split **in Rust** and bound one each. The
alternatives were measured and rejected: `string_to_array` does not exist in SQLite,
`instr` does not exist in PostgreSQL (it fails with `function instr(unknown, text)
does not exist`), and counting separators with `length - length(replace(...))` is
wrong for any name containing a space.

The membership clauses share one bind of the joined value, delimited by `//`. The
double separator is load-bearing: with a single `/` the trailing participant is
followed by `%` rather than `/`, so `LIKE '%/bob/%'` is false for it and a duo stops
matching itself. `//` also keeps `alice` from matching `alice_v2`.

## Out of scope, deliberately

Facets, the `Plan` refactor, pgvector, MMR, NL→AST, and the omnibox are all in
`advanced-search-as-built.md` and all sit *downstream* of this substrate. Doing
them first would mean building retrievers with nothing to retrieve.

## Out of scope, deliberately

Facets, the `Plan` refactor, pgvector, MMR, NL→AST, and the omnibox are all in
`advanced-search-as-built.md` and all sit *downstream* of this substrate. Doing
them first would mean building retrievers with nothing to retrieve.
