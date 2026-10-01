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

### 2. Reader-applied tags: NO

The reference proposes three tag sources — author-applied, reader-applied, and
machine-inferred — each with a confidence level.

Spec §49.2 is unambiguous: *"Only confirmed tags count toward gravity"*, and
confirmed means author-confirmed. §15.17 then builds a whole review lifecycle for
entities that arrive from outside, precisely so unverified names are **visible as
unverified** rather than folded in.

A reader-tag layer is not merely unadopted in this ADR; it would have to be
hard-excluded from gravity, confidence, ranking, and every aggregate, or it
becomes a griefing vector: any account could attach any tag to any work and
influence what other readers see. The exclusion would need enforcing in every
write path forever.

**Consequence:** no `work_tag_votes` table. `work_tags` gains no reader-facing
write path. If reader tags are ever wanted, that is an amendment to §49.2, not a
schema detail.

### 3. ML-inferred tags and embeddings: NO

The reference proposes inferred tropes/tone/POV/tense as filterable, plus
embedding search with pgvector.

Spec §47.10 refuses learning-to-rank in the ranking path; §49.9 reiterates:
*"No learning-to-rank, no embeddings, no ML."* This is not a preference about
quality — it is about what a self-hosted, single-instance, volunteer-run archive
can promise its readers.

The most concrete evidence is already in the tree. §49.3's work coordinates
(M45-14, `crates/domain/src/coordinates.rs`) are **four arithmetic measures over
text** — sentence-length variance, dialogue ratio, type-token ratio,
chapter-length spread — and §49.3 gives the reason: *"an embedding is a fifth
that costs the instance CPU it may not have."* The same sentence is why those
coordinates are reproducible byte-for-byte and why their tests pin determinism
rather than tolerance.

**Consequence:** no `work_vibe` inferred axes, no `model_version` columns, no
vector store. Where the reference wants a learned signal, this project wants an
arithmetic one — and says so in the spec.

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
