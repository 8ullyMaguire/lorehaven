# Advanced search: target design

This builds on what you have rather than replacing it. Your strengths are the typed AST, the compile-to-`EXISTS` approach, per-viewer filtering before `LIMIT`, and versioned saved queries. Your gaps are in the schema (relationships, prominence, graph edges) and in the retrieval layers (semantic, facets, derived stats).

**Target shape:**

```
 text / chips / NL ──► parser ──► AST (versioned) ──► planner ──► Plan
                          ▲            │                           │
                  pretty-printer ◄─────┘            ┌──────────────┼───────────────┐
                                                    ▼              ▼               ▼
                                             SQL predicate   lexical (FTS)   vector (ANN/exact)
                                          (visibility+filters   + snippets    + similar_to / sem:
                                           + fields, ONE source of truth)
                                                    └──────────────┼───────────────┘
                                                                   ▼
                              fuse (RRF) → score features → rerank (MMR) → explain
                                                                   ▼
                       Response { results, facets, chips, explanations, relaxations }
```

Three invariants hold the design together:

1. **One predicate.** Visibility, viewer filters, and query fields compile into a single SQL predicate. Results, facet counts, relaxation counts, alerts, and ANN candidates all reuse it, so nothing can leak by construction.
2. **Live checks.** Visibility and blocks are always checked against source tables at query time, never trusted from an index. Slow indexing lanes (embeddings, inferred tags) can lag without ever leaking.
3. **Nothing hidden.** Everything fuzzy (NL parsing, inferred tags, semantic terms, expansion) is shown as an editable AST term.

---

## 1. Principles

1. **One AST, many retrievers.** Keep your three engines, but put them behind one `SearchEngine` trait and one planner. Don't merge them into one table.
2. **Filters are exact and semantics are for ranking.** Semantic terms score by default and filter only when explicitly wrapped in `must(...)` with a threshold. This keeps Boolean semantics honest.
3. **Exclusion is first-class**, including the distinction between "declared absent" and "never declared" (§2).
4. **Popularity is normalized and smoothed**, scoped by fandom, ship, or the whole site.
5. **Every fuzzy thing is labeled** with its source, confidence, and a toggle.
6. **Capabilities degrade by instance.** `SearchCapabilities { vector, trigram, ... }` lets a small SQLite instance run without ANN while Postgres instances get everything.

---

## 2. Tag system and schema (the foundation)

This is the blocker for your journey 12 and for most of what follows, so do it first. Sketches below are dialect-neutral. Both dialects get the same logical tables, as in your existing migrations.

### Taxonomy

```sql
-- existing taxonomy_nodes, extended
ALTER taxonomy_nodes ADD status TEXT NOT NULL DEFAULT 'active'; -- active|pending|merged|deprecated
ALTER taxonomy_nodes ADD merged_into INTEGER NULL;
-- kind widened: fandom, character, ship, trope, genre, mood, setting, format,
-- pov, tense, warning, meta, freeform

-- Fandom scoping (many-to-many: crossovers, shared universes)
taxonomy_node_scope(node_id, fandom_node_id, PRIMARY KEY(node_id, fandom_node_id))

-- Graph edges (DAG for parent/implies; cycle check on write)
taxonomy_edges(src_id, dst_id, rel, created_by, PRIMARY KEY(src_id,dst_id,rel))
  -- rel: parent | implies | related | adjacent_fandom

-- Materialized closure so expansion is a join, not a recursive CTE
taxonomy_closure(ancestor_id, descendant_id, rel, depth,
                 PRIMARY KEY(ancestor_id, descendant_id, rel))
```

