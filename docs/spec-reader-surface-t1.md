# Spec — Reader surface, Tier 1: three discovery surfaces that are pure presentation

Date: 2026-10-04. Status: specified, not yet implemented.
Origin: item 14, 27 and 33 of the 100 reader-facing ideas
(`docs/audit-100-reader-ideas-2026-10-04.md`), all Tier 1 there.

## 1. Why these three, and why together

All three render **data this codebase already stores**. None needs a schema change, a new
dependency, or a product decision:

| Item | Data that exists today | Missing |
|---|---|---|
| 14 "New in your fandoms" | `bookmarks`, `work_tags`, `taxonomy_nodes`, `works.published_at` | the query and the section |
| 27 "Most bookmarked this week" | `bookmarks.created_at`, `bookmarks.subject_type/id` | the query and the section |
| 33 "Similar works" | `work_tags`, plus `jaccard_similarity` in `crates/db/src/federation.rs:359` | the query and the section |

Grouping them is deliberate. Each is one SQL query plus one section on a page that already
exists, and together they fill the two dead ends a returning reader hits: **nothing new**
and **nothing related**. Item 33 in particular is the highest-intent moment on the site —
the end of a work — and it currently goes nowhere.

## 2. The rule that decides all three: a reader's own data never crosses the boundary

Item 14 and item 27 both rank by *bookmark activity*. `bookmarks.is_public` defaults to
**0**, so most bookmark rows are private. A query that counts all bookmarks and shows the
result would publish, in aggregate, what individual readers chose to keep private — and
`is_public = 0` is a promise the schema already makes.

**So both count only `is_public = 1` rows.** For item 27 that is not a subtlety, it is the
whole feature: a "most bookmarked this week" leaderboard built on private bookmarks is
exactly the surveillance §46's content policy exists to prevent, and it would be visible to
everyone including non-readers.

This is the single most important line in this spec, and it is the kind of rule that gets
dropped when someone optimises the query later. It gets a test of its own.

## 3. Item 14 — "New in your fandoms"

**What it answers:** a reader who bookmarked three Harry Potter works opens Discover and
sees nothing new. That is the whole failure. This section fixes it with one join.

**Query.** The reader's fandoms are the taxonomy nodes reached from their *public*
bookmarks' tags, restricted to taxonomy. Then the newest published works carrying any of
those nodes, excluding works the reader has already bookmarked.

```
SELECT DISTINCT w.id, w.title, ...
FROM works w
JOIN work_tags wt ON wt.work_id = w.id
WHERE wt.node_id IN (/* the reader's fandom node ids */)
  AND w.lifecycle = 'published'
  AND w.deleted_at IS NULL
  AND w.visibility = 'public'
  AND w.id NOT IN (/* the reader's bookmarked work ids */)
ORDER BY w.published_at DESC
LIMIT ?1
```

**Cap: 12.** A "new in your fandoms" section of 40 works is a second Discover page.

**Empty means empty, not a substitute.** If the reader has no public bookmarks there are no
fandoms, and the section is **not rendered**. It does not fall back to all recent works: a
section that changes subject when it has no data is a section nobody can learn to read.

## 4. Item 27 — "Most bookmarked this week"

**Query.** Public bookmarks on works, in a seven-day window, counted per work.

```
SELECT w.id, w.title, COUNT(*) AS recent_bookmarks
FROM bookmarks b
JOIN works w ON w.id = b.subject_id
WHERE b.subject_type = 'work'
  AND b.is_public = 1                      -- §2
  AND w.lifecycle = 'published'
  AND w.deleted_at IS NULL
  AND b.created_at >= /* now - 7 days */
GROUP BY w.id, w.title
ORDER BY recent_bookmarks DESC, w.title ASC
LIMIT ?1
```

**The window is computed in Rust and passed as a bound parameter**, not written as
`strftime('now', '-7 days')` / `NOW() - INTERVAL '7 days'` inline. The two dialects need
different date arithmetic (`crates/db/src/longevity.rs:44` uses `NOW() - INTERVAL`,
`crates/db/src/payout_store.rs:154` uses `CAST(strftime('%s', …) AS INTEGER)`), so a
bound string keeps one code path and makes the window testable by passing a fixed clock.

