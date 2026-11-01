# Ranking: every "similar fic" signal × every surface

**Status:** design, 2026-11-01. Written down at the operator's request, together with the three
mechanisms this instance wants on top of it: **a field of algorithms per surface, live A/B with
automatic promotion, and taste gravity pulling every surface toward the operator's taste.**

**Where the code is today** (verified in the tree, not assumed):

| piece | state |
|---|---|
| `crates/domain/src/similar.rs` | **live** — idf-weighted cosine over title + summary, shipped as M63-04 (`dh rec --seed`, `?seed=`). Neighbours, `--explain` tokens, and the gate `ranked N of M, scanned S`. |
| `crates/app/src/rec_engine.rs` | **live** — a strategy registry (`catalogue`, `legacy`, `rrf` + registered names) behind `/api/v1/rec/run`; `--engine` and `--explain` already report each strategy's contribution. |
| `recipes` table (migration 0012) + M11-05 | **live** — shareable recommendation formulas: a recipe names strategies and weights. |
| `crates/domain/src/browse.rs` (M43) | **live** — one `Sort` vocabulary across browse surfaces; `for-you` is a **permutation, never a filter**; per-surface diversity budget. |
| `crates/taste` | **mostly a declaration, on purpose.** Implemented: `arena::glicko` and `eval` (`ndcg_at`, `recall_at`, `overlap_at`). Empty and explicit: `collab` (§12.7), `graph` (§12.2), `rank` (§12.7/§19), `diversify` (§12.7), `embed` (§12.2), `profile` (§12.2), `signal` (§12.7), `budget` (§8.2/§22.3). |
| spec §16.1–§16.5, §16.16, §43 | **the contract these surfaces must obey** — engine contract, influence layer, diversity budget, taste gravity, ordering. |
| `library_items` data | **the constraint everything below runs into**: 1,247 of 141,317 rows carry any endorsement; `views` null on all of them; `word_count` null on all engaged board rows; `work_tags` joins **0** of those 1,247 rows; `reading_progress` holds **1 row**. |

That last row is why the build order at the end is not the order of this document's tiers: a
signal that needs data the catalogue does not have is a signal that ships as a refusal.

---

# Part I — The signals, ranked by power

## Tier S — strongest similarity signals

| Rank | Signal | Why it's strong | Why it's hard | Data you need |
|------|--------|----------------|---------------|---------------|
| **1** | **Co-shelving** (fics that appear on the same reading lists / bookshelves) | This is collaborative filtering at its purest. A reader who curates a list is making an editorial judgment: "these belong together." Thousands of those judgments converge on real similarity that no tag system can capture. | You need reading lists to be a first-class feature with real adoption. Cold-start problem for new fics. | User bookshelves/lists, ≥ several hundred active curators |
| **2** | **Fandom + pairing + trope tag intersection** (Jaccard or weighted cosine on tag vectors) | Tags are the *native language* of fanfic discovery. A reader looking for "enemies-to-lovers Drarry 100k+ complete" is expressing a high-dimensional query that tags answer perfectly. Explainable, auditable, no black box. | Tag quality varies wildly. Author-applied tags are inconsistent. You need a controlled vocabulary or synonym mapping, or results degrade fast. | Normalised tag taxonomy with fandom, character, relationship, trope layers |
| **3** | **Engagement-pattern similarity** (fics whose kudos/bookmark/subscriber overlap resembles this one) | Captures taste clusters that cross fandom lines. "People who bookmarked this Worm fic also bookmarked these RWBY fics" finds connections no metadata can. | Needs dense engagement data. Sparse in small archives. Privacy-sensitive if you expose user-level signals. | Per-user engagement events (kudos, bookmarks, follows) at scale |
| **4** | **Same author's other works** | Trivially computable, surprisingly high click-through. Readers bond with voice, not just premise. An author's backlist is the most natural "similar" list there is. | Only works if the author has multiple fics. Doesn't help discover *new* authors. | Author ID (you have this) |

## Tier A — strong, feasible signals