> **As built (M46-03).** Two corrections, both verified against real engines rather
> than reasoned about:
>
> * `merged_into INTEGER` → **TEXT**. `taxonomy_nodes.id` is TEXT on both dialects,
>   and PostgreSQL 15 refuses the plan's DDL outright: *"foreign key constraint
>   cannot be implemented / Key columns are of incompatible types: integer and
>   text"*.
> * `status` is **separate from** `review_status` (0082's `'unverified' | 'curated'`).
>   They are different axes — curation versus lifecycle — and overloading one column
>   makes "curated but merged" inexpressible. `status` is `'active' | 'pending' |
>   'merged' | 'deprecated'`, with `merged_into` set if and only if `status =
>   'merged'`.
>
> The SQLite half enforces the two ALTER-able constraints with **triggers**, not
> `ADD CONSTRAINT`: the SQLite the application links is 3.46.0 (bundled by
> `libsqlite3-sys 0.30.1`) and `ADD CONSTRAINT` arrived in 3.50.0. The system
> `sqlite3` CLI here is 3.53.4 and *does* accept it, so a CLI-only check passes
> against a migration the default engine cannot apply — that is how this was caught.
> PostgreSQL keeps real CHECKs. See `migrations/sqlite/0103_character_relationships.sql`
> for the same idiom and the same reasoning.

- **Scoping** is what lets "Spike (Buffy)" and "Spike (Cowboy Bebop)" coexist, and it makes the omnibox disambiguation real.
- **Closure** keeps `tag:"Fake Dating"+children` as `descendant_id IN (SELECT ... FROM taxonomy_closure WHERE ancestor_id=?)`. This works identically in SQLite and Postgres and avoids huge `IN` lists.
- **Implications** are off by default in search and only apply when the user toggles them, because bad curator edges would otherwise silently distort every query.

### Work-level tagging

```sql
ALTER work_tags ADD prominence TEXT NOT NULL DEFAULT 'secondary'; -- primary|secondary|background
ALTER work_tags ADD source     TEXT NOT NULL DEFAULT 'author';    -- author|reader|inferred|curator
ALTER work_tags ADD confidence REAL NOT NULL DEFAULT 1.0;
ALTER work_tags ADD status     TEXT NOT NULL DEFAULT 'applied';   -- applied|suggested|rejected
-- backfill prominence from existing `weight` with a documented threshold

work_tag_votes(work_id, node_id, voter_pseud_id, vote)   -- reader layer, aggregated into confidence
```

### Characters and relationships

The key modeling decision: **a ship node is a participant set, and the relationship type belongs to the work**, because the same pair can be romantic in one fic and platonic in another.

```sql
ship_participants(ship_node_id, character_node_id, PRIMARY KEY(...))
  -- canonical ship identity = sorted participant set; auto-create on first use

work_characters(work_id, character_node_id, prominence, is_pov BOOLEAN)

work_relationships(id, work_id, ship_node_id, rel_type, prominence, label)
  -- rel_type: romantic | platonic | familial | qpp | sexual | antagonistic | other
```

This enables:

| Query | Compiles to |
|---|---|
| any romantic pairing involving X | `EXISTS(work_relationships wr JOIN ship_participants sp … WHERE sp.character=X AND wr.rel_type='romantic')` |
| X with anyone except Y | the same, plus `NOT EXISTS` for participant Y within the same relationship row |
| **Journey 12: X present, no relationship involving X** | `EXISTS(work_characters X) AND NOT EXISTS(relationship involving X)` |
| primary ship only | add `wr.prominence='primary'` |

> **As built (M46-04).** The plan says to re-key `content_warnings`; it is a
> **sibling table, `work_warnings`**, not a re-key. `content_warnings.post_id` is a
> *forum post* (TEXT, referencing `forum_posts(id)`), written and read by
> `crates/db/src/spoilers.rs` behind `routes/spoilers.rs` and pinned by five tests in
> `milestone_34_spoilers.rs`. Re-keying would mean a table rebuild with a live route
> behind it — the operation 0082, 0103 and 0104 all decline on the same grounds.
>
> The two answer different questions: a reader's free-text mark on a post versus an
> author's taxonomy-typed declaration about a work or chapter. Keeping them separate
> also stops a work warning being silently attached to an unrelated post.
>
> `declaration` keeps `none_apply` and `creator_chose_not_to_say` **distinct**:
> collapsing them would let a creator's silence read as a clean bill of health.
> Uniqueness over the nullable `chapter_id` is `UNIQUE NULLS NOT DISTINCT` on
> PostgreSQL and a unique index on `coalesce(chapter_id, '')` on SQLite, because an
> expression inside a table-level UNIQUE constraint is rejected on SQLite
> ("expressions prohibited in PRIMARY KEY and UNIQUE constraints").
>
> Also in 0105: `work_tag_votes` (the reader layer behind `work_tags.confidence`, keyed
> on a pseudonymous id with no reversible mapping per §11.17) and `work_index_policy`
> plus `work_passages`. `work_index_policy.allow_embedding` defaults to **0**, because
> §47.10/§49.9 forbid inferred data influencing anything.

### Warnings

Re-key `content_warnings` from `post_id` to works and chapters:

```sql
work_warnings(work_id, chapter_id NULL, warning_node_id, severity,
              depiction,        -- on_page | referenced | implied
              declaration,      -- declared | none_apply | creator_chose_not_to_say | reader_flagged
              spoiler_safe_note TEXT)
```

The `declaration` column fixes a common search failure. "Exclude MCD" should be able to exclude works that *declare* MCD and, optionally, works that *declined to say*. These need separate toggles (`warnings:strict`), and the UI should show the difference.

### Derived and inferred data

```sql
work_vibe(work_id, axis, value, confidence, source, model_version)  -- replaces filtering on TEXT vector
work_metrics_derived(work_id, kudos_per_hit, comments_per_kudos, bookmark_rate,
                     pct_site, pct_fandom, pct_ship, velocity_7d, velocity_30d,
                     hidden_gem, median_update_gap_days, next_update_expected, computed_at)
metric_distributions(scope_node_id NULL, metric, p10,p25,p50,p75,p90,p99, n, computed_at)
author_signals(pseud_id, completion_rate, median_gap_days, last_post_at, abandonment_risk)
taxonomy_cooccurrence(a_id, b_id, scope_node_id, count, pmi)
work_index_policy(work_id, policy)    -- full | metadata_only | link_only
```

Your 5-axis taste vector stays and becomes the **interpretable vibe layer**. It is stored in a filterable form, so "angst ≥ 0.7" is a plain `Comparison`.

---

## 3. Query capabilities

### B. Query language v2 (build this before semantic)

**New AST nodes**

| Node | Purpose |
|---|---|
| `Range(field, lo, hi)` | `words:20k..100k`, `updated:30d..1y` |
| `Scoped(field, Vec<SubPredicate>)` | conditions that apply to the *same row* (correlated `EXISTS`) |
| `Boost(node, f32)` | `^2` |
| `MinMatch(n, Vec)` | at least N of these |
| `Expand(term, ExpandMode)` | exact / synonyms / children / implied |
| `Me(MeField)` | viewer-relative: `me:read`, `me:kudosed`, `me:bookmarked`, `me:dnf`, `me:subscribed`, `me:following_author` |
| `Ref(Set)` | `in:collection:…`, `in:list:…`, `bookmarked_by:@u` |
| `Semantic(text, mode)` | `sem:"…"`, a scoring term, or filtering when inside `must()` |
| `SimilarTo(pos, neg)` | `similar_to:(W1,W2,-W3)` |
| `Phrase / Proximity` | `"exact line"`, `"a b"~5` |

`ValueKind` gains `Duration`, `Percentile`, `Count`, and `Ref`, so category errors stay typed.

**Scoped sub-predicates are the key correctness feature.** Without them, `ship:A/B AND type:romantic` can match two different relationships:

```
fandom:"Star Wars"
AND ship:(with:"Obi-Wan Kenobi" with:"Anakin Skywalker" type:romantic prominence:primary)
AND tag:"Slow Burn"+children^2
AND NOT warning:("Major Character Death" depiction:on_page)
AND warnings:strict
AND words:20k..100k
AND status:complete AND updated:<2y
AND kudos_pct(fandom):>80
AND NOT me:read AND NOT me:dnf
AND min_match(2, tag:"Found Family", tag:"Hurt/Comfort", tag:"Banter")
AND sem:"stuck in a lighthouse during a storm"
```

Journey 12 is `character:"X" AND NOT ship:(with:"X")`.

**Other language decisions**
- **Regex:** drop general regex. Offer wildcard and fuzzy via `pg_trgm`, and on SQLite via your term table. Allow regex only on entity-name fields, using a registered function with a statement timeout.
- **Pretty-printer:** add a canonical `ast → text` printer. Chips, URLs, saved views, and recipes all depend on it. Test `parse(print(ast)) == ast` with property tests and golden files.
- **Saved views:** `query_version` bumps with grammar changes, and old views are migrated by AST transform rather than by re-parsing text.
- **Errors:** return span-based diagnostics ("unknown field `fandm`, did you mean `fandom`?") so the omnibox can underline them.

### A. Omnibox and entity engines

Introduce `EntityKind` and a `SearchEngine` trait with `parse_fields()`, `compile()`, and `rank()`. Implement it for the engines you already have and add the missing ones:

| Entity | Fields beyond today's |
|---|---|
| Taxonomy (fandom, character, ship, tag) | kind, scope fandom, size, 30/90d trend, parents/children, co-occurring, "similar" |
| Collection / challenge / prompt | open/closed, deadline, rating cap, fandom, theme, prompts remaining, anonymity state |
| Series | complete, length, last update |
| User | fandoms written, ships, languages, cadence, open_to (prompts/commissions/beta/gifts) |
| Bookmark / rec list | bookmarker, note text, bookmark tags, list, fandom of work |
| Forum | as today |

**Omnibox flow:** parse fielded tokens and send bare text to all engines in parallel (prefix and trigram on `taxonomy_nodes.norm` and aliases, plus top works). Return grouped results with per-group scores, and offer disambiguation using fandom scope ("Spike · Buffy" vs "Spike · Cowboy Bebop"). Entity pages are saved queries: a fandom page is just the `fandom:X` view with its facets.

**Entity-to-entity discovery** ("fandoms like this", "ships similar to this", "tags that co-occur") comes from `taxonomy_cooccurrence` (PMI, not raw counts), shared-author overlap, tag-distribution cosine, and later embedding centroids.

### C. Semantic and natural-language search

**Embeddings (self-hostable, multilingual, since fanfic is):**
- Work level: summary, primary tags, and an opening chunk.
- Passage level: chunked body text, only for `index_policy = full`.
- Node level: tags, characters, and fandoms, used for alias suggestion and adjacency.
- User taste: opt-out.

**Storage:** pgvector (HNSW, halfvec) on Postgres. On SQLite, use `sqlite-vec` or an exact scan over a small candidate set, gated by `SearchCapabilities`.

**Hybrid execution** (the planner chooses by estimated selectivity of the SQL predicate):
- *Small candidate set (≤ N):* run the SQL filter first, then rank candidates exactly by vector similarity.
- *Large set:* use ANN with iterative scan, over-fetching and re-checking the full predicate, because short pages are the common failure of ANN plus filters. Never post-filter after `LIMIT`; this extends your existing invariant.
- Fuse lexical and semantic rankings with RRF.

**More-like-this with negatives:** `q = mean(pos) − λ·mean(neg)` for the vector part, plus tag-graph overlap (weighted Jaccard over closure-expanded tags) as a second signal. Both are visible in the explanation.

**NL → AST:** an LLM translates the user's sentence into *query-language text*, never SQL. The text goes through your parser and the typed validator, and any error is fed back once for self-correction. Unresolved phrases ("canon-compatible") resolve through aliases, then node embeddings. Low-confidence mappings appear as suggestion chips rather than silent filters. The model sees only the user's query string, never work text, which limits prompt injection and data exposure. This is an instance-level provider setting, off by default.

**Vibe sliders** are range filters over `work_vibe` (`vibe:angst:>0.7`). Fill the axes with a classifier or an embedding projection and store `model_version` so axes can be recomputed.

**Inferred tags** (tropes, POV, tense, pacing) are written to `work_tags` with `source=inferred` and a confidence. They are excluded by default and enabled with a visible toggle, and the UI labels them.

### D. Full-text

Keep your FTS5 and tsvector paths and add:
- **Snippets:** `snippet()` in FTS5 and `ts_headline` in Postgres, over a new `work_passages(work_id, chapter_id, seq, text)` table that doubles as the passage-embedding unit.
- **Scope:** `in:work:`, `in:series:`, `in:author:`, `in:collection:`.
- **Quote search:** phrase and proximity operators over `works_index_terms` positions, which you already store.
- **Opt-out:** `work_index_policy` is checked in the predicate for body, passage, and semantic terms, so a `metadata_only` work can never match them.

### E. Metadata, derived metrics, author signals

All of these become ordinary fields backed by `work_metrics_derived`, `metric_distributions`, and `author_signals`.

- **Normalization:** percentile lookup (`kudos_pct(fandom):>80`) is a single join against stored distributions.
- **Smoothing:** use Bayesian shrinkage on rates (kudos/hit, bookmark rate) so a work with 3 hits and 2 kudos doesn't top the charts.
- **Hidden gem:** high smoothed engagement rate combined with low exposure relative to the scope median.
- **Trending:** `velocity_7d` and `velocity_30d` from `stat_snapshots`.
- **Cadence:** `median_update_gap_days` and `next_update_expected`, so users can search "updates at least monthly".
- **Anti-gaming:** exclude `rating_anomaly_events` and flagged `quality_signals` from all rank features.
- **Missing counter:** engagement ratios need a hit or read counter. If you don't track one, build an aggregate, privacy-respecting counter, or define ratios against `reading_state` opens instead.
- **Chapter fields:** add chapter count, average chapter length, and a `format:` facet (podfic, fanart, translation, remix, crossover with primary-fandom flag, AI-use disclosure) backed by one searchable work-format table. Your media tables can stay separate.

### F. Personalized and social

- **`me:` namespace:** `me:read`, `me:kudosed`, `me:bookmarked`, `me:dnf`, `me:subscribed`, and `me:following_author`. These compile like `content_filters` (`NOT EXISTS` before `LIMIT`).
- **Presets:** hide-read, hide-DNF, and hide-kudosed are saved defaults per surface, stored in `reader_sort_preferences`-style rows. Muting has two modes, hard hide and downrank.
- **Social graph:** `bookmarked_by:(bookmarked:W)` compiles to a self-join over public bookmarks. The "readers who liked X also liked" item-item table is aggregated with a minimum-support threshold, and only public or opted-in signals feed it.
- **Bookmark and rec search:** index bookmark notes and tags as their own entity kind. This is high-value curated text, and it is also the best source of evaluation ground truth.

---

## 4. Ranking

Replace "one scorer" with explicit stages, reusing what you have:

1. **Candidates:** SQL predicate, lexical hits, ANN hits, and `meta_ranker` strategies for discovery slots.
2. **Score features:** lexical, semantic, tag-match weighted by prominence, boosts, quality prior, normalized popularity, recency, cold-start bonus, and taste fit (`taste_signal`).
3. **Fuse:** RRF across rankers, or a learned reranker once you have click data.
4. **Rerank:** real MMR using embedding or tag similarity, with `one_per_author` and `one_per_series` modes. `diversity_class` becomes a feature, not the algorithm.
5. **Explain:** extend `Candidate.reason` into a structured explanation, rendered to a string.

```rust
Explanation { matched_clauses, via_expansion: Vec<(term, via_node)>,
              inferred_tags_used, similar_basis: Vec<WorkId>,
              features: Vec<(name, value)> }
```

**Sorts:** relevance, newest, updated, kudos, fandom-percentile, trending, hidden gem, length, seeded random (stable across pages), "surprise me" (random from the top-K by quality with MMR), and personalized.

**Controls:** exclude the top N% by hits, a cold-start boost toggle, and personalization that is opt-out and resettable, with a visible taste summary.

**Pagination:** use keyset cursors plus an `index_epoch`, not offset, so hybrid results don't shuffle between pages.

---

## 5. UX contract (the API response)

One envelope serves every surface:

```json
{
  "chips": [ /* parsed AST as editable chips, with source: user|nl|preset|default */ ],
  "results": [ { "entity": "...", "snippet": "...", "explanation": {...} } ],
  "facets": { /* disjunctive, visibility-safe */ },
  "relaxations": [ { "drop": "status:complete", "gain": 43 } ],
  "diagnostics": [ /* span errors, ambiguous terms */ ],
  "cursor": "...", "epoch": 1234
}
```

- **Facets** are computed from the same compiled predicate as `WITH cand AS (...)`, then aggregated by node kind. They are **disjunctive**: each facet group's counts omit that group's own filter, so users see what including or excluding would do. Counts are capped or approximate above a threshold (shown as "~"), cached by `(query hash, viewer-filter hash, epoch)`, and subject to a minimum-cell-size rule for anonymous collections. A regression test asserts that facet counts never exceed result-set size.
- **Relaxation** is AST-based: drop each top-level conjunct, widen ranges, enable synonym expansion, and rank suggestions by gain versus loss. This is cheap because the AST makes each suggestion a transform.
- **Chips:** include/exclude toggles, grouping, and collapse all come from AST editing plus the printer. Chips carry provenance (user-typed, NL-derived, preset, inferred).
- **Saved searches** become feeds via `search_alerts` plus a `search_alert_seen(alert_id, work_id)` table for diffing, delivered by email, RSS, and in-app notification. **Recipes** are public `saved_views` with `forked_from`. Presets ("comfort reads", "binge WIPs") are views pinned to a surface.
- **Result cards** show prominence-ordered tags, warnings with depiction, reading time, cadence, snippet, and "why this."

---

## 6. Privacy and safety

You are strong here, so extend the pattern:

- **Facets and relaxations reuse the visibility predicate**, as above.
- **`work_index_policy`** (full / metadata_only / link_only) is enforced in the predicate for body, passage, semantic, similar-to, and recommendation candidates. Changing it triggers fast-lane removal from vector tables as a hygiene step, but correctness doesn't depend on it.
- **Anonymous collections:** authorship is stripped from author facets, the author field, `similar_to` explanations, and co-occurrence aggregates until reveal. Ranking features derived from authorship (author signals) are also suppressed.
- **Embeddings and inferred data** are derived only from content already visible to the viewer class, never from drafts, and are deleted with the work.
- **Taste vectors and co-occurrence** are opt-out, aggregated with minimum-support thresholds, and never exposed per-user.
- **Harassment resistance:** no cross-work search by commenter, rate limits on enumeration-style queries (reuse `abuse_counters`), and complexity budgets on the AST (max depth, terms, expansion size) so one query can't become a denial of service.
- **NL parsing:** the model sees only the query string, and its output is validated by the parser, so it cannot emit SQL or bypass viewer filters.
- **Federation:** index remote works from received activities only, never forward a user's query to remote instances, and carry the remote `index_policy` and visibility in the activity. Add an `origin` field and facet so users can scope to local or remote.
- **Query logs:** hash and minimize, with a short retention period and no user ID unless the user opts in.

---

## 7. Architecture

- **Engine:** stay on SQL. Your compiler and dual-dialect discipline are real assets, and nothing here needs Elasticsearch. Add pgvector (Postgres) and `sqlite-vec` (SQLite) behind a `VectorIndex` trait. If scale later demands it, the `Plan` boundary lets you swap in Vespa or OpenSearch for the lexical and vector stages without touching the AST or the privacy layer.
- **Planner:** `AST → Plan { predicate, lexical, semantic, boosts, sort, facet_requests }`. This refactor of `query_sql*.rs` is the central code change. Each engine implements `compile(plan)`.
- **Indexing lanes** (fed by domain events, following your `outbox_events` pattern, in a `search_index_jobs` table):
  - *Fast:* deletes, visibility, tag edits, `index_policy`.
  - *Slow:* embeddings, passages, inferred tags, vibe axes.
  - *Batch:* percentiles, co-occurrence, author signals, closure rebuild.
- **Closure maintenance:** on edge change, recompute the affected subtree transactionally, with cycle detection at write time.
- **Dialect parity:** a capability matrix in code, with contract tests that run every query fixture on both dialects and diff results.
- **Eval harness:** `search_query_log`, `search_clicks`, and `search_reads` (impression → click → read-through). Offline relevance sets come from bookmarks with notes, rec lists, and series-continuation reads. Track nDCG@k, MRR, zero-result rate, and latency percentiles. Replay a corpus of ASTs against candidate rankers behind flags. Extend `forum_search_misses` into a general misses table.

---

## 8. Build order

| Phase | Scope | Unlocks |
|---|---|---|
| **1. Schema** | Taxonomy scope, edges, closure; `work_relationships` / `ship_participants` / `work_characters`; tag prominence, source, confidence; `work_warnings`; `work_index_policy`; backfill and curator tools | Journey 12, disambiguation, prominence-aware exclusion |
| **2. Language and planner** | `Plan` refactor, ranges, scoped predicates, `Expand`, `MinMatch`, boosts, `Me`, `Ref`, pretty-printer, diagnostics, complexity budgets | Everything below compiles onto this |
| **3. Response and discovery UX** | Facets (disjunctive, cached), relaxation, snippets and passages, hide-read / DNF exclusion, alert feeds, omnibox and `SearchEngine` trait with taxonomy, collection, series, and bookmark engines | MVP-complete search |
| **4. Stats** | Derived metrics, distributions, author signals, co-occurrence, hit/read counter, new sorts | Fandom-normalized sorting, hidden gems, trending |
| **5. Semantic** | Embeddings, `VectorIndex`, hybrid planner, `sem:`, `similar_to` with negatives, vibe axes, inferred tags, NL → AST | Differentiators |
| **6. Social and learning** | Item-item co-reads, `bookmarked_by`, bookmark and rec search, MMR, learned reranking, personalization UI, federated indexing | Long-term quality |

Phases 4 and 5 can run in parallel once Phase 2 lands, since both only add retrievers and features behind the planner.

---

## 9. Decisions and risks

- **Character backfill is the hardest migration.** Existing character nodes are ambiguous across fandoms. You need a curator or semi-automatic split tool, using the fandoms of the works that use each node, and a period where nodes are `pending`.
- **Implication and parent edges need governance.** Make them curator-only, default them off in queries, and show every expansion as a chip ("via Fake Dating → children").
- **Expansion blow-up:** cap closure depth and result counts, and charge expansions against the complexity budget.
- **ANN with filters** is the main source of subtle bugs (short pages and recall loss under selective filters). Test with adversarial selectivity.
- **Embedding hosting and cost:** choose an open multilingual model you can self-host, version embeddings by `model_version`, and plan for re-embedding.
- **Popularity feedback loops:** normalized and smoothed metrics help, but also keep the cold-start boost and an anti-popularity toggle, and measure exposure inequality in the eval harness.
- **Hit counting** has privacy implications. Aggregate it with no per-user trail, or use opt-out reading state.
- **Dialect parity** will tax every feature. The capability matrix and cross-dialect fixtures are what keep that cost manageable, and some features (ANN) may be Postgres-first by design.

If you tell me which phase you're starting, I can draft the Phase 1 migrations (both dialects), the `Plan` and AST type definitions, or the facet CTE and relaxation algorithm in Rust.
