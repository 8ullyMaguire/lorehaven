# Plan — Reader surface Tier 1 (items 14, 27, 33)

Implements `docs/spec-reader-surface-t1.md`. Written 2026-10-04 before any code.
Every symbol referenced below was confirmed to exist by grep; the ones to note are
recorded in §0.

## 0. Symbols verified before writing this plan

| symbol | location | confirmed |
|---|---|---|
| `jaccard_similarity(&HashSet<String>, &HashSet<String>) -> f64` | `crates/db/src/federation.rs:359` | exists, **private** (needs `pub(crate)`) |
| `db.sql(sqlite, postgres) -> SqlPair` | used by `crates/db/src/dnf.rs:47` | exists |
| `Backend::{Sqlite, Postgres}`, `sqlite_pool()`, `postgres_pool()` | `crate::{Backend, Database}` | exists |
| `bookmarks.is_public INTEGER NOT NULL DEFAULT 0` | `migrations/sqlite/0009_library.sql:69` | exists |
| `bookmarks.{account_id,subject_type,subject_id,created_at}` | `0009_library.sql:55` | exists |
| `works.{id,title,summary,lifecycle,visibility,published_at,deleted_at}` | `0003_works.sql` | exists |
| `work_tags(work_id,node_id,weight)`, PK `(work_id,node_id)` | `0011_taxonomy.sql` | exists |
| `taxonomy_nodes(id,kind,canonical,norm)` | `0011_taxonomy.sql` | exists |
| store test `exec(db, tmpl, args)` with `?N#u` / `?N#i` rewrites | `crates/db/tests/concierge_store.rs:93` | exists — **copy, do not rewrite** |
| store test `connect(tag) -> Database` | `crates/db/tests/concierge_store.rs:38` | exists — copy |

**Two things this plan could have gotten wrong and did not:**
- sqlx does **not** translate `?1` into `$1` for PostgreSQL. Using `?1` yields
  `operator does not exist: ? integer`. The `exec` helper does all three rewrites; a
  hand-rolled query in the new store functions must use the same convention.
- `jaccard_similarity` is `fn`, not `pub fn`. Step S2 makes it visible rather than
  duplicating it.

## S1 — the new store module

`crates/db/src/reader_surface.rs`, new. Three public functions plus one shared row type.

```rust
//! Reader-surface discovery queries: items 14, 27 and 33 of the 100-idea audit.
//!
//! Read-only. None of these writes, and none of them sees a private bookmark --
//! see the is_public predicate in most_bookmarked_this_week, which is the whole
//! reason this file is separate rather than three additions to discovery.rs.

use anyhow::Result;
use serde::Serialize;

use crate::Database;

/// One work as the discovery surfaces need it.
#[derive(Debug, Clone, Serialize)]
pub struct SurfaceWork {
    pub id: String,
    pub title: String,
    pub summary: String,
    pub published_at: Option<String>,
    pub completion: String,
    /// Item 27 only: distinct PUBLIC bookmarkers in the window.
    pub recent_bookmarks: Option<i64>,
    /// Item 33 only: weighted-Jaccard score in [0,1].
    pub similarity: Option<f64>,
}
```

**S1.1 `most_bookmarked_this_week`.** Signature:

```rust
pub async fn most_bookmarked_this_week(
    db: &Database,
    window_start: &str,   // RFC3339, computed by the caller
    limit: i64,
) -> Result<Vec<SurfaceWork>>
```

The window is a **bound parameter**, not inline date arithmetic. The two engines need
different spellings (`longevity.rs:44` uses `NOW() - INTERVAL`, `payout_store.rs:154`
uses `CAST(strftime('%s', …) AS INTEGER)`), and a bound value keeps one code path.

SQLite arm:

```sql
SELECT w.id, w.title, w.summary, w.published_at, w.completion,
       COUNT(DISTINCT b.account_id) AS recent_bookmarks
FROM bookmarks b
JOIN works w ON w.id = b.subject_id
WHERE b.subject_type = 'work'
  AND b.is_public = 1
  AND w.lifecycle = 'published'
  AND w.deleted_at IS NULL
  AND b.created_at >= ?1
GROUP BY w.id, w.title, w.summary, w.published_at, w.completion
ORDER BY recent_bookmarks DESC, w.title ASC
LIMIT ?2
```

Postgres arm is the same with `$1::timestamptz` / `$2` and `w.id::text` on the selects,
because uuid columns do not serialise to `String` without the cast.

`ORDER BY recent_bookmarks DESC, w.title ASC` — the tie-break is load-bearing, not
decoration: without it two equal counts return in engine order, the test is flaky, and the
page reshuffles between renders.