| Rank | Signal | Why it's good | Limitations |
|------|--------|--------------|-------------|
| **5** | **Summary/description embedding similarity** (TF-IDF, SBERT, or LLM embeddings on summaries) | Captures premise similarity that tags miss. "Both stories are about a protagonist waking up in a fantasy world with a broken system" — the vibe match. | Summaries are short and inconsistent. Some are one line of shitpost. Embedding quality degrades on fanfic-specific jargon without fine-tuning. |
| **6** | **"Readers also read" — session-based co-reads** | What did people click *next* after reading this fic? Pure behavioural signal, no explicit action needed. | Requires session tracking. Noisy — people click for many reasons. Needs volume to converge. |
| **7** | **Fandom graph proximity** (same universe → same fandom → adjacent fandom → same medium) | Structural. A Naruto fic is closer to a Boruto fic than to a Worm fic. A Worm fic is closer to other web serials than to anime fics. You can build a fandom ontology once and it works forever. | Static — doesn't capture taste. "I like Worm for the power gaming, not because it's a web serial." Fandom adjacency ≠ story similarity. |
| **8** | **Crosspost identity** (same fic on multiple archives) | You already have this in your catalogue. If a fic exists on SB and QQ and AO3, you can use the *union* of engagement signals from all copies. This isn't "similar fics" per se, but it's the foundation for every cross-source metric. | Only helps fics that are crossposted. |

## Tier B — useful secondary signals

| Rank | Signal | Use case |
|------|--------|----------|
| **9** | **Word count bracket similarity** | Readers have strong length preferences. A 2k one-shot and a 500k epic are not substitutes even if tags match perfectly. Filter, not rank. |
| **10** | **Completion status matching** | Many readers filter "complete only." Recommending an abandoned fic to someone who just finished a complete one is a bad experience. Filter, not rank. |
| **11** | **Rating/content-warning match** | Safety and preference. Don't recommend an Explicit fic to someone reading Gen unless they've opted in. Hard filter. |
| **12** | **Update cadence similarity** | "I like fics that update weekly" is a real preference. Pairs with your `updater_reliability` metric. Secondary sort. |
| **13** | **Author-network proximity** (authors who beta for each other, co-write, or are in the same writing circles) | Social signal. Hard to extract automatically, but communities like SB/SV have visible social graphs (forum interactions, quote chains). |
| **14** | **Full-text embedding similarity** | The nuclear option. Embed the actual story content. Captures writing style, pacing, tone. Computationally expensive, needs the full text, but it's the closest thing to "this *feels* like that." |

## Tier C — weak but free

| Rank | Signal | Notes |
|------|--------|-------|
| **15** | **Publication date proximity** | Recency bias can be a feature ("what's current in this fandom?") but is a terrible similarity signal on its own. |
| **16** | **Title/naming pattern similarity** | Occasionally useful (SI fics, CYOA fics have naming conventions), mostly noise. |
| **17** | **Source (archive) match** | "Other fics on SB" isn't similarity, it's just co-location. But it's a weak cultural signal — SB skews toward a specific tone. |

---

# Part II — Where each signal goes: every surface

Surface names below are the canonical identifiers the registry uses; they are what
`--engine`, the experiment log and the gravity dial are keyed by.

### 📖 Story page (`story.*`) — the most important surface

| Placement | Surface id | What to show | Signals | Why here |
|-----------|-----------|-------------|---------|----------|
| Sidebar / below summary | `story.similar` | "Similar stories" (5–10) | #2 tag intersection + #1 co-shelving + #3 engagement overlap, blended | The primary discovery moment: the reader is deciding "do I want to read this?" |
| Below the fic / end of chapter | `story.also_enjoyed` | "Readers also enjoyed" (5) | #6 co-reads + #1 co-shelving | The reader just *finished*. Highest-intent surface on the site. |
| Author byline | `story.more_by_author` | "More by this author" (3) | #4 same author | Trivial, high CTR, always show when the author has other works. |
| Tag chips | `story.tag_chips` → filtered browse | Each tag links to a filtered browse | #7 fandom graph + #2 tags | Let the reader pull on any thread. |
| "Part of N reading lists" | `story.lists_containing` | The public lists containing this fic | #1 co-shelving | Curated bundles beat any algorithm. |

### 🔍 Search (`search.*`)

| Placement | Surface id | What to show | Signals |
|-----------|-----------|-------------|---------|
| Result grouping | `search.results` | Cluster by fandom or trope, not a flat list | #2 tags + #7 fandom graph |
| "Did you mean…" / related searches | `search.suggest` | Adjacent tags or fandoms when a search is thin | #7 fandom graph + tag synonyms |
| Inline quality signal | `search.results` | `source_pct` badge or engagement tier on each result | M63-03 metrics |
| Filters as first-class UI | `search.facets` | Length, status, rating, recency as toggles | #9–#12 |

### 👤 Author profile (`author.*`)

