# Advanced search, Phase 2 — the query planner

- Status: accepted
- Date: 2026-10-01
- Amends: spec §15.3, §15.4 (adds §15.4.1)
- Plan: `docs/plans/advanced-search-as-built.md` (Phase 2)
- Milestone: M46-05
- ADR: `docs/adr/0026-search-stays-on-sql.md`

## Context

Phase 1 of the plan (M46-01 through M46-04) built the schema the search design sits
on: `work_characters` / `work_relationships` / `ship_participants`, the taxonomy
graph (`taxonomy_node_scope`, `taxonomy_edges`, `taxonomy_closure`), `work_warnings`,
`work_tag_votes`, `work_index_policy` and `work_passages`. The substrate existed and
the query language could not reach most of it.

Three specific gaps, each verified by grep against `docs/spec.md` and the
implementation tree rather than inferred:

1. **`character:` and `relationship:` matched nothing.** Both compiled to
   `work_tags` lookups for a `kind` nothing in the tree ever wrote. Fixed in M46-01.
2. **`ExistsCharacter` / `ExistsRelationship` had no schema and no AST node**, so
   journey 12 — an acceptance criterion — was not implementable. Fixed in M46-01.
3. **The grammar had no way to express a correlated predicate, a quantified
   disjunction, an expansion, or a cost bound.** §15.3 specifies the rule that one
   character must never satisfy another character's attributes; §15.4's surface could
   not state it. That is this amendment.

## What is adopted

| # | Mechanic | Why |
|---|---|---|
| 1 | **Scoped sub-predicates** `ship:(with:"A" with:"B" type:romantic)` | The only way to make §15.3's correlation rule reachable from text. Measured: the uncorrelated form matches a fixture it must not. |
| 2 | **`min_match(n, …)`** | §15.7 is conjunctive only; "any two of these three" was inexpressible. |
| 3 | **Expansion** `tag:"X"+children^2` over the materialised closure | "Fake Dating" should reach its children without a recursive CTE. |
| 4 | **Typed values and ranges** `words:20k..100k`, `kudos_pct(fandom):>80` | Category errors stay typed instead of becoming no-op string comparisons. |
| 5 | **Complexity budget** | Search is rate-limited, which bounds frequency, not the cost of one query. |
| 6 | **Pretty-printer + span diagnostics + `query_version`** | Chips, share links and saved views all render a query back to a reader. |

## What is refused, and why

**`Ref(...)` as a separate term.** §15.4 already specifies query aliases: private,
expanded before parsing, unable to shadow built-ins, expandable in the interface. The
proposal's `Ref` is the same idea under a new name. Two spellings of one mechanism
is how they come to mean different things, so `Ref` resolves through the alias table
and adds nothing.

**General regex.** §15.5's answer to a misspelled name is a *suggestion the reader
confirms*, never silent correction. A regex field is a silent-wildcard machine with a
different failure mode, and a registered function with a statement timeout is a
separate decision with its own abuse analysis.

## What is deferred, not refused — and the correction this forced

The first version of ADR 0026 refused embeddings and inferred tags outright, quoting
§47.10 and §49.9 as a blanket ban. Re-reading them in full, both are **scoped, and
both name a sequencing constraint rather than a permanent bar**:

> §47.10: *"adding one now would mean the offline evaluation in §47.3 has nothing to
> evaluate yet."*
> §49.9: *"A model added here would have no evaluation data until §47.3's propensity
> logging has run."*

So semantic search is **deferred until §47.3's eval harness has data**. The
deferral is enforced structurally rather than by intention:
`work_index_policy.allow_embedding` defaults to `0`, and `work_passages` carries
`model_version` so a future re-embed is never mistaken for a search across mixed
vector spaces.

**Reader tags were corrected the same way.** ADR 0026 refused a reader-tag layer
entirely. §49.2 in fact says a tag counts toward gravity *"only if it is reader- or
wrangler-confirmed"*, and scopes itself to **gravity** — not to display, not to
search. A reader vote that never touches gravity is not a griefing vector.
`work_tag_votes` therefore ships (M46-04) feeding `confidence` only, with no path
into gravity, ranking, or any reader count; `work_tags.source` stays constrained to
`'author'` so that is a database invariant.

What a reader's *confirmation of an author tag* does remains unspecified, and is
recorded as an open question rather than designed here.

## The standing lesson from Phase 1

Three diagnoses in this project were confidently wrong on inspection and were
corrected only by tests that contradicted them. Every claim in this document is
marked with the evidence that supports it, and where a claim rests on a spec section
the section's own words are quoted — because a paraphrase is what produced the
mistakes.