**Ties break on title, ascending.** Without it, two works with equal counts come back in
whatever order the engine chose, which makes the test flaky and the page unstable between
renders. This is not decoration — a flaky test gets "fixed" by being retried until green.

**A work cannot appear twice**, so the count is distinct bookmarkers rather than rows:
`COUNT(DISTINCT b.account_id)`.

## 5. Item 33 — "Similar works"

**What it answers:** the reader finishes a work and the page ends. This is the highest-intent
discovery moment on the site.

**Similarity: weighted Jaccard over the reader-visible tag set.** The existing
`jaccard_similarity(a, b)` at `federation.rs:359` is exactly this on `HashSet<String>` and is
**already unit-tested**; reuse it rather than writing a second one. Weights, because tags
are not equal evidence:

| Tag kind | Weight | Why |
|---|---|---|
| fandom | 3 | strongest signal about what a reader wants |
| relationship | 3 | a ship is a very specific request |
| character | 2 | |
| freeform / mood / others | 1 | |

Weighted Jaccard, `Σ min(wi,wj) / Σ max(wi,wj)` over shared vs. union. It stays in [0,1],
so a score is comparable across works and the same helper shape is reusable.

**The cold-start honesty rule.** With few tags, similarity is noise. So:
- a work with **fewer than 2 tags is never recommended** — there is nothing to compare;
- results below **0.15 are dropped**, because "similar" at 0.05 is a lie to the reader;
- if nothing clears the bar, the section is **not rendered**. An empty rail that says
  "Similar works" with nothing in it is worse than no rail.

**Where the query runs.** The candidate set is "works sharing at least one tag", which the
index on `work_tags(work_id, node_id)` answers. Score in Rust, not SQL: expressing weighted
Jaccard in SQL means a dialect-specific CASE expression per tag kind, and the Rust version
is unit-testable without a database.

**Limit 5, cap 4 shown on mobile.** A similar-works rail longer than five stops being a rail.

## 6. Data model changes

**None.** Every field this spec reads already exists. No migration, on either engine. This
is the load-bearing claim of the audit and it is why these three are Tier 1.

## 7. Testing

**Store** (`crates/db/tests/reader_surface_t1.rs`), each dialect-agnostic so it runs on both
engines under the existing harness:

- `t1_new_in_your_fandoms_only_returns_works_in_a_bookmarked_fandom` — a bookmarked fandom's
  new work appears; an unrelated fandom's does not.
- `t1_new_in_your_fandoms_omits_works_you_already_bookmarked` — the reader's own library is
  not "new" to them.
- `t1_new_in_your_fandoms_is_empty_without_public_bookmarks` — a reader with only PRIVATE
  bookmarks gets an empty set, not a fallback to everything.
- `t1_most_bookmarked_this_week_counts_only_public_bookmarks` — **the §2 rule as a test.**
  Ten private bookmarks on one work and one public bookmark on another: the second wins.
- `t1_most_bookmarked_this_week_respects_the_window` — a bookmark 8 days old is excluded; one
  from 2 days ago is counted. Fixed clock, bound parameter.
- `t1_most_bookmarked_this_week_breaks_ties_on_title` — two works with equal counts come
  back title-ascending, deterministically.
- `t1_most_bookmarked_this_week_counts_distinct_readers_not_rows`
- `t1_similar_works_excludes_the_work_itself` — a work is trivially 100% similar to itself.
- `t1_similar_works_drops_results_below_the_floor` — two works sharing one freeform tag
  score ~0.14 and are excluded.
- `t1_similar_works_skips_works_with_fewer_than_two_tags`
- `t1_similar_works_weights_fandom_above_freeform` — identical fandom+freeform counts,
  different fandom overlap ⇒ the fandom match ranks higher.

**Frontend** (`frontend/src/lib/components/SimilarWorksRail.test.ts` and siblings):
renders 5, respects the empty case, and the weight explanation is in the aria text.

**The mutation that must be killed** (per the mutation-gate discipline: a green test never
seen red is not evidence): removing `AND b.is_public = 1` from item 27. The
`most_bookmarked_counts_only_public` test must go red.

## 8. Explicitly not in scope

- Private-bookmark-based recommendations of any kind, including for the reader themself.
- The `ao3_id` column and the cold-start similarity asset — separate plan, and the
  `ao3_id` migration is the prerequisite.
- Editing tags inline from these surfaces. Discovery surfaces are read-only here.