| Placement | Surface id | What to show | Signals |
|-----------|-----------|-------------|---------|
| Works list | `author.works` | All fics, quality/recency toggle | #4 same author + the quality composite |
| "Authors like this" | `author.similar` | 3–5 similar authors | #3 engagement overlap at author level |
| "Appears on lists with" | `author.colists` | Authors whose fics co-occur on lists | #1 co-shelving, aggregated |
| Fandom spread | `author.fandoms` | Which fandoms this author writes in | #7 fandom graph |

### 🏠 Homepage (`home.*`)

| Placement | Surface id | What to show | Signals |
|-----------|-----------|-------------|---------|
| "Trending" | `home.trending` | Highest velocity this week | velocity (needs the snapshot series) |
| "Hidden gems" | `home.hidden_gems` | High velocity, low absolute count | hidden-gem metric (needs velocity) |
| "Popular in [fandom]" | `home.popular_in_fandom` | Top fics per fandom the reader has shown interest in | #2 + #3, personalised when logged in |
| "Recently updated" | `home.recently_updated` | Active long-runners | #12 cadence |
| "New this week" | `home.new_this_week` | Just-published, sorted by early engagement | #15 + velocity |
| "Curated lists" | `home.curated` | Editorially or community-curated collections | #1 |

### 📚 Library (`library.*`)

| Placement | Surface id | What to show | Signals |
|-----------|-----------|-------------|---------|
| "Because you shelved X" | `library.because_you_shelved` | 3 recs per shelved fic | #1 + #2 |
| "Fill the gap" | `library.fill_the_gap` | Works that would complete a taste cluster the reader has half-read | #3 — what 80 % of similar shelves hold and this reader does not |
| "Your authors updated" | `library.authors_updated` | New chapters from followed authors | #4 + #12 |

### 🏷️ Tag / fandom browse (`tag.*`)

| Placement | Surface id | What to show | Signals |
|-----------|-----------|-------------|---------|
| Default sort | `tag.default_sort` | Quality composite, not recency | the `quality` metric |
| "Adjacent tags" | `tag.adjacent` | Tags that frequently co-occur | #2 co-occurrence matrix |
| "Cross-fandom picks" | `tag.cross_fandom` | "If you like [trope] in A, try it in B" | #2 + #7 |
| Faceted filtering | `tag.facets` | Length / status / rating / source | #9–#12 |

---

# Part III — The three mechanisms this instance wants

The signal list above answers "what could we rank by". This part answers "how does the site get
better at it without anyone shipping a winner by hand", and it is the part that turns 17 signals
into a system.

## III.1 A field of algorithms per surface, not one ranker

Every surface in Part II names a **field** of two to four algorithms, not one. A surface serves
one of them at a time; which one is data, not code:

```toml
[surface.story.similar]
algorithms = ["text_cosine", "tag_jaccard", "co_shelving", "rrf_blend"]
incumbent  = "text_cosine"     # what anonymous traffic gets until an experiment says otherwise
explore    = 0.10             # share of traffic the challenger field receives
```

Rules that make this safe to run on a live instance:

- **A surface with one algorithm is legal** (and is where every surface starts); the field grows
  as algorithms land.
- **The registry is closed and named.** An algorithm is a registered id with a declared data
  dependency (`needs = ["work_tags"]`); an algorithm whose dependency is empty in this catalogue
  refuses by name, exactly as `/catalogue?sort=velocity` refuses today with the milestone that
  unblocks it.
- **A recipe (§16.3, M11-05) is a composition of algorithms, not a fourth thing.** A surface's
  field can therefore contain `rrf_blend` over the others, which is what the existing
  `rec_engine` registry already does for `/rec/run`.
- **The ordering contract (§43) and M43-02 hold here too: an algorithm may reorder, never
  filter.** A surface that drops rows has changed what exists, which is a different product.

**Field per surface, opening state:**

