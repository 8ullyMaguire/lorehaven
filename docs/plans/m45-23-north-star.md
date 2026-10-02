# M45-23 — North-star metric with per-mechanism attribution

**Status:** planned → spec written, not yet implemented. This plan is executable by an LLM with
no other context.

## What this row is

`docs/requirements.csv` M45-23:

> North-star metric with per-mechanism attribution
> Notes: Gaps review B5. Works rated per month + time-to-find; attribute each loved work to
> the surfacing mechanism.

## The finding that shapes this row

**The prerequisite the design review called "impossible to retrofit" is already built.**
`docs/spec-gaps-design-review-2026-09-22.md` line 474 lists as item 5 of the priority order
and says, of A4: *"logging selection probabilities per ranked slot … Without them, none of
the historical data can be used for unbiased evaluation later."*

That is done: **M45-13**, "Uniform-random exploration slots with per-slot
selection-probability logging", `implemented-fully-tested`, and migration `0098_ranking.sql`
adds `recommendation_slots.propensity REAL` with a comment explaining that historical rows
are NULL rather than defaulted, because the counterfactual that would have been logged no
longer exists.

**So the honest half of the problem — propensity — is available. The other half is not, and
this is the finding that decides the shape of the work:**

`recommendation_slots` records `pseud_id`, `work_id`, `request_id`, `position`, `reasons`,
`recipe_stage`, `instance_curation`, `blend_score`, `propensity`, `slot_kind`. **It does not
record which mechanism surfaced the work.** `crates/db/src/recommendation_slots.rs` has no
mechanism or strategy column either.

That is the whole of M45-23's second clause, and it is a one-line-per-row fact that has to be
written **at serve time**, because it is unrecoverable afterwards — the same argument
migration 0098 makes about `propensity`. So the order is: write the mechanism down, then
attribute. Building the attribution query first would produce a metric attributed to
"unknown" forever.

## What the spec already settles

§53.5 and §53.6 are written and constrain this row tightly. Read them before designing
anything; they are not optional context.

- **§53.5** — the feed-quality rate. Two rules that matter here:
  - *"It reports its own missing inputs."* With no ratings and no completions the rate is
    **undefined, not zero**. Reporting zero reads as total failure and invites a change to a
    recipe that has simply not been tested yet. A missing-inputs field is therefore part of
    the response, not a nicety.
  - The numerator and the denominator are scoped the same way.
