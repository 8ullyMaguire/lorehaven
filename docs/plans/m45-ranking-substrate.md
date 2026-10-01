# Plan — spec §47, the ranking substrate

Implements `docs/spec.md` §47 (added 2026-09-30) and closes five M45 rows:
**M45-13** (propensity logging), **M45-11** (earned vs incentivized), **M45-15**
(exposure floor), **M45-49** (MMR + satiation), **M45-10** (Scout value).

**Why this is first.** Every remaining M45 row assumes a ranking substrate. None
exists: `personalized_recommendations` (`crates/db/src/discovery.rs:209`) orders
by `w.updated_at DESC` (lines 238/255 — the two dialect arms). The 45 rows are
not tuning changes to an existing ranker.

**Why the order below.** M45-13 and M45-11 are marked *retrofit-critical* in
`requirements.csv` and the marking is not stylistic. A propensity that was not
logged cannot be recovered — the counterfactual is gone — so they are steps 2 and
3, before anything that generates impressions. M45-15, -49 and -10 depend on
the impression log existing. Building -49 before -13 would mean re-ranking work
nobody can evaluate.

Read `docs/spec.md` §47 first. It is the contract; this file is the sequence.

---

> **This plan has now been corrected four times by implementing it, which is the
> argument for writing it before the code.** A plan naming a plausible API
> instead of the real one produces work that does not compile and teaches the
> implementer to trust the plan. What each round caught:
>
> 1. **`migration_parity` does not exist.** The real test is `migration_catalogue`
>    (`crates/db/tests/`), and it compares migration *declarations*, not columns —
>    so it will not catch a `REAL` where PostgreSQL needs `DOUBLE PRECISION`.
> 2. **The whole CREATE-based schema was wrong.** The obvious design adds an
>    `impressions` table and an `interactions` table. Both duplicate tables that
>    already exist: `recommendation_slots` (0076, §33.3) already logs every served
>    slot, and `work_view_log` / `work_kudos` (0068, §9.4–9.5) already record
>    reads and kudos. Migration 0098 is therefore **ALTER, not CREATE** — it adds
>    `slot_kind` and `propensity` to the first and `kind`/`source`/
>    `obscurity_at_read` to the second and third. Two copies of a record, with
>    nothing keeping them in step, is the failure mode.
> 3. **`chrono` is not a dependency of `lorehaven-db`.** The crate uses the
>    workspace `time` crate (`crates/db/Cargo.toml:26`); see
>    `crates/db/src/economy.rs:227`. And `sql_owned` takes `String`, not `&str`.
> 4. **A unit test caught a real defect in the spec's own formula.**
>    `scout_value` as first written was `a.clamp(0,1) * b.clamp(0,1)`, and
>    `f64::clamp` *propagates* NaN rather than clamping it (verified by running
>    it: `f64::NAN.clamp(0.0,1.0) == NaN`, while `5.0.clamp(0.0,1.0) == 1.0`). A
>    NaN rating therefore produced a NaN credit value, and a NaN summed into a
>    ledger poisons every total it touches. NaN is now filtered before the clamp
>    and becomes **0.0, not 1.0** — under-crediting loses one payout, over-crediting
>    mints credit the closed loop in §17 exists to prevent.
>
> **All seven steps are implemented and committed** (through `586e7ad`). Step 7
> was not a wiring exercise: it exposed three defects in `rank_works` that no test
> in its own module could see, and the workspace gate then exposed a conflict
> between two of the plan's own requirements. Both are written up under *What
> wiring the route found* below — read that before changing anything in
> `crates/db/src/ranking.rs`.

## Symbols this plan uses (verified against the tree)

| Symbol | Location |
|---|---|
| `personalized_recommendations` | `crates/db/src/discovery.rs:209` |
| `public_recommendations` | `crates/db/src/discovery.rs:148` |
| `content_filter_sql::for_pseud` / `::exclusion_for` | `crates/db/src/content_filter_sql.rs` |
| `sql_owned(db, sqlite, postgres)` | `crates/db/src/lib.rs:395` |
| `crate::Backend::{Sqlite,Postgres}` | `crates/db/src/lib.rs` |
| `lorehaven_domain::WorkId` (has `WorkId::new()`) | `crates/domain/src/ids.rs:107` |
| id generation: `Uuid::new_v4().to_string()` | e.g. `crates/db/src/admin.rs:21` |
| `TestDb::connect_with_dir(tag, dir)` | `crates/test-support/src/lib.rs:283` |
| `scratch_dir(tag)` | `crates/test-support/src/lib.rs:143` |
| `jobs::enqueue` | `crates/db/src/jobs.rs:135` |