| surface | field |
|---|---|
| `story.similar` | `text_cosine` (live, M63-04), `tag_jaccard`, `co_shelving`, `rrf_blend` |
| `story.also_enjoyed` | `co_read`, `co_shelving`, `engagement_overlap` |
| `story.more_by_author` | `same_author` (by definition; the field is its *ordering*) |
| `story.lists_containing` | `co_shelving` (by definition) |
| `search.results` | `bm25_text`, `tag_jaccard`, `quality_pct`, `rrf_blend` |
| `author.works` | `quality_pct`, `recency`, `velocity` |
| `author.similar` | `engagement_overlap`, `co_shelving`, `fandom_graph` |
| `home.trending` | `velocity`, `velocity_decayed`, `engagement_overlap` |
| `home.hidden_gems` | `hidden_gem`, `velocity` |
| `home.popular_in_fandom` | `tag_jaccard`, `quality_pct`, `fandom_graph` |
| `home.new_this_week` | `recency`, `early_velocity` |
| `library.because_you_shelved` | `co_shelving`, `text_cosine`, `tag_jaccard` |
| `library.fill_the_gap` | `engagement_overlap` |
| `tag.default_sort` | `quality_pct`, `source_pct`, `recency` |
| `tag.adjacent` | `tag_cooccurrence` |
| `tag.cross_fandom` | `tag_jaccard` + `fandom_graph` |

## III.2 Live A/B with automatic promotion

The experiment is a first-class object, and promotion is a decision the instance makes from
evidence it logged, not a release a person remembers to cut.

**Assignment.** `bucket = hash(surface, experiment_id, subject)` where `subject` is the pseud for
a signed-in reader and the session id otherwise; sticky for the whole experiment for a subject.
Anonymous traffic is included — the catalogue's readers mostly are — but its share is capped so
one bot cannot decide a winner.

**What is logged.** One impression row per (surface, algorithm, position, item) served, plus the
event that follows it: `open`, `read_threshold` (e.g. 50 % of the first chapter), `bookmark`,
`kudos`, `negative` (the reader hid it / "not interested"). Rows are append-only; an aggregate
snapshot per (experiment, arm, day) is what promotion reads, so promotion never depends on a
query over the raw log at decision time.

**Metrics, per arm.**

| kind | metric |
|---|---|
| primary | CTR@k, and conversion (`bookmark` \| `kudos` \| `read_threshold`) per 1,000 impressions |
| guardrail | negative-feedback rate, diversity of the served set (distinct fandom/author share), catalogue coverage, p95 latency |
| anti-gaming | impression share per source (one archive must not take the field), new-work share |

**Promotion.** A challenger replaces the incumbent when **all** of:

1. it leads on the primary metric with a sequential test that is valid under continuous
   monitoring (a confidence sequence, not a fixed-horizon p-value that was peeked at);
2. every guardrail is within its bound — a challenger that wins CTR by serving one archive is
   not a winner, it is a regression with better numbers;
3. the minimum sample is met (≥ 1,000 impressions and ≥ 50 conversions per arm, both
   configurable) and the experiment ran ≥ 7 days, so a weekday effect is not a verdict;
4. the promotion is written down: experiment id, arms, metric values, guardrail values, the
   algorithms swapped, and the previous incumbent, so a rollback is one row, not an archaeology
   exercise.

**Rollback is automatic too**: if a promoted arm breaches a guardrail for two consecutive daily
snapshots it is demoted to the previous incumbent and the experiment is marked `rolled_back`.
Both directions are logged with the numbers that caused them.

**Anti-collapse.** A surface never runs a field of one forever: at least `explore` (default 10 %)
of traffic goes to non-incumbent arms while a challenger is live, and an exhausted field (no
challenger in the last 30 days) triggers a *proposal* — "tag_jaccard has not been tried on
`story.similar`" — not a silent promotion.

## III.3 Taste gravity on every surface

The operator's taste is an influence layer (§16.3), not a scoring term, and it applies **after**
the blend, on every surface in Part II — the same place §16.16 already puts taste gravity for the
feed, extended to recommendation surfaces:

```text
served_score(item) = (1 - g_surface) * algorithm_score(item)
                   +  g_surface      * taste_match(item, operator_profile)
```

with `taste_match` computed from the operator's profile (§16.2) over the entities the catalogue
already has — fandom, tag, author, source — and `g_surface` a **per-surface** strength, because
"more by this author" must stay that author and "hidden gems" must stay hidden:

```toml
[gravity]
enabled = true
default = 0.15                    # every surface starts here
[gravity.surface]
"story.more_by_author" = 0.0      # the byline is a promise, not a recommendation
"story.similar"        = 0.10     # similarity is the point; gravity only breaks ties
"home.trending"        = 0.35     # an editorial surface may lead with taste
"home.hidden_gems"     = 0.25
"tag.cross_fandom"     = 0.20
```

Constraints that are not negotiable, because they are already promises this instance makes:

- **Gravity reorders, it does not filter.** M43-02's rule (for-you ordering is a permutation,
  never a filter) applies unchanged. A work the reader could have found before the dial is still
  findable after it.
