# Advanced Search for a Fanfiction Archive — external design reference

> **Status: pasted reference, not yet reconciled with `docs/spec.md`.** Saved verbatim as
> received on 2026-10-01. See "Reconciliation notes" at the bottom for the conflicts this
> document has with existing spec clauses, and the open questions it raises. Nothing in
> this file is normative until those are resolved.

---

## 1. Design principles

1. **Everything is an entity in one graph.** Works, chapters, series, users and pseuds, fandoms, characters, ships, tags, collections, challenges, prompts, bookmarks, and bookmark lists are all searchable, linkable, and filterable by each other.
2. **Structured, fuzzy, and semantic search all work together.** Users should be able to type a precise query, describe a vibe in prose, or point at a story and say "more like this."
3. **Exclusion is as important as inclusion.** Fanfic readers often search by what they don't want.
4. **Popularity is normalized.** A 2k-kudos work in a small fandom is a bigger deal than the same number in a huge one.
5. **Everything is explainable and editable.** Natural-language and personalized queries compile down to visible, editable filters.

---

## 2. Tag system design (this determines search quality)

An open, author-driven tag system works well for search if you add a **canonical layer** on top of the free text.

**Recommended: folksonomy in, graph out**
- Authors type anything. The system maps input to canonical tag nodes (autocomplete, fuzzy matching, embeddings), and unmapped tags stay as "raw" tags until resolved.
- Each tag node has a **type**: fandom, character, relationship, trope, genre, mood, content warning, setting/AU, format (e.g. text messages, epistolary), POV, and so on. Types can be inferred, then confirmed by users or curators.
- The graph holds **synonyms** (merge), **parent/child** (e.g. "Fake Dating" under "Romance Tropes"), **implications** (optional, e.g. "Hogwarts Eighth Year" → "Post-War"), **related** links, and **fandom scoping** (the character "Spike" in *Buffy* is not the one in *Cowboy Bebop*).
- **Structured relationship tags:** a relationship is an object with participants, a type (romantic, platonic, familial, QPP), and optional roles, not a string like "A/B". This lets you query "any romantic pairing involving X" or "X with anyone except Y."
- **Tag prominence:** authors mark tags as *primary*, *secondary*, or *background/mentioned*. "Exclude Major Character Death" and "include Angst" mean very different things when a tag is incidental.
- **Three tag sources**, each with a confidence level and distinct UI treatment:
  1. Author-applied
  2. Reader-applied (voted or suggested, with author approval or a separate "reader tags" layer)
  3. Machine-inferred (tropes, tone, POV, tense, pacing), labeled as inferred and filterable
- **Content warnings** get their own system: severity, whether explicitly on-page or just referenced, chapter-level location, and a spoiler-safe display option.

This lets search treat tags as IDs rather than strings, so synonyms and wrangling don't produce missed results.

---

## 3. Query capabilities

### A. Unified omnibox
- Typed tokens/chips with autocomplete: `@author`, `#tag`, `fandom:`, `ship:`, `collection:`
- Results grouped by entity type ("Fandom: …, Ship: …, Author: …, Works: …"), with scope tabs to narrow
- Query understanding: detects whether "Wen Kexing" is a character, an author, or a tag, and offers disambiguation

### B. Structured query language (power users)
Boolean logic with nesting, plus field operators:

```
fandom:"Star Wars" 
AND ship:(Obi-Wan/Anakin OR "Obi-Wan & Anakin")
AND trope:"slow burn"
AND NOT warning:"Major Character Death"
AND words:20k..100k
AND status:complete
AND updated:<2y
AND kudos_pct:>top20%
```

- Phrase, proximity, wildcard, and fuzzy matching
- Regex for titles and usernames
- Per-term weights and boosts: `trope:"found family"^2`
- Nested groups and "at least N of these" (`min_match:2 of (…)`)
- Tag expansion toggles: *exact*, *include synonyms*, *include children*, *include implied*
- A visual query builder that round-trips with the text syntax