**NEW in this plan** (does not exist yet — introduced by the step that names it):
`rank_works`, `Impression`, `SlotKind`, `log_impression`, `record_interaction`,
`InteractionKind`, `scout_value`, `mmr_rerank`, `Satiation`.

Highest migration number in the tree is **0097**. This plan adds `0098` and
`0099`. Write both dialects — every step below has a PostgreSQL arm, because a
suite pinned to SQLite ships the PostgreSQL path unverified.

---

## Step 1 — migration `0098`: the log columns (DONE)

**Files:** `migrations/sqlite/0098_ranking.sql`, `migrations/postgres/0098_ranking.sql`

**ALTER, not CREATE.** The tables that already record served slots, reads and
kudos are `recommendation_slots` (0076), `work_view_log` and `work_kudos` (0068).
This migration adds what §47.3 and §47.4 need and those tables lack:

| Table | Added | Why |
|---|---|---|
| `recommendation_slots` | `slot_kind TEXT NOT NULL DEFAULT 'ranked'` + CHECK | §47.3 — which pool the row was drawn from |
| `recommendation_slots` | `propensity REAL` **NULLABLE** + CHECK | §47.3 — the selection probability |
| `work_view_log` | `kind`, `source`, `obscurity_at_read REAL NOT NULL DEFAULT 1.0` | §47.4, §47.7 |
| `work_kudos` | `kind`, `source`, `obscurity_at_read REAL NOT NULL DEFAULT 1.0` | §47.4, §47.7 |

**Three defaults, each deliberate and each pointing a different way:**

- `propensity` is **NULLABLE**, with no default. A row written before this
  migration has no selection probability and it cannot be reconstructed — the
  counterfactual is gone. Defaulting to 1.0 would be the quiet version of the
  defect this migration exists to close: a value that looks usable, is not, and
  silently biases every offline estimate. NULL is distinguishable; 1.0 is not.
- `kind` defaults to **`'earned'`**, which is *correct* rather than convenient:
  no incentive existed before this migration, so every historical row genuinely
  was earned.
- `obscurity_at_read` defaults to **`1.0`** ("not obscure"), so historical rows
  earn **no** scout value. Under-crediting loses a payout; over-crediting mints
  credit the closed loop in §17 exists to prevent.

Note the asymmetry: the column default is `'earned'` and the code default in
`InteractionKind::for_source` is also `Earned` — but for the **opposite** reason,
and that is not an inconsistency. The column default is safe because the world
changed; the code default is deliberately unsafe because a *new incentive* would
then be silently counted as earned, and making that a visible edit to one `match`
arm is the point.

**Dialect differences:** `propensity` is `DOUBLE PRECISION` on PostgreSQL, not
`REAL` — PostgreSQL's `REAL` is 4 bytes and a probability that has been through
several multiplications needs 8. Verified on the running server.

**Verified:**
```sh
cargo test -p lorehaven-db --test migration_catalogue   # 6 passed
```
Plus a behavioural probe against both engines, because the catalogue test does not
compare columns. On PostgreSQL, applied all 98 migrations (277 tables, no
errors), then confirmed the constraints actually reject:

| Insert | Result |
|---|---|
| `propensity = 0` | REJECTED — `recommendation_slots_propensity_check` |
| `propensity = 1.5` | REJECTED — same |
| `slot_kind = 'bogus'` | REJECTED — `recommendation_slots_slot_kind_check` |
| `propensity` omitted | ACCEPTED (historical row) |
| `work_kudos.kind = 'bogus'` | REJECTED — `work_kudos_kind_check` |
| `work_kudos.kind = 'incentivized'` | ACCEPTED |
| `work_view_log` default | `earned`, `obscurity_at_read = 1.0` |

