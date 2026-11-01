# M45-25 — Keystone authors, droughts, and the routing record

**Row:** `M45-25` (community) — "Keystone author program: drought detection, feedback routing,
beta matching". `Gaps review C1. Opt-in credit patronage; never touches ranking.`

**Plan position:** `docs/plans/m45-gaps-adoption.md` Phase 3 (`M45-25, -26, -28, -29`), gate
"supply routing works from real taste". Phase 2 closed (`M45-14/24/31/35` all
`implemented-*-tested`), so this is the next row in the plan's own order — not the CSV's id
order, which the plan explicitly overrides ("the order is dependency order, and breaking it
costs real work").

---

## What the premise measured, before any design

Measured on the live instance, 2026-11-01 (`psql "postgresql:///darkhold?host=/run/postgresql"`):

| fact | value | what it decides |
|---|---|---|
| `comments` | **0 rows** | a *drought* (engagement without conversation) cannot be told apart from the whole corpus: every engaged work has zero conversation |
| `work_ratings` | **0 rows** | the second conversation signal is empty too |
| `instance_taste_settings` | **0 rows** | no operator-tuned weights on this instance |
| `admin_taste_profile` | **0 rows** | no operator taste targets, so a taste term is 0.0 today — and must be visibly 0.0, not silently absent |
| `credit_transactions` | 1,115 rows | the ledger is live, so patronage has somewhere to land |
| engaged rows (`likes > 0`) | 1,247 across 920 authors; 202 with ≥ 2 works, 59 with ≥ 3, max 14 | **keystone detection is computable today** — this half of the row is not blocked |

So the row splits along what the data can support:

- **keystone detection** — computable from endorsement data now, and improves when an operator
  configures taste (`admin_taste_profile` is read, and empty means a 0.0 bonus rather than a
  missing term).
- **drought detection** — the predicate is written and tested, but on *this* catalogue it refuses
  by name (`measured: false`) instead of listing 1,247 works as "in drought", which is what a
  naive count would do. The first comment on the site makes it real.
- **beta matching** — **not this row.** `M45-33` owns "beta and specialist matchmaking"; this row
  emits the keystone list that row matches against. Recorded as a deferral with its reason.

## Definitions (spec §58, written this turn)

**Keystone author.** An author (key = `lower(trim(author_text))`) with at least `min_works`
engaged works (default 2) whose **mean within-source percentile** is at least `min_pct`
(default 0.75).

The percentile is M63-03's rule, unchanged: rank *among the engaged rows of the author's own
source*, `pct_raw = (rank - 0.5) / n`, then shrink toward 0.5 by `n / (n + 10)` so a four-work
source cannot outrank a 400-work one on the same raw percentile. A mean of these is comparable
across sources, which a mean of raw likes is not.

**Taste bonus.** `sum(weight)` over `admin_taste_profile` rows whose `dimension_key` is
`author:<key>`, clamped to `[-0.25, +0.25]` and floored at 0 — the operator's taste may raise a
keystone, never lower one, and never enters a *ranking*. `score = mean_pct + bonus`.

**Drought.** A work with endorsement and no conversation. Conversation = `comments` on the work
plus `work_ratings` for it. **When the corpus has no conversation at all the report is
`measured: false` with a reason and an empty author list** — the refusal is the deliverable, the
same shape `?sort=velocity` uses for a missing second snapshot.

**Routing.** A reader opts in and receives one work from one keystone author: the author with the
highest `score` that has a work this reader has not been routed before, the least-routed work
first, then the most liked. The record is the routing row; completion pays
`PATRONAGE_BP = 500` (5.00 credits) through the existing ledger with idempotency key
`supply-route:<id>`, so a repeated completion pays once.

**Invariants.** Supply never touches ranking (§12 positivity); no public per-work drought count;
reruns replace the keystone snapshot rather than appending; a routing row is never deleted.

---

## Files

| file | change |
|---|---|
| `docs/spec.md` | **new §58** — Supply: keystone authors, droughts, and the routing record |
| `migrations/{sqlite,postgres}/0133_supply_keystones.sql` | `supply_keystones`, `supply_routing` |
| `crates/db/src/supply.rs` | new: `recompute`, `keystones`, `drought`, `route`, `complete`, `routes_for` |
| `crates/db/src/lib.rs` | `pub mod supply;` |
| `crates/app/src/routes/supply.rs` | new: 5 routes (2 operator, 3 reader, 1 operator recompute) |
| `crates/app/src/routes/mod.rs`, `crates/app/src/server.rs` | register the router |
| `crates/cli/src/supply.rs`, `crates/cli/src/main.rs` | `dh supply keystones\|droughts\|recompute\|route\|complete` |
| `crates/db/tests/supply_keystones.rs` | 7 tests, SQLite + optional `DARKHOLD_TEST_PG_URL` |
| `crates/app/tests/m45_25_supply.rs` | 6 route tests |

## Steps, each with its verification

1. **Spec §58.** `grep -n "^## 58" docs/spec.md` → one hit.
2. **Migration.** `cargo test -p darkhold-db --lib` applies it; then
   `dh migrate --status` on the host → `133 applied, 0 pending`.
3. **Store.** `cargo test -p darkhold-db --test supply_keystones` → `7 passed` on SQLite, then
   the same with `DARKHOLD_TEST_PG_URL` set → `7 passed`.
4. **Mutations** (each must be caught by the named test):
   - drop the shrinkage → `a_small_source_does_not_outrank_a_large_one`
   - `min_works` 2 → 1 → `one_work_is_not_a_keystone`
   - taste bonus unclamped negative → `operator_taste_raises_a_keystone_and_never_lowers_one`
   - `measured:false` replaced by an empty list → `an_empty_corpus_refuses_instead_of_listing_everything`
   - `not exists` exclusion removed from `route` → `a_reader_is_not_routed_the_same_work_twice`
   - idempotency key dropped on completion → `completing_twice_pays_once`
5. **Routes.** `cargo test -p darkhold-app --test m45_25_supply` → `6 passed`.
6. **Deploy** per `darkhold-thinkcentre-deploy`, then live proof:
   `curl -s 127.0.0.1:8081/api/v1/supply/keystones?limit=3` (operator token) and
   `dh supply droughts` → the refusal with its reason.

---

## What landed, and the five things that diverged

Migration **0133** (`supply_keystones`, `supply_routing`), `crates/db/src/supply.rs` (five
functions, three of them refusals), `crates/app/src/routes/supply.rs` (six doors),
`crates/cli/src/supply.rs` (`dh supply keystones|droughts|recompute|route|routing|complete`),
spec **§58**, and `docs/snapshot-column-policy.json`'s decision for `supply_routing.account_id`.

| # | what the plan said | what the build found |
|---|---|---|
| 1 | taste bonus "clamped to ±0.25 and floored at zero" | the table has `CHECK (weight >= 0.0)`, so a stored negative weight is impossible and the *floor* is the schema's, not the code's. §58.1 was rewritten to say which half is which; the test asserts both (a 0.4 weight lands at 0.25, an author with no weight gets 0). |
| 2 | `works_with_conversation` by counting distinct works | the first version counted **every** joined work, because `COUNT(DISTINCT li.id)` over a `LEFT JOIN` counts rows with no comment too. Caught by `conversation_turns_the_report_on` (3 where the answer is 1); the fix is `COUNT(DISTINCT CASE WHEN c.id IS NOT NULL THEN li.id END)`. |
| 3 | — | `library_items.likes` is `INTEGER` on PostgreSQL, so `get::<i64>` panicked with a `ColumnDecode`. Fixed with `CAST(likes AS BIGINT)` in both dialect arms; only the PostgreSQL arm could have caught it. |
| 4 | — | **`scripts/check-snapshot-pii.py` refused the new migration.** §11.16 requires a policy decision for every column that carries an identity, and `supply_routing.account_id` had none; `cargo test -p darkhold-app --lib` failed with "the gate check reported Fatal". The decision is `drop_table`, with the reason that the routing record exists to hold one reader's opt-in and its research value is derivable without it. |
| 5 | complete-once via the idempotency key | **there are two guards**, and either alone suffices: the `completed_at` early return and the `supply-route:<id>` key. Removing one leaves the test green, which is the point of having both — the mutation that fails is the one that removes both. Reported rather than dressed up as a single-guard test. |

Beta matching remains **M45-33**'s, as the plan said when it took this row.

### Live verification

`133 applied, 0 pending`; unit active; the operator doors and the reader doors are
credential-gated, so what is provable without an operator session is named in the session note
rather than claimed here.