### C. Natural-language and semantic search
- "Slow burn enemies to lovers, canon-compatible, no cheating, finished, over 50k" is parsed by an LLM into structured filters shown as editable chips, with the remainder turned into a semantic query.
- Embedding search over summaries, tags, and body-text chunks, so "a story where they're stuck in a lighthouse during a storm" finds matching scenes even without those words in the metadata.
- **More-like-this** from one or many works, with **negative examples** ("like A and B, not like C").
- **Vibe sliders:** angst ↔ fluff, plot ↔ character study, slow ↔ fast burn, dialogue-heavy ↔ descriptive.

### D. Full-text search
- Within works, at chapter or passage level, returning snippets with highlighting
- Search a single work, series, author's works, or collection
- Quote search ("find that fic with this line")
- Opt-in per work, so authors who don't want body-text indexing can restrict to metadata

### E. Metadata and numeric filters
- Words, chapters, average chapter length, rating, language, completion status, publish and update dates, update cadence
- Engagement: hits, kudos, comments, bookmarks, subscriptions
- **Derived metrics:** kudos/hit, comments/kudos, bookmark rate, fandom-percentile rank, trending velocity, "hidden gem" score (high engagement-to-exposure ratio)
- **Author signals:** completion rate, abandonment risk, posting frequency
- Format: podfic, fanart, translation, inspired-by, remix, crossover (A AND B, or A as primary), AI-use disclosure
- Writing style (inferred): POV, tense, reading level, dialogue ratio

### F. Personalized and social filters
- **Hide:** read, kudosed, bookmarked, DNF'd, authors I've blocked, tags I've muted. Muting has two modes: *hard hide* or *downrank*.
- **Include only:** works from authors I follow, in my subscriptions, or in my to-read list
- **Social graph queries:** "bookmarked by people who bookmarked X," "recced by users with similar taste," "in collections that include Y"
- Search other users' **public bookmarks and rec lists**, including their notes and tags, since bookmark notes are valuable curated text.

### G. Non-work entity search
| Entity | Useful filters |
|---|---|
| **Users** | fandoms written, ships, languages, post frequency, open to prompts or commissions, beta-ing, gift preferences |
| **Fandoms** | size, activity trend, related and adjacent fandoms, media type, "fandoms like this" |
| **Characters/Ships** | by fandom, popularity trend, common tropes paired with them, "ships similar to this" |
| **Collections/Challenges** | open or closed, deadlines, themes, fandom, rating limits, anonymity state, prompts remaining |
| **Series** | complete, length, update status |
| **Tags** | usage count, trend, type, parent/child, "tags that co-occur with…" |
| **Prompts/Requests** | unclaimed, by fandom, by ship |

Tag and fandom search double as discovery tools, with co-occurrence ("people who use this tag also use…") and trending views.

---

## 4. Ranking

- **Hybrid retrieval:** BM25 for lexical matching, vector similarity for semantic matching, and metadata filters, merged by reciprocal rank fusion or a learned reranker.
- **Sort options:** relevance, newest, recently updated, most kudos, **fandom-normalized quality**, trending, hidden gems, longest or shortest, random, "surprise me within these filters," and **personalized**.
- **Diversity reranking** (MMR) to avoid ten near-identical results, with an optional "one per author" mode.
- **Cold-start boost** so new works with few hits can surface.
- **Anti-popularity-bias controls:** an "exclude top N% by hits" toggle.
- **"Why this result?"** shows which filters matched, which tags were inferred, and what the similarity basis was.
- **Personalization is opt-in, inspectable, and resettable.**

---

## 5. UX features