**SQLite caveat for the implementer:** SQLite has no `gen_random_uuid()`; write
probe rows with literal ids. Both engines otherwise behave identically here.

---

## Step 2 — `crates/db/src/ranking.rs` (DONE, except `rank_works`)

**Files:** `crates/db/src/ranking.rs` (new), `crates/db/src/lib.rs` (`pub mod ranking;`)

Delivered: `SlotKind` (+ `as_str`), `Impression`, `Ranked`, `log_impression`,
`InteractionKind` (+ `as_str`, + `for_source`), `InteractionRow`,
`record_interaction`, `scout_value`.

**`log_impression` takes a `slot_id`, not a reader.** It `UPDATE`s the two new
columns on the `recommendation_slots` row that §33.3 already wrote, rather than
inserting into a table of its own:

```rust
let sqlite  = "UPDATE recommendation_slots SET slot_kind = ?, propensity = ? WHERE id = ?";
let postgres = "UPDATE recommendation_slots SET slot_kind = $1, propensity = $2 WHERE id = $3";
// ...
let sql = sql_owned(db, sqlite.to_owned(), postgres.to_owned());
```

**`InteractionRow` is an enum, not a `&str` key.** The two tables do not share a
primary-key shape (`work_view_log` is (work_id, viewer_hash, viewed_at);
`work_kudos` is (work_id, account_id)). A single key string invites binding the
same value into three unrelated columns, which compiles, runs, and updates zero
rows.

**`scout_value` filters NaN before clamping** — see correction 4 above. This is
the one place where the spec's formula was wrong and the test proved it.

**Verified:** `cargo test -p lorehaven-db --lib` → **82 passed**. The NaN fix is
shown red by removing the filter: `scout_value_is_clamped_at_both_ends` fails
with `a NaN obscurity must cost the engagement its credit, not mint one`.

---

## Step 3 — `record_interaction`: earned vs incentivized (M45-11) — **DONE in step 2**

Originally a separate step with its own `INSERT INTO interactions`. It turned out
to be an `UPDATE` against `work_view_log` / `work_kudos`, so it shipped with
`InteractionKind` and `record_interaction` in step 2. The interesting part is the
**default direction**, which is the opposite of the column default in migration
0098 and on purpose:

```rust
#[must_use]
pub fn for_source(source: &str) -> Self {
    match source {
        "reading_club" | "topic_subscription" | "bounty" | "taste_notification" => {
            Self::Incentivized
        }
        _ => Self::Earned,
    }
}
```

`reading_club`, `topic_subscription`, `bounty` and `taste_notification` are
`Incentivized` **by definition** (§47.4) — the reader was routed there by the
incentive rather than by the ranking, which is the same condition, not a
statistical tendency.

Everything else defaults to `Earned`, and that default is deliberately the kind
that is *not* discounted: a new reading-club feature that nobody remembered to
add to this `match` would inflate ranking exactly as silently as if it had been
added. Making it a visible edit is the point. `an_unlisted_source_is_earned_and_
that_is_the_deliberate_default` asserts the default so it cannot be flipped
quietly.

---

## Step 4 — `rank_works`: the pipeline (DONE)

**Files:** `crates/db/src/ranking.rs`

Signature as built:

```rust
pub async fn rank_works(
    db: &Database,
    account: &str,
    candidates: Vec<WorkId>,
    dimensions_for: &dyn Fn(&WorkId) -> Vec<String>,
    options: &RankOptions,
) -> Result<RankedOutcome>
```

`dimensions_for` is a **parameter, not a query inside the function** — that keeps
the ranking logic pure and testable, and keeps candidate *eligibility* with the
caller that owns the trust decision (§30.7). §47.2's deliberate split.

`RankOptions::default()` has **every mechanism off and `lambda = 1.0`**. §47.9
requires two calls with the same state to return identical ordering when
exploration is off, and that has to be the *default* or the acceptance criterion
tests a mode nobody uses.

`RankedOutcome` returns `{ ranked, exploration, floor, impressions }` plus
`all_impressions()`. §47.8's invariant is checkable against that without
re-deriving which stage produced which row.

### The three things that were wrong in the first draft

