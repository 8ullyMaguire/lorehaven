# Lorehaven — what's left

Updated at the start of each turn. Last commit: `052559b` (gap E).

## Done this project

| Gap | Feature | Where | Tests |
|---|---|---|---|
| G | §20.3 author payout multipliers | `crates/domain/src/payouts.rs` | 12 |
| G | §20.3 payout store (signals → ledger) | `crates/db/src/payout_store.rs` | 14 |
| G | §20.3 weekly recalculation job | `crates/app/src/payout_recalc.rs` | 12 |
| D | Series-aware recs (first *unfinished* entry) | `crates/db/src/series_recs.rs` | 17 |
| E | Earned-bookmark ratio + §53.6 definition | `crates/domain/src/earned_bookmark.rs` | 13 |
| F | Hidden-classics rec strategy | `crates/db/src/rec_strategy.rs` | 11 |
| B | Blind Date daily surface | `crates/db/src/discovery.rs` | 8 |

Plus, as incidental fixes found by the above: migration 0110 FK divergence,
`hit_rate.rs` migrating the shared postgres database, a `kind_index` collision,
and a race in the test-support schema cache.

## Remaining ranked gaps (from docs/plans/100-ideas-audit.md)

All six of the audit's "real gaps" are now closed or nearly so. The audit's
remaining Tier 2 rows that were never gaps (EXISTS) need nothing.

- **Gap C** — AI pre-read scoring (#18). Named as the highest-value of the six.
  Needs an AI provider interface; §23.7 specifies task categories but not this
  one. Two AI tasks exist (`translation.rs`, `narration.rs`) but no shared
  provider trait. **Largest remaining piece, and the only one left of the six.**
- Gaps A, B, D, E, F, G — closed.

## Also outstanding

- 31 rows in the M45 tracker still marked `planned`.
- `docs/plans/100-ideas-audit.md` needs its gap F section marked CLOSED.
- The §0.3 "three TEXT/uuid columns" note should become four now
  (`work_view_log.work_id` in `rec_strategy.rs` is the fourth site).

## Immediate next steps

1. Both mutation gates green (F and B) — in flight.
2. clippy + fmt, commit F, B and `crates/domain/src/ai.rs` together.
3. Gap C step 2: `crates/domain/src/preread.rs` — aggregate per-dimension verdicts into a
   per-work report, with the abstain path. Still pure domain.
4. Then the adapters in `crates/app/src/ai/`, which need a live provider to verify.

`crates/domain/src/ai.rs` is **done**: `AiTask`, `CostQuote`, `PreReadVerdict` (with
range validation), `AiAbstain` (retryable vs terminal), the `AiProvider` trait, and
`AiConsent`. 7 unit tests green, clippy clean. One decision worth noting: the trait uses
**boxed futures rather than `async fn`**, because a native `async fn` in a trait cannot
state `Send` on the returned future — so a provider holding a `reqwest::Client` could not
be awaited in a spawned worker task, which is where every caller belongs.

Gap C's design is settled in `docs/plans/gap-c-ai-pre-read-scoring.md` — six decisions
recorded, and the two spec prohibitions (§32.6 no public composite scores, §0.3 no
payment moving a ranking signal) that shape all of them.

## Lessons from the mutation gates (cost real time, keep these)

- **A test that passes for the wrong reason is a GREEN(BAD) with extra steps.**
  `an_unpublished_work_is_never_offered` seeded a draft with no completions, so
  `completions > 0` excluded it and the lifecycle clause was never exercised.
  Fixtures must clear every *other* gate, or the gate under test is untested.
- **Presence assertions don't kill ordering mutations.** The log-vs-linear mutation
  survived because the test only asserted both works appear; a linear denominator
  also returns both. Assert the ORDER, with a pair the two denominators disagree on.
- **Solve for the comparator.** Guessed comparators hold under both the real query
  and the mutation. Search for a pair where correct and mutated land on opposite
  sides, then use it.
- **A compile error is only a valid RED if the mutation was meant to compile.**
  Removing a `format!` placeholder makes the build fail — the tests never ran. Keep
  such mutations balanced (`OR 1 = 0`) so they genuinely change behaviour.
- **Scope mutation patterns.** `AND w.lifecycle = 'published'` matches the first
  sibling strategy in the file, not the one under test, so the result is meaningless.
- **Neither engine has a portable SQL function**: no `md5` on SQLite, no
  `LOG10`/`LN` without SQLITE_ENABLE_MATH_FUNCTIONS, `::text` casts only on PG. Do
  hashing in Rust. This is now the rule for every store in this codebase.
- **To disable `AND x NOT IN (SELECT … WHERE <pred>)`, add `AND 1 = 0` to the predicate.**
  `AND (x NOT IN (…) OR 1 = 0)` is a *no-op* — `X OR false` is `X` — and reports a false
  survivor. `1 = 0` empties the subquery, which is what actually disables it, and keeps
  the `format!` placeholder used so the build still succeeds.
- **With healthy works also in the catalogue, "an ineligible work loses" is luck, not
  logic.** Make the ineligible work the *only* candidate so any leak is a guaranteed pick.
  This bit THREE separate blind-date mutations (bookmark, visibility, published_at): an
  ineligible work competing with 25 healthy ones loses about 24 days in 25 by chance.
- **A mutation that orphans a function is a compile error, not a RED.** Replacing
  `min_by_key(f(id))` with `min_by_key(id.clone())` leaves `f` uncalled, and warnings are
  denied in this repo, so the build fails and the test never runs. Mutate *inside* the
  function (drop the seed from the hash) instead of removing its only call site.
- **`works.id` is uuid on PG and TEXT on SQLite, while `subject_id` /
  `work_view_log.work_id` are TEXT on both** — the fourth, fifth and sixth sites.

## Working notes (learned the hard way, keep applying)

- **Mutation testing finds survivors every time.** A surviving mutation means a
  fixture that ONLY that condition can break. Add the fixture, don't argue.
- Two dialect arms drift. Prefer named `format!` args over positional `{}`.
- `crates/db/src/rec_strategy.rs` siblings use `julianday('now')`, which is
  SQLite-only, so those strategies have never run on PostgreSQL. Not a task, but
  do not copy that pattern.
- A surviving mutation can mean the *code* was wrong, not the tests (gap E's
  `usable()`).