- **Bounded, floor and ceiling are configuration** (§16.16's shape), and the dial's zero is the
  reader's strongest control (§16.5) — unless the operator has locked the dial, in which case the
  landing page says so.
- **Never narrated per item.** §0.3: the influence is not printed as a per-item reason and no
  score this specific is exposed. What *is* published is the coarse, auditable summary — mode,
  and a histogram of pull distribution — so drift is visible without enabling per-work gaming.
- **Not a lower bound on the algorithm's own job.** Gravity is measured as a separate arm in
  III.2's experiments, so "the operator's taste helped" is a number, not a belief.

## III.4 What this changes in the specification

Not rewritten here — recorded so M64-01 moves the spec before it moves the code, which is this
repo's rule:

| section | change |
|---|---|
| §16.1 (engine contract) | the contract currently describes the *feed* engine; it gains the surface id as an input and states that a surface serves a field of algorithms, one at a time |
| §16.3 (influence layer) | gravity is already an influence layer here; III.3 makes it per-surface and states that it applies after the blend on every surface, not only the feed |
| §16.16 (taste gravity) | gains the per-surface strength table and the "reorders, never filters" constraint, which today is only stated for `for-you` sorting (M43-02) |
| §16.17 (diversity budget) | the guardrail metrics in III.2 belong beside the existing budget, since "diversity" is already a configured quantity |
| §43 (ordering contract) | gains the experiment arm as a factor, so a served order can be reproduced by (surface, algorithm, recipe, bucket) |
| new §16.18 | the experiment object: assignment, the append-only log, the four promotion conditions, and automatic rollback |

---

# Part IV — Build order, and why it is not the tier order

The catalogue's data decides the order. Each row below is a milestone row in
`docs/requirements.csv` (milestone **M64**, created for this design at the operator's request).

| # | Row | Ships | Depends on |
|---|---|---|---|
| 1 | M64-01 | Surface registry: canonical surface ids, per-surface algorithm field, config, and the refusal for an algorithm whose data is empty | — |
| 2 | M64-02 | Impression + event logging (append-only) and sticky assignment | M64-01 |
| 3 | M64-03 | Per-arm metrics, guardrails, daily snapshots | M64-02 |
| 4 | M64-04 | Auto-promotion and auto-rollback with the four conditions in III.2 | M64-03 |
| 5 | M64-05 | `same_author` (the cheapest real win on `story.more_by_author`) | M64-01 |
| 6 | M64-06 | `tag_jaccard` + `tag_cooccurrence`, refused by name while tags cover no engaged row | **M63-32** |
| 7 | M64-07 | `crosspost_union` — one story's engagement from every archive | M63-04's resolution rule |
| 8 | M64-08 | `fandom_graph` adjacency, operator-managed | M64-06 |
| 9 | M64-09 | Taste gravity at every surface (III.3), with the per-surface dial | M64-01 |
| 10 | M64-10 | Facets as first-class filters everywhere (length, status, rating, cadence) | M63-04 (`word_count`/`views` are null on engaged rows) |
| 11 | M64-11 | `co_shelving` — reading lists as a first-class feature, then list co-occurrence | lists adoption |
| 12 | M64-12 | `co_read` — session transitions | M64-02 |
| 13 | M64-13 | `engagement_overlap` with k-anonymity and aggregation thresholds | M64-02 + enough readers |
| 14 | M64-14 | Embedding similarity (summary first, full text last), feature-gated on a provider | an AI provider |
| 15 | M64-15 | Operator console: the variant table, hold/promote, preview-as-persona | M64-04 |
| 16 | M64-16 | Offline pre-registration harness on `taste::eval` so a challenger is argued on history before it takes traffic | `crates/taste::eval` |

## The one sentence version

**Tag similarity is your best dollar-for-dollar investment because you already have the data, it's
explainable, and it works on every surface** — but the *endgame* is co-shelving, because a human
curator's reading list captures dimensions of similarity ("vibes") that no tag system ever will.
Build the tag engine now, design reading lists as a first-class feature from day one so the data
accumulates, and the recommendation system gets smarter for free as users use the site.

**Two amendments this instance's data forces on that sentence:** tags currently cover **none** of
the rows that carry an endorsement (M63-32 is the backfill, and until it lands `tag_jaccard`
refuses rather than returning an empty page), and the ONE thing already live — text similarity —
ranks the **unmeasured 99 %** of the catalogue, which is why III.3's gravity is not decoration:
it is what an operator has instead of a taste corpus, until readers supply one.