1. **`ranked_propensity` had to offset by the minimum.** Arena weights are signed
   (Plackett-Luce can go negative), so a raw sum can be zero or negative and the
   shares become meaningless or negative. Now shifted by the floor score, which
   changes the *distribution* between candidates but not their order.

2. **Identical scores give `1/n`, not `1.0` each.** `1.0` would tell the offline
   estimator there was no selection at all, which is a different claim about the
   world.

3. **MMR blended two incommensurable scales.** Relevance is an absolute weight in
   [0,1]; novelty is a ratio over the current candidate set. Blending them raw at
   `lambda = 0.5` promoted a **0.20-scored fluff work to first place** — not
   because the reader had seen three angst fics, but because `1.0 > 0.20`. Fixed
   by normalising relevance against the best candidate (`score / best`) and
   bounding the diversity term at `NOVELTY_FLOOR = 0.25`.

   This is caught by `three_angst_fics_in_a_row_nudge_the_fourth_toward_fluff`,
   which is §47.6's own worked example written as an assertion. The test failed
   on first run and the fix went into the blend, not into the test.

### Two deliberate non-defensive choices

- **`total_cmp`, never `partial_cmp(..).unwrap()`.** A caller handing in a NaN
  score must not turn a ranking into a panic; `mmr_survives_a_non_finite_score_
  without_panicking` covers it.
- **Exploration randomness is `Uuid::new_v4()`, not a seeded generator.** A
  *reproducible* random slot is a fixed slot. §47.3 wants an unmeasurable draw
  whose probability is known — and the probability, not the draw, is what offline
  evaluation consumes. The pool shrinks per draw, so consecutive slots are
  independent rather than repeats, and each logs `1/|pool_at_that_draw|`.

**Verified:** `cargo test -p lorehaven-db --lib` → **97 passed**, no clippy
warnings in `ranking.rs`.

---

## Step 5 — Scout value (M45-10)

**Files:** `crates/db/src/ranking.rs`

```rust
/// §47.7. Credit for engaging with a work that later earns a high rating,
/// weighted by how obscure it was at the time of engagement.
#[must_use]
pub fn scout_value(obscurity_at_read: f64, later_rating: f64) -> f64 {
    obscurity_at_read.clamp(0.0, 1.0) * later_rating.clamp(0.0, 1.0)
}
```

Keep this a **pure function**. It is a credit mechanism, not a ranking input
(§47.7): a reader's recommendations do not improve because they scouted well,
and wiring it into ranking closes a loop where being scouted raises reach, which
raises the score that pays the scout. If a later step wants it in ranking, that
is a spec amendment, not a code change.

**Verify:** a table-driven unit test over the corners (0.0, 1.0, negatives,
`f64::NAN`) plus the §47.9 acceptance clause: value unchanged when computed at
engagement time versus later.

---

## Step 6 — the suite

**Files:** `crates/app/tests/m45_ranking.rs`

Every test runs on **both** engines via `TestDb::connect_with_dir`. One file, one
flag:

```sh
cargo test -p lorehaven-app --test m45_ranking
LOREHAVEN_TEST_PG_URL='postgres://postgres:***@127.0.0.1:5432/postgres' \
  cargo test -p lorehaven-app --test m45_ranking
```

Required cases — each is a §47.9 acceptance clause, and each must be shown to
**fail** when the property it asserts is broken:

| # | Asserts |
|---|---|
| 1 | Every returned row has an `impressions` row with `propensity > 0` |
| 2 | `exploration_slots == 0` ⇒ two calls return byte-identical order |
| 3 | An `incentivized` interaction raises credit-ledger total and changes no ranking signal |
| 4 | Three consecutive picks sharing a tag change the fourth's diversity term; `lambda = 1.0` makes ordering identical to no-MMR |
| 5 | Scout value stored at engagement time is unchanged when recomputed after the work becomes popular |
| 6 | Exposure floor gives a zero-impression work impressions without promoting it past the trust bar or past a reader's content filter |
| 7 | Output is a permutation of the filtered candidate set — no silent drops |

**Injection gate.** Prove cases 1, 3 and 5 by breaking them: delete the
`propensity` write and confirm case 1 fails; make `for_source` always return
`Earned` and confirm case 3 fails; recompute `obscurity_at_read` from the present
and confirm case 5 fails. A test that has never been seen red is not evidence.

