# M63-02 — the window grammar, on three clocks

Item: `docs/requirements.csv` M63-02. Plan written before code, per the standing prompt.

## What the item is

`GET /api/v1/catalogue` gains three window parameters, sharing one parser:

| parameter | column it constrains | the question it answers |
|---|---|---|
| `updated` | `library_items.source_updated_at` | when did the author last touch it on the archive |
| `refreshed` | `library_items.last_synced_at` | when did our crawler last recheck it |
| `added` | `library_items.created_at` | when did it enter this catalogue |

Each accepts a window, not just a duration:

```
7d 36h 3w 6m 2y          within the last N (upper bound unbounded)
>7d >1y                  older than N            (upper bound now-N, half-open)
-30d..-7d                between 30 and 7 days ago
2026-10-01..2026-11-01   absolute range, half-open [start, end)
..-7d                    everything older than a week ago
@2026-11-01              that calendar day
today yesterday this-week last-week this-month last-month this-year last-year
any                      no constraint (the default)
```

**Why it is one parser and not three:** `7d` and `>7d` must partition the catalogue exactly.
With `>=` on both ends the boundary row is in both windows and every "how many are old vs new"
answer is wrong by the boundary. Half-open everywhere makes the two sets disjoint and complete.

**Why the parser is in `crates/domain`, not the route:** the CLI (`dh rec --updated '>7d'`,
M63-01) must mean the same thing as the route. A parser per surface forks on the first change.

## The defect this item found before writing any code

The three clocks are **not in one format** on the live database (checked
`select source_updated_at, last_synced_at, created_at from library_items …` on thinkcentre):

| column | observed shape | count non-null of 141,317 |
|---|---|---|
| `source_updated_at` | `2026-10-13` — a bare **date**, no time | 91,471 |
| `last_synced_at` | `2026-11-01T12:14:46+00:00` | 95,036 |
| `created_at` | `2026-10-27T09:13:34+00:00` | all |

Text comparison against an RFC3339 bound is therefore wrong at the edges: `2026-10-13` sorts
before `2026-10-13T00:00:00Z` (prefix), and `…46+00:00` sorts before `…46Z` because `+` (0x2B)
precedes `Z` (0x5A). Both errors are off-by-boundary, which is the hardest kind to notice.

So the predicate **normalises both sides**:

```sql
-- SQLite
datetime(l.library_items_col) >= datetime(?1) AND datetime(l.col) < datetime(?2)
-- PostgreSQL
(l.col)::timestamptz >= (?1::timestamptz) AND (l.col)::timestamptz < (?2::timestamptz)
```

`datetime()` understands the bare date, the `+00:00` form and the `Z` form; PostgreSQL's cast
does too. Both sides go through the same rule, so any session-timezone assumption cancels.

Cost, stated rather than hidden: a function on the column means **no index** for these predicates
until an expression index exists. At 141k rows a scan is milliseconds; the expression index is
recorded as a deferral in the item's notes, with the write-side format fix that would make plain
column comparison safe (filed as M63-31).

## Files

| file | change |
|---|---|
| `crates/domain/src/window.rs` | new: `Bounds`, `parse(raw, now)`, `WindowError`, unit tests |
| `crates/domain/src/lib.rs` | `pub mod window;` |
| `crates/db/src/catalogue.rs` | `CatalogueQuery` gains `updated/refreshed/added: Option<Bounds>`; `search` adds the predicates |
| `crates/app/src/routes/catalogue.rs` | three new query params, parsed with the shared parser; a bad window is a 422 naming the field |
| `crates/db/tests/catalogue_window.rs` | new: partition, NULL exclusion, three timestamp shapes |

## Steps, each with its verification

1. **Parser + unit tests.**
   `cargo test -p darkhold-domain window` → `running N tests … test result: ok.`
2. **DB predicates.**
   `cargo test -p darkhold-db --test catalogue_window` → ok; and the same file with
   `DARKHOLD_TEST_PG_URL` set, on PostgreSQL.
3. **Route params.**
   `cargo test -p darkhold-app --test catalogue_window_route` → ok.
4. **Mutation, both directions.**
   a. Flip `<` to `<=` in the upper bound → the partition test must fail with a duplicate row.
   b. Drop the `datetime()` normalisation → the mixed-format test must fail.
   `cargo test -p darkhold-db --test catalogue_window` after each: FAILED, then green when reverted.
5. **Gates.** `cargo test -p darkhold-domain -p darkhold-db -p darkhold-app --lib` and
   `cargo clippy -p darkhold-domain -p darkhold-db -p darkhold-app --all-targets`. Clippy on the
   workspace currently fails on three pre-existing `work_ratings.rs` warnings; the item's own files
   must produce none.
6. **Deploy.** Blocked this turn: the live instance runs a different lineage (local master is
   ahead 1155 / behind 1046 of `origin/master`; the host tree does not contain this line's
   commits). Recorded in `docs/plans/deploy-lineage-decision.md`, waiting on the operator.
7. **Docs.** `requirements.csv` M63-02 → `implemented-locally-tested` with this file as evidence;
   M63-31 filed for the write-side timestamp format; session note in the vault.

## What this item deliberately does not do

- **No timezone knob.** Calendar words (`this-week`, `last-month`) are UTC. The instance has no
  configured display timezone, and inventing one here would put a second source of truth beside
  the frontend's. Deferral recorded with its reason.
- **No `dh` binding yet.** M63-01/28 consume the parser; this item makes the server side exist so
  they have something to consume.
- **No expression index.** Written above, waiting on M63-31, which is the fix that makes it
  unnecessary.
