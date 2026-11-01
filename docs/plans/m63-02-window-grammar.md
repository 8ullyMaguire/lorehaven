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

## What landed, and the two things that changed on the way

Shipped `b5f5b9cf` (the item's second commit; `9b1bb539` was the grammar itself). Two findings
rewrote the plan's own claim about how bad the timestamp problem was:

### 1. The formats are four, not three

Measured on the live database by shape:

| shape | rows | example |
|---|---|---|
| `YYYY-MM-DD` | 63,947 | `2026-10-13` |
| `YYYY-MM-DDTHH:MM:SS` (**no zone**) | 27,461 | `2026-10-27T01:04:45` |
| `…Z` | 60 | `2026-09-20T00:00:00Z` |
| `…SS.NNNNNN` (no zone) | 3 | `2025-04-04T15:18:27.394000` |

The zone-less majority is the one that matters, and it does not "drop rows" — it **moves the
window**. PostgreSQL's live session runs with `timezone = Europe/Madrid` (measured), so a plain
`::timestamptz` reads a zone-less value as Madrid wall time … and reads the bound the same way. Rows
and bounds stay consistent with each other, so nothing looks broken; what shifts is the whole
window. On production `@2026-10-27` covered 23:00 on the 26th to 23:00 on the 27th, and a row stored
as `2026-10-26T23:30:00Z` — an instant, not a wall clock — fell inside a day it is not in. SQLite
reads both sides as UTC, so the two engines disagreed.

Fixed by making the shape decide the reading, on both sides of every comparison:

```sql
-- the column
CASE WHEN col ~ '(Z|[+-][0-9]{2}:[0-9]{2})$' THEN (col)::timestamptz
     ELSE ((col)::timestamp AT TIME ZONE 'UTC') END
-- the bound (always formatted naive-UTC by Bounds::from_sql)
((?::timestamp) AT TIME ZONE 'UTC')
```

Both are now independent of the session timezone, so the engines agree by construction.

### 2. Deploy was not blocked by a lineage after all

`~/code/rust/darkhold` is a **symlink to the thinkcentre tree over the mount**
(`/home/alvaro/mnt/thinkcentre/personal/documents/code/rust/darkhold`). 46 of its 51 "uncommitted"
files were byte-identical to this line's HEAD: the deploy tree had simply never committed the
catalogue work. Preserved on branch `wip/deploy-checkout-2026-11-01`, then reset to this line — one
history again. `docs/plans/deploy-lineage-decision.md` is resolved by that, and the deploy ran from
the shared tree.

## Verification

| claim | command | output |
|---|---|---|
| grammar | `cargo test -p darkhold-domain --lib window` | 25 passed |
| predicates, both engines | `cargo test -p darkhold-db --test catalogue_window` | 4 passed (SQLite) / 4 passed (PG, session set to `Europe/Madrid`) |
| wiring | `cargo test -p darkhold-app --test catalogue_window_route` | 2 passed |
| lib gates | `cargo test -p darkhold-domain -p darkhold-db -p darkhold-app --lib` | 236 / 157 / 784 passed |
| live build | `curl /api/v1/meta` | `Darkhold 0.1.0 0.1.0+b5f5b9cf` |
| live filter | `curl '…/catalogue?updated=7d&limit=2'` | `200`, 2 items |
| live validation | `curl '…/catalogue?updated=nonsense'` | `422 VALIDATION_FAILED "updated: window needs a whole number before its unit"` |
| fix compiled in | `strings <binary> \| grep -c "AT TIME ZONE 'UTC'"` | 7 |

Mutations, all reverted green afterwards:

| mutation | test that failed |
|---|---|
| upper bound `<` → `<=` | `a_window_partitions_the_catalogue_exactly` |
| drop `datetime()` on SQLite | `mixed_timestamp_shapes_compare_as_instants` |
| route accepts `updated` and drops it | both route tests |
| PostgreSQL column read as session-local | `a_zone_less_bound_does_not_move_the_window`, `mixed_timestamp_shapes_compare_as_instants` |
| PostgreSQL bound read as session-local | `a_zone_less_bound_does_not_move_the_window` |

## The limitation this does not fix

No live row currently stores a timestamp inside the one-hour band the session timezone was shifting,
so the endpoint returns the same rows before and after: 10 for `@2026-10-27`, and the UTC-anchored
SQL reference also says 10. The behaviour is fixed and the fixture proves it; the live data cannot
demonstrate it, and saying otherwise would be a claim about rows that do not exist.

Still open, unchanged: the predicate is a function call on the column, so it does not use an index.
M63-31 (normalise at the writer, backfill) is what removes both the function and the deferral.