---

## Step 7 — wire the route, then the ledger

**Files:** `crates/app/src/routes/discovery.rs` (additive only)

> **Corrected 2026-10-01, before the edit.** This step previously said to replace
> `ORDER BY w.updated_at DESC`. **There is no such clause in this route.** The
> ordering is `r.sort_by_key(|c| -c.score)` at `discovery.rs:347`, and `score` is
> not a taste signal — every engine sets it to `(limit - idx)`, i.e. it just
> restates each engine's own position. So the real finding is stronger than the
> plan assumed: discovery has **no taste-based ordering at all**, which is
> consistent with `personalized_recommendations` ordering by `w.updated_at DESC`
> *inside* `crates/db/src/discovery.rs`. Fix the wrong clause reference rather
> than code to it.

What step 7 actually is:

1. Take the blended candidate ids the route already computes, and reorder them
   with `rank_works` instead of by `score`. **Leave candidate selection alone** —
   §47.2 splits eligibility (a trust question, §30.7) from ordering (a taste
   question), and the existing engine blend is the selector.
2. The route already has a `dimensions_for` problem to solve: `rank_works` takes
   a `&dyn Fn(&WorkId) -> Vec<String>`, and the route has no tag list per work
   in hand. `lorehaven_db::taxonomy::tag_names_for_work` exists and is already
   called at `discovery.rs:369` for theme gravity — reuse that shape rather than
   inventing a second tag fetch.
3. **Order of operations matters.** Half-life (`discovery.rs:318`), operator
   affinity (`:342`) and theme gravity (`:353`) all reorder `blended` *after* the
   blend. `rank_works` must run where those operators can still be honoured, or
   installing it will silently drop all three. Decide and record that, do not
   discover it by diffing output.
4. `rank_works` is **only meaningful for a signed-in reader** — it loads
   `TagWeights::for_reader(db, account)`. Anonymous discovery must keep its
   current path, and that is a branch, not a detail.
5. Log the impressions. `RankedOutcome::all_impressions()` (ranking.rs:443) is the
   one call that satisfies §47.8, and **nothing logs them yet** — that is why
   M45-49 is `implemented-locally-tested` and why the extension's impression story
   in §48 is blocked. This is the step that unblocks both.

Then update `docs/requirements.csv`: M45-13, M45-11, M45-15, M45-49, M45-10 →
`implemented-fully-tested`, with the test names in `evidence`, in the same commit
as the code.

**A test that must go red first.** The route's ordering is currently
`sort_by_key(|c| -c.score)`, so pinning the order to `score` in a test is
vacuous — it is what the code already does. The test worth writing is one that
gives two works *opposite* engine positions and the *same* score-derived
ordering, then asserts the taste profile decides. Without that, step 7 can be
"done" by a test that proves nothing.

**Do not claim M45-10 (`scout_value`) from this step** unless the route actually
reads it. `scout_value` is a curation-credit computation, not a feed ordering; if
step 7 does not call it, M45-10 stays `implemented-locally-tested`.

## Gate results (2026-09-30, re-verified after two test-isolation fixes)

| Engine | Result |
|---|---|
| SQLite | **3426 passed, 0 failed**, exit 0, `--test-threads=4` |
| PostgreSQL | **3426 passed, 0 failed**, exit 0, `--test-threads=2` |
| `cargo fmt --all --check` | clean |
| `cargo clippy` | no new warnings in the changed files |
| doctests | 0 in `lorehaven_app` (unchanged; the section reports `running 0 tests`) |

Three failure modes were separated out during verification, and none of them was
a defect in the ranking code. All three name something other than their cause,
which is what made them expensive.

**1. `arena_weights` NOT NULL — real, and only the full gate saw it.**
`the_readers_weights_decide_the_order` passed alone and failed in the workspace
run. The fixture omitted `id` and `updated_at`, which `0066_taste_arena.sql:19`
declares NOT NULL with no default. The insert is guarded by `WHERE NOT EXISTS`, so
an earlier test in the same file satisfied the guard and the malformed statement
never ran. Fixed; 10/10 on both engines.