- **Live facet counts** that update as filters apply ("Angst (342), Fluff (1,204)…"), including "zero results because of this filter" hints.
- **Click once to include, again to exclude**, with chips that group and collapse.
- **Saved searches** become feeds: email, RSS, or in-app alerts when new matches appear.
- **Search presets/profiles:** "Comfort reads," "Long WIPs I can binge," "Safe for work," switchable in one click.
- **Shareable search URLs** and public "search recipes" that others can fork.
- **Result cards** show summary, primary tags, prominence-ordered warnings, reading time, update cadence, a matching snippet, and one-line reader pitches.
- **Result grouping** by fandom, author, or series.
- **Query relaxation:** "No results. Relax: remove 'complete' (+43), widen word count (+18)."
- **Reading-state integration:** continue where you left off, "next in series," "similar to what you just finished."

---

## 6. Privacy, permissions, and safety

These are easy to get wrong in search.
- **Visibility** (drafts, restricted/registered-only, locked, unlisted, orphaned) must be enforced at query time and in facet counts, so counts don't leak hidden works.
- **Anonymous collections and exchanges:** authorship stays hidden in search, facets, and "more like this" until reveal.
- **Opt-outs:** authors can exclude works from semantic and body-text indexing, recommendations, or all search (link-only).
- **Blocklists and muted tags** are private and never exposed through shared searches.
- **Harassment resistance:** no searching for comments by a specific user across works, and rate limits on enumeration-style queries.
- **Inferred tags and embeddings** shouldn't expose private data.

---

## 7. Architecture sketch

- **Engine:** Vespa or OpenSearch/Elasticsearch for hybrid lexical plus vector ranking at scale. Postgres plus pgvector, or Meilisearch/Typesense, is fine for an MVP.
- **Indexes:** one per entity type, with denormalized documents, plus a separate **tag-graph service** for synonym and implication expansion (at index time for stable relations, at query time for user-toggled expansion).
- **Embeddings:** work-level (summary and tags), passage-level (chunked text), and user-level (taste vectors, opt-in).
- **Pipeline:** event-driven indexing (publish, edit, tag change, stats update) with separate slow lanes for ML-inferred tags and embeddings, and a fast lane for metadata and visibility changes.
- **User-specific filters:** Bloom filters or bitsets of read/blocked IDs applied at query time.
- **Stats:** engagement counters updated in near-real-time; normalized percentiles recomputed periodically per fandom and ship.
- **Eval:** logged queries, zero-result rates, and click and read-through metrics, with an offline relevance test set built from bookmarks and recs as ground truth.

---

## 8. Tiered rollout

1. **MVP:** unified entity search, structured filters, exclusion, tag canonicalization, facet counts, saved searches.
2. **Next:** the query language, fandom-normalized stats, full-text and snippets, hide-read, and relationship objects.
3. **Differentiators:** semantic and natural-language search, more-like-this with negative examples, inferred tags, vibe sliders, and social graph queries.
4. **Long term:** personalized ranking, bookmark and rec-list search, reader-tag layers, and chapter-level content warnings.

---

If you tell me your expected scale, whether you want reader-applied tags, and your stance on ML-inferred metadata, I can propose a concrete tag schema and index mapping.

---

## Reconciliation notes (added 2026-10-01, not part of the pasted text)

### Where lorehaven's existing spec already covers this — do not "add" it

| Pasted proposal | Existing spec clause | Verdict |
|---|---|---|
| Structured query language, field operators, nesting | §15.3 typed query AST, §15.4 user-facing query language (`title: author: fandom: character: relationship: tag: mood: summary: body: language: status:`), §15.5 fuzzy matching | **Already specified.** §15.3 even defines `ExistsCharacter`/`ExistsRelationship` nodes. The pasted language is a superset, not a replacement. |
| Full-text body search, snippets | §15.9 full-text body search | Already specified. |
| Popularity normalized per fandom | §47 ranking substrate, §29.2 Elo pairwise ranking | Already specified, and spec's version is *better founded* — see conflicts below. |
| More-like-this / recommendations | §16.6 community similarity, §16.7 recipe builder, §43 recommendation-first browsing ("one ordering contract for every surface") | Already specified with a stronger contract. |
| Natural-language search | §23.8 natural-language search assist | Already specified. |
| Saved searches | §14.2 saved searches and named views | Already specified. |
| Recommendation transparency ("Why this result?") | §33.3 recommendation transparency and curation labour | Already specified, with more rigour. |
| Fandom-normalized quality sort | §47.2 the ranking contract | Already specified. |
| Tag canonical layer, synonyms, parent/child | §4.5 taxonomy tables, §15.8 mood and tone taxonomy, M45 directory category governance | Partly specified. **Synonym/implication graph edges are the genuine gap.** |
| Facet counts | §15.15 length histogram in search; facet counts partially | **Gap** — need to check whether live facet counts exist. |