- **§53.6** — the earned-bookmark ratio, *"the ratio that cannot be faked."* The definition
  contains a deliberate non-obvious choice:
  - **A "hit" here is a completion, and it is `finished` alone** — *not* §53.5's "finished or
    rated >= 4". The two definitions coexist deliberately, and the spec says why: reusing
    §53.5's would put the cheap signal (a four-star rating, one click) inside the expensive one
    (a completion, a reader's time), defeating the ratio's purpose.
  - **Per work, over a window, with the denominator reported.** A ratio whose numerator is
    windowed and whose denominator is not is not a ratio.
- **§53.2** — no per-account detail in an economy or metrics view. `FlowSummary::
  carries_account_detail()` is the existing type-level assertion of that pattern.
- **§0.3** — the metric is *read*, never *chased*. See "What this deliberately does not do".

## Design decision: the metric, and what "attribution" means

The row says "north-star", which is one word for two different things. The note splits it:
*"Works rated per month + time-to-find; attribute each loved work to the surfacing
mechanism."* So there are **two measures**, and conflating them would produce a number nobody
can act on.

| Measure | Definition | Source |
|---|---|---|
| **output** | distinct works rated >= 4 per month | `ratings` |
| **time-to-find** | median days from a work's first *earned* view to its first >= 4 rating | `work_view_log` → `ratings` |

**"Loved work" is `rating >= 4`, and that is the join key for attribution.** A work is loved
when a reader rated it >= 4; the mechanism is whatever put it in front of that reader.
Attribution therefore reads `recommendation_slots`, matches the reader + work, and reports
the mechanism that served it.

**A loved work with no slot row is attributed to `unattributed`, and counted.** Same rule as
M45-18's undeclared mechanisms and for the same reason: a work a reader loved is a fact, and
a dashboard that drops the ones it cannot explain reports a smaller success than exists.
This is the single most important property of the row.

**A work served by several mechanisms is attributed to the earliest slot that preceded the
rating**, not to all of them and not to the last. Earliest is the one that had the causal
claim: the later slot did not cause a rating that had already happened. Counting it twice
would inflate the winner and deflate everyone else.

## Step 1 — record the mechanism at serve time

**Migration 0113**, both engines: `ALTER TABLE recommendation_slots ADD COLUMN mechanism TEXT`.
Nullable, for the same reason `propensity` is: historical rows have no mechanism and it
cannot be recovered.

Do **not** backfill it from `reasons`. `reasons` is a reader-facing JSON explanation written
for a person; parsing it to recover a mechanism would make the metric depend on prose. NULL
means unattributed, which is honest.

The write site is wherever a slot row is inserted — read it, do not assume. The column must be
populated in the same statement that writes `position`, or a serve path will be missed and
silently produce `unattributed` rows forever.

## Step 2 — the attribution store

**File:** `crates/db/src/north_star.rs` (new)

```rust
pub struct NorthStar {
    pub works_rated_per_month: f64,
    pub median_days_to_find: Option<f64>,
    pub rated_works: i64,
    pub loved_works: i64,
    pub unattributed: i64,       // loved, no slot row
    pub by_mechanism: Vec<MechanismAttribution>, // key, loved_works, share
    pub missing_inputs: Vec<MissingInput>,      // see 53.5
}
```

- `median_days_to_find` is `None`, never `0.0`, when there is no completed pair. §53.5.
- `missing_inputs` names what was absent (`ratings`, `completions`, `slots`) so the response
  reports its own incompleteness instead of a zero that reads as failure.
- Percentile: compute in Rust from ordered counts, not `PERCENTILE_CONT` — SQLite has no
  such function, and this codebase already composes scores in Rust for that reason (see
  `rec_strategy::hidden_classics_strategy`, which does this because SQLite lacks `LOG10`).
- The window is inclusive at both ends, like every other window in this codebase.

## Step 3 — the route

`GET /admin/metrics/north-star?since=&until=`, operator-only, `RouteClass::Default`,
404-not-403 — the same three constraints M45-18's route carries, and copy its `require_operator`
reasoning verbatim rather than reinventing it.

**No per-account anything.** §53.2. The response is instance-level aggregate only.

## Step 4 — tests

**File:** `crates/app/tests/north_star.rs` (new)

Cases that will not pass by accident:

1. **A loved work with no slot row is counted and attributed to `unattributed`.** The
   headline property. Fails if the store filters unattributable rows out.
2. **The earliest slot wins.** Seed one work served twice, rated after both. Assert the
   attribution is the first. A "last slot" or "all slots" implementation fails this.
3. **A slot recorded *after* the rating does not claim it.** A slot served an hour after the
   rating cannot have caused it. This is the case that separates the two.
4. **`median_days_to_find` is `None`, not `0.0`, with no completed pair.** §53.5. Assert the
   JSON has `null`, not `0` — a test on the Rust type alone would miss a serialiser that
   turns `None` into `0`.
5. **A rating of 3 is not a loved work.** The `>= 4` boundary, tested at 3 and at 4.
6. **`finished` alone counts as a hit.** §53.6's deliberate narrow definition: a 4-star rating
   with no completion does **not** earn the denominator, and that must be asserted directly or
   the cheap signal will drift back in.
7. Attribution shares sum to 1.0 across mechanisms **plus** `unattributed`.

Case 6 is the one most likely to be got wrong, because reusing §53.5's "finished or rated >= 4"
looks like the obvious implementation and the spec explicitly forbids it.

## What this deliberately does not do

- **No composite score.** §0.3 and the standing rule against one: a single "north-star number"
  is a ranking signal waiting to be wired up. Two named measures, no arithmetic on them.
- **No per-account breakdown.** §53.2.
- **No target, no grading, no "you should be at 40%".** §53.5: *"The number is read, not
  chased."* A window with a low rate is a statement about the recipe or the import mix, and
  the response is to change one of those deliberately.
- **No backfill of `mechanism` from `reasons`.** See step 1.

## Verify the whole step

```bash
cd ~/code-local/rust/lorehaven && export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all
cargo clippy -p lorehaven-db -p lorehaven-app --all-targets   # must print nothing
cargo test -p lorehaven-db -p lorehaven-app --no-fail-fast -- --test-threads=4
LOREHAVEN_TEST_PG_URL='postgres://postgres:smoke_pw@127.0.0.1:5432/postgres' \
  cargo test -p lorehaven-db -p lorehaven-app --no-fail-fast -- --test-threads=4
```

**Do not run a mutation harness concurrently, and do not `git add -A` while one runs.** A gate
edits the source under test; a concurrent `cargo test` compiles mutated source and reports the
gate's intended RED as a failure, and a broad add stages whichever mutation is applied at that
moment. That has produced four false results in this project, one of which shipped a bug.