**2. Two test-isolation bugs, each a different mechanism, both intermittent.**

- `vote_decay_score.rs` built its scratch path from
  `SystemTime::now().as_nanos()`. That is a clock *reading*: it does not advance
  between two calls the scheduler runs back to back on one thread, so **eight
  tests produced five directories** and four of them migrated the same file.
  Symptom: `table accounts already exists`, which names a migration. Fix: a
  monotonic `AtomicU64` counter plus `process::id()`.
- `preservation_recheck_wiring.rs` called its scratch helper **twice** per test
  with the same tag — once for `connect_with_dir`, once from `state_for` — and the
  helper began with `remove_dir_all`, so the second call unlinked the live
  database. Symptom: `no such table: jobs`. Fix: remove the directory at most once
  per (process, tag).

Both present in the tree while an earlier gate reported green. A gate result is
true when measured; it does not certify the next run.

**3. `E0463: can't find crate` from rustdoc — a rebuild race, not a missing
crate.** A gate exited 101 with every test passing and 17+6 doctest link errors.
All 43 `--extern` rlibs existed on disk with valid `!<arch>` headers, and the
failing rlib's mtime fell *inside* the run. A concurrent `cargo build` was
regenerating them. `cargo build -p lorehaven-app --lib` first, then the doctest,
passes. Check the `--extern` paths before believing the message:

```sh
python3 - <<'PY'
import re, os
t = open("gate.log", errors="replace").read()
paths = dict(re.findall(r'--extern (\w+)=(\S+)', t))
print(len(paths), "externs,", [n for n,p in paths.items() if not os.path.isfile(p)], "missing")
PY
```

**Pool exhaustion is still environmental.** At `--test-threads=4` across the
workspace, binaries compete for SQLite connections and tests fail with
`pool timed out while waiting for an open connection`. Those pass serially.

**Leaked `lh_test_*` PostgreSQL databases still cascade.** An interrupted run
leaves one per unfinished test and the sweeper skips any with a live backend. 1928
of them once turned a gate into 58 `pg_database_datname_index` failures. Sweep
before a gate:

```sh
psql -h 127.0.0.1 -U postgres -tAc \
  "select datname from pg_database where datname like 'lh\\_test\\_%'" \
  | xargs -P 16 -n 1 dropdb -h 127.0.0.1 -U postgres --if-exists --force
```

(`DROP DATABASE` cannot run inside a PL/pgSQL function, so the `DO $$` batch form
does not work — `dropdb` in parallel does. 1928 databases: 6m20s.)

## Definition of done

- [x] `migrations/{sqlite,postgres}/0098_ranking.sql` exist and parity passes
- [x] `cargo test -p lorehaven-app --test m45_ranking` green on **both** engines
- [x] Cases 1, 3, 5 and the determinism case shown red by injection
- [x] `cargo fmt --all --check` clean
- [x] `cargo clippy` introduces no new warnings in the changed files
- [x] Full workspace suite green on both engines (see Gate results)
- [x] The five M45 rows updated in `requirements.csv` with evidence
- [x] `docs/goal.md` counts re-derived, not remembered
- [x] **Step 7 — `GET /api/v1/discovery` calls `rank_works` and logs every served
      row** (`84c8216`, `5d22356`). M45-11, -13 and -49 are now
      `implemented-fully-tested`. M45-10 (`scout_value`) and M45-15 (exposure
      floor) stay `implemented-locally-tested` on purpose: the route does not
      call them, and a row that claims otherwise is the thing §"What complete
      means" in `docs/goal.md` exists to prevent.

### What wiring the route found

Two defects in `rank_works` that no test in its own module could see, because
every one of them either used `variety: true` or had equal scores:

1. `candidates.sort()` before scoring made the output a function of the UUIDs.
   Whenever taste could not separate two candidates, the caller's engine ranking
   was replaced by lexicographic order — and a uuid-ordered list looks exactly
   like a ranked one until you check which way round it is.
2. With `variety: false` the ordering was `scored.iter()` — input order, never
   sorted by score. So the "relevance only" path was a pass-through and
   `rank_works` only ranked by taste when MMR happened to be switched on.