### Genuine gaps worth adopting

1. **Tag implication edges** (`Hogwarts Eighth Year` → `Post-War`) as a first-class graph relation, with user-toggleable expansion at query time. Spec has parent/child taxonomy and synonyms, but not a general implication edge. This is the single most valuable idea in the pasted document.
2. **Tag prominence** (primary/secondary/background). This directly serves the use case in §15.3 journey 12 — "exclude Major Character Death" means something different when the tag is incidental. Currently `work_tags` has no prominence column (needs verification).
4. **"Why this result?"** as a general mechanism (not just recommendations).
5. **Vibe sliders** — but note §36.11 Mood Journal already covers mood-based discovery; sliders are a UI refinement, not new capability.
6. **Anti-popularity-bias toggle** ("exclude top N% by hits").

### Where the pasted document conflicts with the spec — do NOT adopt silently

1. **Reader-applied tags vs §49.2.** The pasted doc proposes reader-applied tags as one of three tag sources. Spec §49.2 is emphatic: *"Only confirmed tags count toward gravity"* and confirmed means author-confirmed. A reader-tag layer would need to be explicitly excluded from gravity/confidence, or it becomes a griefing vector. Adopt with a hard constraint, or not at all.
2. **ML-inferred tags vs §47.10.** The pasted doc proposes machine-inferred tropes/tone/POV/tense as filterable. Spec §47.10 *refuses* learning-to-rank, embeddings, and ML in the ranking path (§49.9 reiterates: "No learning-to-rank, no embeddings, no ML"). This is a direct architectural contradiction, not a gap. Needs an explicit decision, not a silent merge.
3. **Embedding search / semantic vectors** — same conflict as above.
4. **Vespa/OpenSearch/Elasticsearch** vs the spec's Postgres + SQLite two-dialect architecture (`scripts/postgres-journey.sh`, dual-migration `migrations/sqlite/` + `migrations/postgres/`). Adding a third search engine is a large architectural commitment the spec has so far avoided.

### The one place the spec is genuinely incomplete on the §15.3 use case

The `ExistsCharacter`/`ExistsRelationship` AST nodes in §15.3 have **no implementation and no schema**: there is no migration defining character or relationship tables, and `QueryAst` in `crates/domain/src/query.rs` has only `Text | Phrase | Fielded | Comparison | And | Or | Not`. Characters and ships exist only as `taxonomy_nodes` rows with `kind = 'character'` / `'ship'`, which supports a flat name match and nothing else — no prominence, no attributes, no participants.

So "character A without a relationship" (spec §25.1 journey 12) is **not implementable today** without: new tables, a `work_characters` join with prominence, a `work_relationships` table with participants and kinds, and two new AST variants. That is the actual work, and the pasted document does not change it — it only adds more search features on top of a substrate that does not exist yet.

### Open questions the pasted document asks back

- Expected scale (determines whether SQLite FTS5 is acceptable or a real engine is needed).
- Reader-applied tags: yes/no (conflicts with §49.2).
- Stance on ML-inferred metadata (conflicts with §47.10 / §49.9).

These three answers decide whether this document is a 5% addition or a rewrite of the search architecture.