**S1.2 `new_in_your_fandoms`.**

```rust
pub async fn new_in_your_fandoms(
    db: &Database,
    account: &str,
    limit: i64,
) -> Result<Vec<SurfaceWork>>
```

Two statements: the reader's fandom node ids from their **public** bookmarks, then the
newest published works carrying any of them, minus works they already bookmarked. Returns
an empty `Vec` when there are no public bookmarks — never a fallback.

**S1.3 `similar_works`.**

```rust
pub async fn similar_works(
    db: &Database,
    work_id: &str,
    limit: i64,
) -> Result<Vec<SurfaceWork>>
```

Loads `(work_id → Vec<(node_id, weight)>)` for the subject and every work sharing at least
one node, **scores in Rust**, and returns the top `limit`. Scoring in SQL would mean a
CASE expression per tag kind per engine; scoring in Rust is unit-testable with no database
at all, which is where the weight test lives.

**Verify**

```bash
cargo build -p lorehaven-db 2>&1 | tail -5
```
expect: no output, exit 0.

## S2 — make the existing similarity helper reachable, and test the weights

`crates/db/src/federation.rs:359` — change `fn jaccard_similarity` to `pub(crate) fn`, and
export `reader_surface` from `crates/db/src/lib.rs`.

Add `crates/db/src/reader_surface.rs`'s scorer as a **pure function** so it is testable
without a database:

```rust
/// Weighted Jaccard over tag weights, in [0,1].
///
/// `FILTER_FLOOR` is the honesty rule: below it, "similar" is a lie to the reader, and an
/// empty rail is better than a wrong one.
pub const FILTER_FLOOR: f64 = 0.15;
/// Fewer than this many tags and there is nothing to compare.
pub const MIN_TAGS: usize = 2;

pub fn weighted_jaccard(a: &[(String, i64)], b: &[(String, i64)]) -> f64
```

Fandom and relationship weight 3, character 2, everything else 1 — resolved through
`taxonomy_nodes.kind`, so the weight lives in one function rather than being baked into the
caller.

**Verify**

```bash
cargo test -p lorehaven-db reader_surface 2>&1 | tail -8
```
expect: the pure scorer tests pass with no database.

## S3 — the store tests, on both engines

`crates/db/tests/reader_surface_t1.rs`, **copying `connect` and `exec` verbatim** from
`concierge_store.rs` (they exist because this crate cannot depend on `test_support`).

The eleven tests are listed in spec §7. Write them in this order: the
`most_bookmarked_counts_only_public_bookmarks` test **first**, because it is the one that
has to fail when the predicate is removed, and a test written last is a test written to fit
the code.

**Verify**

```bash
cargo test -p lorehaven-db --test reader_surface_t1 2>&1 | tail -12
```
expect: 11 passed.

Then on PostgreSQL, which is where dialect defects actually live:

```bash
export LOREHAVEN_TEST_PG_URL='postgres://lorehaven:lorehaven@127.0.0.1:55433/postgres'
cargo test -p lorehaven-db --test reader_surface_t1 2>&1 | tail -12
unset LOREHAVEN_TEST_PG_URL
```
expect: 11 passed. SQLite accepts SQL PostgreSQL refuses; milestone 8 shipped four defects
that only this run found, which is why it is not optional.

## S4 — mutation-verify the privacy rule

Remove `AND b.is_public = 1` from the SQLite and Postgres arms. Run
`cargo test -p lorehaven-db --test reader_surface_t1`. **Expect the
`most_bookmarked_counts_only_public_bookmarks` test to fail and the other ten to stay
green** — one test red, not a cascade. Restore from git and confirm green.

A test never seen red is not evidence. If several tests go red, the fixtures share state
and that is its own bug to fix before proceeding.

## S5 — HTTP routes

`crates/app/src/routes/reader_surface.rs`, new. Three GET routes under the existing
discover surface. Read `requires_auth` for item 14 (it is per-reader) and leaves items 27
and 33 **public**, because both are computed only from public rows and a login wall around
public data teaches readers that the data is not public.

Every response that can be empty returns `{"works": []}`, never `null` — a client
iterating the result should not have to handle null (the Concord lesson, which applies
here identically).

**Verify**

```bash
cargo test -p lorehaven-app reader_surface 2>&1 | tail -8
```

## S6 — the frontend sections

`frontend/src/lib/components/NewInYourFandoms.svelte`,
`MostBookmarkedThisWeek.svelte`, `SimilarWorksRail.svelte`.

**The empty case is the rule, in all three.** A section that renders a heading with nothing
under it is worse than no section: it teaches the reader that the site has nothing to show.
Each component returns nothing when its list is empty.

