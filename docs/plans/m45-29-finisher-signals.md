# M45-29 — Finisher signals (§60): cadence and completion history, positive framing only

**Status:** planned 2026-11-02 against measurement. Migration `0138`. Module `finishers`, **off by
default** (`default_on: false`, relevance 3) — the row's own note says so: *"'update pls' pressure is
considered rude."*

## The idea, as the gaps review wrote it (C5)

> Show update cadence ("updates about every two weeks") and completion history ("finished 9 of 10
> works"), with a Finisher badge. Positive framing only; never label non-finishers. Authors can
> optionally enable completion pledges through existing reserve/capture escrow. Off by default.

## Premises, measured 2026-11-02 on the reference instance

| claim | measurement | verdict |
|---|---|---|
| completion history is computable | 58,042 works: `in_progress` 42,531, `abandoned` **15,220**, `complete` **291**; `owner_pseud_id` present on all | holds |
| cadence is computable | `revisions` spans **2 distinct days** (`2026-09-20`, `2026-10-27`) across 127,474 rows; 57,712 works have units, max 784 units on one work | **no**: an import writes every revision on the day it ran, so a gap is the import's, not the author's |
| authors exist to show signals for | `count(distinct works.owner_pseud_id)` = **3** | the surface is real but the population is a toy |
| "existing reserve/capture escrow" can hold a pledge | `credit_holds` exists (`reserve_hold`/`release_hold`/`capture_hold`), keyed by **`job_id`**, and `capture_hold` posts **no ledger entry**; no sink is defined for a burned pledge | **not without inventing money rules** |

Two consequences:

1. **Cadence must be able to say "unknown".** A median gap over revisions that all landed on one day
   is not a cadence; it is the import. So the answer is `{measurable: false, reason}` for those, and
   `{measurable: false}` is a **result**, not an error — the same shape §58.2 uses for a drought.
2. **Pledges are deferred, by name.** `credit_holds` is a queue hold: its key is a `job_id`, and
   capturing one changes no balance. A "pledge" built on it would look like it holds credits and
   would not — worse than no pledge, because an author would believe a commitment had been recorded
   as money. The economy milestone owns `post_transaction` legs, a balance check and a sink bucket;
   this row records the deferral rather than inventing the rules.

## Step 1 — domain (`crates/domain/src/finishers.rs`, pure)

```rust
pub enum Cadence { Measurable { median_days: i64, samples: usize }, Unmeasured { reason: String } }
pub struct History { pub finished: i64, pub concluded: i64 }
pub fn cadence(days: &[i64]) -> Cadence;          // median of gaps in whole days
pub fn history(finished: i64, abandoned: i64) -> History;
pub fn summary(h: &History) -> String;            // "finished 9 of 10 concluded works"
pub fn is_positive(summary: &str) -> bool;        // the framing guard, tested
```

`History` deliberately has **no** `abandoned` field: the count exists inside `history()` to compute
`concluded`, and stops there. A field named `abandoned` on a public payload is a label, and C5's rule
is that the archive never labels a non-finisher. The ratio "9 of 10" is the most the surface says,
which is exactly the example the review gives.

## Step 2 — store (`crates/db/src/finishers.rs`)

| function | rule |
|---|---|
| `author_history(db, pseud_id)` | `finished` = works with `completion='complete'`; `concluded` = complete + abandoned. No other completion state counts. |
| `work_dates(db, work_id)` | the distinct `substr(created_at,1,10)` of `revisions` joined through `units`, ascending |
| `author_dates(db, pseud_id, limit)` | the same per work, for works with ≥3 revisions, capped — the sample the median is taken over |
| `has_revisions(db)` | one catalog query, so a schema without `revisions` answers in words (M63-33 shape) |

No new table for signals: a stored badge would need an invalidation story every time a work's
completion changed, and the counts are a `GROUP BY` over two columns.

**Migration `0138`** therefore only adds what has no home: `completion_pledges` — **not used by any
route in this milestone**. It is created with the rest because a later pledge milestone must not need
a second migration to add a table whose shape is already argued here; the route refuses pledges today
with the reason above. Two columns, a state CHECK and no FK (works ids are TEXT here, as in 0137).

Hold on — creating a table with no writer is a liability, and the repo's rule is "existing schema
over new tables". Better: **no migration at all**. The signals are derived, pledges are deferred, and
the deferral does not need a table to be honest. `0138` is dropped from this milestone; the row's
notes carry the pledge decision.

## Step 3 — routes (`crates/app/src/routes/finishers.rs`), module `finishers`

| method + path | who | answers |
|---|---|---|
| `GET /api/v1/authors/{pseud}/finisher-signals` | anyone | `{finished, concluded, summary, cadence: {measurable, median_days?, samples?, reason?}}` |
| `GET /api/v1/works/{id}/cadence` | anyone | `{measurable, median_days?, samples?, reason?}` |

Both read doors, no writes: a signal is derived, and a derived thing with a write door is a thing
someone will eventually correct by hand.

Module row: id `finishers`, relevance **3**, `default_on: false`, `Toggle::Operator`, spec §60, no
nav entry (the signal is a line on an author page).

## Step 4 — tests

`crates/db/tests/finishers.rs` (both engines), 8 cases: history counts only `complete` as finished
and `complete`+`abandoned` as concluded (`in_progress` is neither); an author with no works answers
zeroes rather than nothing; dates come back ascending and distinct; a work whose revisions all landed
on one day is **unmeasured** with a reason; a work with two days has a median of that gap; the sample
is capped; a schema without `revisions` answers `measurable: false` with a reason and no error; the
summary string never contains a negative word.

`crates/app/tests/m45_29_finishers.rs`, 6 cases: both doors are public; a missing pseud is `404`;
the payload has **no key** named `abandoned` or `unfinished`; the summary for 9/10 reads
"finished 9 of 10 concluded works"; cadence for an import-shaped work says why; the module is
registered **disabled by default** and enabling it by flag is the operator's call.

`crates/domain/src/finishers.rs` unit tests: median of an odd and an even count; a zero gap is 0
days not a division by zero; empty input is `Unmeasured`; `is_positive` rejects "abandoned",
"unfinished", "incomplete", "failed", "stalled".

## Verification

| # | command | expected |
|---|---|---|
| 1 | `cargo test -p darkhold-domain --lib finishers` | 6 passed |
| 2 | `cargo test -p darkhold-db --test finishers` | 8 passed |
| 3 | same with `DARKHOLD_TEST_PG_URL` | 8 passed |
| 4 | `cargo test -p darkhold-app --test m45_29_finishers` | 6 passed |
| 5 | mutations | every one caught |
| 6 | `python3 scripts/ideas_catalogue.py` | 176 ideas, 0 homeless |

Mutations that must be caught: count `in_progress` as concluded; count `abandoned` as finished; put
`abandoned` back on the payload; median over raw gaps rather than differences; treat one-day spans as
a 0-day cadence; answer `measurable: true` when there are no revisions; `default_on: true` for the
module.

## Deferred, by name

- **Completion pledges via escrow.** `credit_holds` is a `job_id`-keyed queue hold and capturing one
  moves no credits; a pledge needs a balance check, a ledger leg each way and a defined sink. That is
  the economy's work, and building it here would ship a commitment that looks like money and is not.
- **A stored Finisher badge.** A badge is a threshold, a threshold is a policy, and the review does
  not give one. The surface says what was finished; a badge can be a `WHERE finished >= N` over the
  same counts when someone decides N.
- **Update cadence for imported works.** Not deferred by choice — unmeasurable, and the answer says so.
