# ADR 0026: Search stays on SQL; no reader tags, no inferred tags

- Status: accepted
- Date: 2026-10-01
- Amends: nothing; constrains `docs/plans/advanced-search-*.md`
- Plan: `docs/plans/advanced-search-phase1-substrate.md`

## Context

Two design documents arrived on 2026-10-01:

- `docs/plans/advanced-search-external-reference.md` — an external design
  reference for fanfiction search, pasted verbatim and explicitly marked
  non-normative "until reconciled". Its author closes by asking three questions:
  expected scale, reader-applied tags yes/no, and the stance on ML-inferred
  metadata.
- `docs/plans/advanced-search-as-built.md` — a target design that "builds on what
  you have rather than replacing it", with a six-phase build order.

The three questions the reference asks back are not open preferences. Each one
conflicts with a decision this project has already made and would have to undo.

## The three answers

### 1. Scale: SQLite FTS5 stays; no Elasticsearch, no Vespa, no third engine

The spec commits to two dialects — `migrations/sqlite/` and
`migrations/postgres/` in lockstep — and pays for it with parity tests that tax
every feature (`the_two_dialects_declare_the_same_columns_and_indexes`, the
two-dialect test suites, the capability notes in each store module). That is a
real, recurring cost.

`advanced-search-as-built.md` §7 already concludes the same thing ("stay on SQL…
nothing here needs Elasticsearch") and puts vector search behind a `VectorIndex`
trait so the boundary is swappable later if scale demands it. Agreeing with the
reference here costs nothing and defers the question honestly.

**Consequence:** the `Plan` boundary is where a future engine would enter, and
nothing above it would change. No third engine is introduced now.

### 2. Reader-applied tags: PARTLY — confidence yes, gravity no

The reference proposes three tag sources — author-applied, reader-applied, and
machine-inferred — each with a confidence level.

Spec §49.2 says: *"A tag contributes to gravity **only if it is reader- or
wrangler-confirmed.** Author-applied and machine-inferred tags are stored and
displayed, and are not counted."*

**This ADR originally refused reader tags outright and that was wrong on two
counts**, both found by re-reading the section rather than its headline:

* §49.2 already *names reader confirmation* as one of the two qualifying kinds. A
  reader layer is not something the spec excludes; it is something it anticipates.
* The section's own scope is **gravity**, not display and not search. A reader
  vote that never touches gravity is not a griefing vector, because it changes
  nothing other readers see.

What is genuinely refused is a reader layer that feeds **ranking or any aggregate**.
So the split is:

* **Adopted:** `work_tag_votes` (migration 0105) — one vote per
  (work, tag, pseudonymous voter), CHECKed to -1|0|1, feeding
  `work_tags.confidence` only. `voter_pseud_id` has no reversible mapping
  anywhere, per §11.17's rule that a reader headcount is not derivable.
* **Refused:** any reader contribution to gravity, ranking, or counts of readers.
  `work_tag_votes` has no foreign key into any ranking input, and §49.2's
  contribution cap stays as specified.
* `work_tags.source` remains constrained to `'author'` in the database, so
  "a reader tag is never a gravity input" is an invariant rather than a convention
  every write path must remember. **Reader *confirmation* of an author tag is a
  separate, still-unspecified question — see the amendment in §15.4a.**

### 3. ML-inferred tags and embeddings: DEFERRED, not refused

The reference proposes inferred tropes/tone/POV/tense as filterable, plus
embedding search with pgvector.

§47.10 and §49.9 both refuse it, and **this ADR originally quoted them as a blanket
ban. They are not.** Both are scoped, and both name the same reason:

> §47.10: *"adding one now would mean the offline evaluation in §47.3 has nothing to
> evaluate yet."*
> §49.9: *"A model added here would have no evaluation data until §47.3's propensity
> logging has run."*

So the refusal is **"not until §47.3's evaluation harness exists"**, not "never".
That is a sequencing constraint, and this project can satisfy it: §7's eval
harness (`search_query_log`, nDCG@k, zero-result rate) is unbuilt work, not a
permanent bar. Phase 5 of the as-built plan is therefore **deferred, not cancelled**.

What is true, and is the strongest evidence for the arithmetic-first preference:
§49.3's work coordinates (M45-14, `crates/domain/src/coordinates.rs`) are **four
arithmetic measures over text**, and §49.3 gives the reason: *"an embedding is a
fifth that costs the instance CPU it may not have."* That is why those coordinates
are byte-for-byte reproducible and why their tests pin determinism.

**Consequence for now:** no `work_vibe` inferred axes and no vector store in the
retrieval path. `work_passages.model_version` (migration 0105) exists anyway, because
if embeddings are ever added, a passage computed by an unlabelled model is
indistinguishable from one computed by a different model, and the search returns
confident nonsense. `work_index_policy.allow_embedding` defaults to **0**, so the
deferral is enforced by a database default rather than by intent.

The amendment in §15.4a makes the sequencing explicit so a future session does not
have to re-derive it.

## What *is* adopted

Not everything in the reference is refused. Two items are genuine gaps and are
adopted in Phase 1 (`docs/plans/advanced-search-phase1-substrate.md`):

- **Tag prominence** (`primary` / `secondary` / `background`). "Exclude Major
  Character Death" means something different when the tag is incidental, and
  journey 12 depends on it.
- **Structured relationships**: a ship is a participant *set* and the relationship
  *type* belongs to the work. The same pair can be romantic in one fic and
  platonic in another, so the ship node cannot be the unit of relationship truth.

The reconciliation in the reference's own closing notes is what made this
tractable: it had already separated "already specified" from "genuine gap" from
"do not adopt silently". The one thing it could not settle — that §15.3's
`ExistsCharacter` / `ExistsRelationship` have no schema and no implementation —
was re-verified against the tree and is the actual work.

## Consequences

- `docs/plans/advanced-search-*.md` are target designs, not commitments. Phase 1
  is the substrate; the rest is downstream of it.
- Anything in the reference requiring ML is deferred indefinitely, and saying so
  in the spec is preferable to a comment in a plan.
- The dialect cost is unchanged: two engines, parity-tested.
