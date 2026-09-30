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
> Steps 1–3 and 5 are implemented and committed. Step 4 (`rank_works`) and step 6
> (the suite) are not.

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

## Step 4 — `rank_works`: the pipeline

**Files:** `crates/db/src/ranking.rs`

Per §47.2 the stage order is fixed and a stage may not reorder it. Signature:

```rust
pub struct RankOptions {
    /// Uniform-random exploration slots (M45-13). Default 0.
    pub exploration_slots: usize,
    /// Guarantee impressions for zero-impression works (M45-15).
    pub exposure_floor: bool,
    /// MMR diversity weight in [0,1]; 1.0 = relevance only (M45-49).
    pub lambda: f64,
    /// Whether MMR/satiation runs at all.
    pub variety: bool,
}

pub async fn rank_works(
    db: &Database,
    account: &str,
    candidates: Vec<WorkId>,
    options: &RankOptions,
) -> Result<Vec<Ranked>>
```

Implementation notes, each of which is an acceptance criterion in disguise:

- **Determinism.** Sort candidates by id *before* scoring. Two calls with the
  same database state must return identical ordering when
  `exploration_slots == 0` (§47.9). An SQL query with no `ORDER BY` will not
  give you this for free — the test asserts it and the test will find out.
- **Propensity for a ranked row** is the reader's normalised score share:
  `score_i / Σ scores` (1.0 when the set has one row). Document the choice.
- **Exploration slots** draw uniformly from the eligible-but-unshown set, with
  the pool reduced after each draw so consecutive slots are independent (§47.3).
- **Satiation** reads the reader's recent picks' tags with a time decay, and
  raises *scores* — it must never remove a candidate. A hard exclusion would be
  a filter, and §43.3 forbids filters.
- **MMR re-ranks the top of the list** and does not drop rows.

**Verify:**
```sh
cargo test -p lorehaven-app --test m45_ranking
```
Expected: the determinism, permutation, propensity and satiation tests pass.

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

Call `rank_works` from the existing personalized path, replacing the
`ORDER BY w.updated_at DESC` *ordering* only. **Leave the candidate selection
alone** — §47.2 splits eligibility (a trust question, §30.7) from ordering (a
taste question). Replacing the `w.updated_at DESC` clause is the whole change.

Then update `docs/requirements.csv`: M45-13, M45-11, M45-15, M45-49, M45-10 →
`implemented-fully-tested`, with the test names in `evidence`, in the same commit
as the code.

## Definition of done

- [ ] `migrations/{sqlite,postgres}/0098_ranking.sql` exist and parity passes
- [ ] `cargo test -p lorehaven-app --test m45_ranking` green on **both** engines
- [ ] All 7 cases above green, each shown red by injection
- [ ] `cargo fmt --all --check` clean
- [ ] `cargo clippy -p lorehaven-db -p lorehaven-app` introduces no new warnings
- [ ] Full workspace suite green on both engines
- [ ] The five M45 rows updated in `requirements.csv` with evidence
- [ ] `docs/goal.md` counts re-derived, not remembered

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