(2) was hidden by (1). Both are fixed, with the reasoning in the commits and a
unit test each — the second of which was verified red before green.

A third, in the same class and found the same way: `RankOptions::default()` has
`lambda: 1.0`, and `mmr_rerank` short-circuits at `lambda >= 1.0`. So
`variety: true` with the default lambda is a **no-op** — taste computed, paid
for, and discarded. The route now sets `lambda: 0.7` explicitly. A mechanism that
looks configured and is not is its own failure mode, and there are now three.

### The conflict: determinism versus the caller's ranking

Fixing (1) turned `two_calls_on_the_same_state_agree_exactly` red, and that test
was not merely asserting the bug — it was **passing because of** it. It asserts
order-insensitivity (the same candidates reversed must rank identically), and
sorting by id before scoring is exactly what delivers order-insensitivity.

The two requirements are mutually exclusive:

| ties resolved by | order-insensitive? | caller's ranking honoured? |
|---|---|---|
| `WorkId` | yes | **no** — replaced for anything taste cannot separate |
| arrival position | no | yes, but the result depends on how the list was built |

§47.2 decides it: *"given the same database state and the same reader, the
ordered result is byte-identical except for the exploration slots."*
Order-insensitivity is the requirement, so the caller's ranking has to survive
*inside* it.

**Resolution: sort by id LAST, not first.** Score descending, then `WorkId`
ascending. Only candidates that are otherwise equal are reordered, so every
score-resolved pair stays where the ranking put it — while the output remains
independent of arrival order.

That yields a matched pair of tests, each verified red against its own half:

- tie-break removed → `two_calls_on_the_same_state_agree_exactly` fails
- dedup reverted to `sort()+dedup()` → `with_no_weights_the_feed_stays_in_engine_order` fails

Neither passes because of the other. That pairing is the deliverable: one test
proves the property, the other proves the property did not cost the caller
something it needed. A single test could not have caught this, because the bug
and the requirement it was accidentally satisfying are the same code.

### The flake had a second source

`blend` collected a `HashMap` with `into_values()` and then `sort_by_key`. A
stable sort resolves ties by **the order it inherited**, and HashMap iteration
order is randomised per process by SipHash — so the route's ranker received a
genuinely different list on every run. Fixed by sorting on
first-appearance-across-engines before the score sort, so the tie order is a
property of the input rather than of the hash seed. Every existing blend test
passed with this in place, because each used candidates whose scores all differed.

**The lesson worth keeping:** when auditing a "stable" sort, ask what the
stability is relative to, and whether the input it inherited was itself
deterministic.

### Gate at `e9ea490`

| backend | passed | failed | suites |
|---|---|---|---|
| SQLite | 3432 | 0 | 159 |
| PostgreSQL | 3432 | 0 | 159 |

Up from 3425 before step 7. Zero compiler warnings. `m29_transparency` was also
run six consecutive times green to confirm the per-process tie-break is gone: one
green run is not evidence of determinism.

**On the gate's own reliability.** A run at the default thread count failed three
unrelated suites (`crowdfunded_bounty_contribution_activates`,
`standard_bounty_creation_succeeds`, `an_absent_audience_leaves_a_work_readable`)
with `pool timed out while waiting for an open connection`. That is
connection-pool exhaustion from running every test binary at full parallelism,
not a product defect. It is worth naming because "three tests failed" reads as a
regression, when the actual finding is that **the gate is only trustworthy at a
bounded thread count**: `--test-threads=4` on SQLite, `2` on PostgreSQL. Check
the error text before investigating the code.

## Not in this plan

- M45-14 (work coordinates / stylometry), M45-16 (tag contribution cap),
  M45-17 (taste leakage), M45-19 (tasting menu), M45-23 (north-star metric),
  M45-31/32 (fandom-blind discovery, trend radar) — these *read* the substrate
  and depend on step 4 landing. Separate plans.
- M45-12 (generated-content posture) needs a spec clause of its own; §47.4
  explicitly refuses to let a detector reclassify an interaction, and that
  refusal is the interesting part of the row.
- M45-45/46/48/50 (admin: versioned taste, view-as-persona, hardware presets,
  scheduled gravity) — §47.10 gives operator weight config a single owner to
  avoid two places setting a weight.