The three are siblings and could become one `DiscoveryRail` component with a `kind` prop —
**and that is the wrong call here**, because their data sources, empty reasons and copy all
differ, and a shared component would need a `kind` switch in five places. Three small
components is the smaller total.

`SimilarWorksRail` shows 5, and the aria text names why each was included ("shares the
fandom Harry Potter") rather than saying "similar".

**Verify**

```bash
cd frontend && ./scripts/fe.sh test 2>&1 | tail -8
cd frontend && ./scripts/fe.sh check 2>&1 | tail -5
```

## S7 — Playwright

Through `./scripts/fe.sh` only — the binary embeds `frontend/dist` at **compile** time, so
`vite build` alone changes nothing served, and `fe.sh` catches a stale binary BY NAME.

`frontend/e2e/reader-surface.spec.ts`: with a seeded reader holding one public bookmark in
one fandom, Discover shows a "New in your fandoms" section containing the new work and not
the unrelated one; the bookmarked work is absent; with no public bookmarks the section is
absent; and the similar-works rail on a work page shows five cards.

Each journey verified **both ways**: passes, and fails when the `is_public = 1` predicate is
removed. A journey never seen red proves nothing.

## Definition of done — ALL MET 2026-10-04

- [x] `cargo build` clean; `cargo clippy --all-targets -- -D warnings` clean
- [x] **18** store tests pass on **SQLite and PostgreSQL** (11 planned; 7 added because the
      plan's list under-counted what the queries actually do — drafts/deleted works,
      unreadable subjects, and the scorer's own bounds)
- [x] the `is_public` mutation turns exactly **one** test red and leaves 17 green
- [x] three routes respond, with `{"works": []}` on empty — 8 route tests, none 404
- [x] `fe.sh test` green (479 tests, 70 files) and `fe.sh check` clean for every file
      this work touches
- [x] `scripts/check-page-headings.py` green — 44 pages, all with an `<h1>`
- [x] Playwright journeys pass (`frontend/e2e/reader-surface.spec.ts`, 6 tests)
- [x] the audit document updated: **81 exist, 19 do not**, and the two rows whose probe
      was wrong (items 8 and 22) say so in place
- [x] requirements ledger: rows `M57-T1-14`, `M57-T1-27`, `M57-T1-33`, `M57-T1-W`

## What the plan got wrong, and what it missed

Four corrections, recorded because the plan is a contract the next reader will trust.

**S1 was half-migrated when I arrived.** `new_in_your_fandoms` interpolated its id lists
with `format!("'{}'", ...)` while `similar_works` bound them — and the file did not
compile, because the bind loop still referenced a variable the rewrite had deleted. The
two styles had been left mid-migration by an earlier pass. The interpolated form is also
the one the module's own comment on `similar_works` explains is wrong: `works.id` is uuid
on PostgreSQL and TEXT on SQLite, so a quoted literal list compares uuid against text on
one engine and works on the other. Both are bound now.

**A sqlx pool-type trap the plan never mentions.** Hoisting `let q = query_as(...)` above
a `match db.backend()` fixes the pool type to whichever arm the compiler resolves first,
and the other arm fails with `type mismatch ... expected Sqlite, found Postgres` — a
database-layer error that is really a lexical one. The query is built inside each arm.

**The 7-day window made the plan's own test guidance wrong.** S3 says a store test may pass
a fixed clock, which is true and is what the store tests do. It does not transfer to a
*route* test: the route computes the window from `Utc::now()`, so a fixture pinned to a
literal date sits nine months outside it and the leaderboard correctly returns nothing.
Three route tests failed for exactly this before the fixtures were made relative.
**Anything that asserts a window belongs at the store level, where the window is an
argument.**

**Validation is 422 here, not 400.** `AppError::Validation` maps to 422 across this
codebase (`error.rs::status_code`). `AppError::field(...)` is the constructor that also
populates `field_errors`; hand-building the struct is what produced the first 400.

## One thing in the tree this work REMOVED

`migrations/{sqlite,postgres}/0116_reader_work_status.sql` (untracked, from an earlier
pass) created a `reader_work_status` table for §57.4's DNF. **DNF already exists**:
`0073_dnf_reasons.sql` has `did_not_finish`, with six structured reasons, an `is_public`
flag and a `note`, plus `works.allow_dnf_feedback` — all shipped in M45-21 and wired
through `crates/{domain,db,app}`'s `dnf.rs`. The 0116 table was a second implementation
of the same feature with a different (weaker) design, referenced by **no code**. Migrations
are embedded from the directory by `crates/db/build.rs`, so it would have shipped on every
instance's next `migrate`. Removed; §57.4 should be closed against 0073, not reimplemented.
