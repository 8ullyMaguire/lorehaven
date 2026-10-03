# Lorehaven — what's left

Updated at the start of each turn. Last commit: `2c2849e`; merge `0161c14`.

**Delegation is unavailable in this profile.** A subagent dispatched for step 3 died in
0.59 s with `HTTP 400: Unable to determine provider for model 'qwen2.5-coder:3b-64k'`. Build
in the main session.

**lorehaven develops on gaming-pc only**, in `~/code-local/rust/lorehaven`. That was true
on 2026-10-02 and stopped being true on 2026-10-03: `~/code/rust/lorehaven` is a bind/sshfs
mount of thinkcentre's own checkout, not a gaming-pc copy. The two diverged at M47 and
have now been merged (see below). thinkcentre is being turned into a plain mirror — one
development site per repo, which is what stops this recurring.

**`git diff` is intercepted here.** Two invocations returned `No syntactic changes` instead of
a diff for a file that genuinely had one. Use `diff <(git show HEAD:f) f` to see a real diff.

## Done this project

| Gap | Feature | Where | Tests |
|---|---|---|---|
| G | §20.3 author payout multipliers | `crates/domain/src/payouts.rs` | 12 |
| G | §20.3 payout store (signals → ledger) | `crates/db/src/payout_store.rs` | 14 |
| G | §20.3 weekly recalculation job | `crates/app/src/payout_recalc.rs` | 12 |
| D | Series-aware recs (first *unfinished* entry) | `crates/db/src/series_recs.rs` | 17 |
| E | Earned-bookmark ratio + §53.6 definition | `crates/domain/src/earned_bookmark.rs` | 13 |
| F | Hidden-classics rec strategy | `crates/db/src/rec_strategy.rs` | 12 |
| B | Blind Date daily surface | `crates/db/src/discovery.rs` | 12 |
| C | §23.7 provider interface | `crates/domain/src/ai.rs` | 7 |
| C | Per-dimension pre-read report | `crates/domain/src/preread.rs` | 8 |
| C | OpenAI-compatible adapter | `crates/app/src/ai/` | 18 |
| C | Report persistence (migration 0111) | `crates/db/src/preread_store.rs` | 12 |
| C | Author-only pre-read route (§32.6) | `crates/app/src/routes/preread.rs` | 10 |
| C | Author-only panel, editor-only | `frontend/src/lib/components/PreReadPanel.svelte` | 6 |

Plus, as incidental fixes found by the above: migration 0110 FK divergence,
`hit_rate.rs` migrating the shared postgres database, a `kind_index` collision,
a race in the test-support schema cache, a `CostQuote::is_priced()` that reported an
unpriced local-model quote as priced, a `TestDb::applied_migrations()` that returned an
empty set for a fully-migrated database (8 suites), and a Blind Date seed that ignored the
day so every reader got the same work forever.

## Remaining ranked gaps (from docs/plans/100-ideas-audit.md)

All six of the audit's "real gaps" are now closed — A, B, C, D, E, F, G. Gap C took the
longest and is described below.

## Gap C is closed

All six steps are done and tested on both engines. What it consists of:

| Step | What |
|---|---|
| 1 | `AiProvider` trait, `AiAbstain`, `CostQuote`, `AiConsent` |
| 2 | `PreReadReport` — per-dimension, with the abstain path |
| 3 | The OpenAI-compatible adapter (also covers Ollama) |
| 4 | Persistence, migration 0111 |
| 5 | Three author-only routes |
| 6 | The editor panel — **a component, not a route** |

The step 6 decision worth keeping: there is no `/preread` path and no route id, so there
is nothing to link to or share. §32.6 says the report is never on the public work page, and
the cheapest way to guarantee that is for the surface not to exist as an address. The panel
renders inside `WorkEditor`, which already branches on ownership, and a 404 makes it render
nothing at all rather than an error message — a "not found" on somebody else's work would
confirm the draft exists, which is exactly what the server's indistinguishable 404 exists
to prevent.

**All six gaps (A–G) are now closed.**

## The thinkcentre merge (done, `0161c14`)

Two lineages diverged at `921a59d` (M47). Merged into master, not rebased — both sides
were published, so merging loses nothing and resetting either one loses a lot.

Carried over that master did **not** have: the taste arena and taste vector were
production *stubs* on master (`dimension_score` hashed `work_id`+dimension into a
pseudo-score, `get_admin_centroid` returned a hardcoded `vec![0.5;5]`, and
`fetch_user_rated_work_vectors` returned nothing) — so every taste vector was empty and
the recommender learned nothing from any rating, and every arena ballot was a no-op.
Also `rec_strategy.rs` (516 lines, 8 strategies + RRF), the pawchive/chyoa scraper
repairs, `dnf.rs`, `routes/external.rs`, migrations 0072/0073, 5 test files.

The plan predicted 2 conflicting files and named the wrong ones. Actual: 4 conflict
blocks over 3 files, none of which the plan named. The lesson is in "Lessons" below.

## M45-18 — faucet/sink dashboard (done)

Plan: `docs/plans/m45-18-faucet-sink-dashboard.md`.

| Step | State |
|---|---|
| 1. Registry, migration 0112 | **done** (`bca9055`) — 112/112 migrations apply on both engines, four seed rows verified identical on SQLite and PostgreSQL |
| 2. `crates/db/src/flow_store.rs` | **done** (`23546d0`) — compiles clean, clippy clean |
| 2a. `Flow::Undeclared` in the domain | **done** (`23546d0`) — see below |
| 3. `crates/app/src/routes/flows.rs` | **done** (`430ceaf`) — `GET /admin/economy/flows`, operator-only, 404-not-403 |
| 4. `crates/app/tests/flow_dashboard.rs` | **done** (`430ceaf`) — 6 cases green on both engines |

Two stale-batch items turned out to be real and are now fixed:

- **`registered_routes_are_tabled` failed on a clean tree** (`ee4b78b`). Four routes were
  registered but absent from `ROUTE_TABLE`: `discovery.rs:/discovery/blind-date`
  (Authenticated) and all three of `preread.rs`'s routes (Pseudonymous). The test stops at the
  first unregistered route, so the error named one and hid three more — searching the table
  for `preread.rs` directly is what found them. Proven: deleting an entry turns it red.
- **A `rec_strategy.rs` doctest had been failing for some time** (`9d0e0cd`). The score
  formula was a four-space-indented doc block, which rustdoc compiles as Rust. The errors
  (`cannot find value completion_rate`, `cannot find function log10`) read like undefined
  variables in the strategy, which is what sent me looking for a code bug that was not there.
  A `\`\`text\`\`` fence fixes it. `cargo test --workspace --doc` is now clean — and it is
  a separate target a plain `cargo test -p <crate>` never runs.
| 5. Frontend | **done** — `AdminEconomyFlows.svelte` + 7 tests, `fetchEconomyFlows` in `api.ts` |

**`Flow` gained a fourth variant, `Undeclared`,** and this was not in the plan. The plan
said a registry miss becomes `MechanismDeclaration::invalid()`. That would have been wrong:
`FlowSummary::compose` counts undeclared via `!declaration.is_valid()`, and
`MechanismDeclaration::neutral()` is *valid* — so every unclassified mechanism would have
rendered as a settled `neutral` with `undeclared == 0`. That is exactly the failure §53.1
forbids: the dashboard reporting a smaller economy than exists while looking deliberate.
`Flow::Undeclared` is the absence of a declaration, distinct from `Flow::Neutral`, which is a
claim that there is no side. Three new domain tests cover it, and the load-bearing one is
proven: reverting the `is_declared()` clause in `compose` turns
`an_undeclared_mechanism_counts_without_being_netted_to_a_side` red.

Two facts that are now recorded in the plan and would have shipped bugs:
`amount_bp` holds **whole credits**, not basis points; and `preservation_dues` /
`preservation_reclaim` share the literal `reference` `{member_id}` on **opposite** sides,
so the registry is keyed on a mechanism name derived from `TxnType`, not on the raw
reference.

## In flight

Nothing. The two full-suite runs that were in flight here completed and were
superseded by per-suite verification on both engines; the tree is published.

**Run suites serially and do not touch the tree while they run.** Four bad results in an
earlier session came from doing otherwise.

## Also outstanding

- **M45-57, curator-submitted source adapters (§55) — Path A built, not deployed.**
  `Category`/`Manifest` types, the declarative manifest schema, and
  `DeclarativeAdapter` are in and tested (340 scrapers tests, 3 gate tests). Plan
  steps 5–8 remain: migration 0113, the submissions store, routes, and the §55.5
  automated check a reviewer reads. **Path B (WASM) is specified and deliberately
  not built** — §55.6 gates it on a sandbox that does not exist, and
  `scripts/check-wasm-gate.py` fails the build if a WASM runtime is adopted before
  then. Worth knowing: `wasmi 0.4` does not compile on this toolchain at all, so
  adopting Path B is not a one-line dependency change here.
- **M45-22, personal concierge queue (§54) — step 1 of 8 done.** The seen-exclusion
  fix shipped (`seen_work_ids` plus the real filter in `generate_traced`; both were
  dead code, so nothing excluded already-read works from any blend). Steps 2–8
  remain: migration 0113, domain selectors, store, routes, WIP notifications,
  frontend.
- **29 rows in the M45 tracker are still `planned`** (M45-22 … M45-56, excluding
  the two partially-implemented rows above). This is the largest remaining pool of
  named work in the repo. M45-18 was in this list while fully shipped; it is now
  `implemented-fully-tested`, so the count is 29, not 31.
- `origin` (git.polarisocial.xyz) is **down** — its Forgejo SQLite reports
  "database disk image is malformed" and every repository on it 500s, not just
  this one. It needs repair on that host; re-authenticating will not help. Work
  goes to github, and the thinkcentre mirror tracks it.
- `preread_reports` exists and is tested, but **no provider is configured**, so nothing
  writes a row in practice. Running an Ollama instance and pointing the adapter at it is the
  last manual step; §23.7's "AI features disabled without configuration" is why that is a
  valid state rather than a gap.
- The §0.3 "three TEXT/uuid columns" note should become six: `rec_strategy.rs` and
  `discovery.rs` added two, `preread_reports.work_id` is the sixth site. Each needs its own
  dialect arm and its own `work_uuid()` helper.
- `docs/plans/100-ideas-audit.md`'s gap F section is already marked CLOSED — the note
  saying otherwise was stale and has been removed.

## Decisions worth keeping

**The AI trait uses boxed futures, not `async fn`.** A native `async fn` in a trait
cannot state `Send` on the returned future, so a provider holding a `reqwest::Client`
could not be awaited in a spawned worker task — which is where every caller belongs.

**The adapter is deliberately narrow**: reachability, a quote, a run, and what was
charged. Four capabilities is one `POST /v1/chat/completions` plus one
`GET /v1/models`, which is why a stub server is sufficient verification. Ollama and most
local runners expose the same API, so they get this adapter rather than their own.

**The adapter was verified against a real axum server on a loopback port**, not a mocked
`reqwest`. Mocking would verify that the adapter calls the methods the mock expects — a
statement about the test, not the adapter. Binding a port also catches what a mock cannot:
a URL built wrong, a body field misspelled, a status treated as success.

**A quote is unpriced rather than guessed.** Nothing in the code knows what a token costs,
because §23.7 gives that to the administrator. So `CostQuote` has an explicit `priced`
field rather than inferring it from a non-empty `unit` — a local Ollama quote has unit
"tokens" and no price, and the first version let §22.11's budget guardrail through on a
figure that was not money.

**Three things an adapter must never do**, each a §23.7 requirement rather than a style
choice: return unvalidated model output (a chat completion is prose until parsed against a
schema), send private text without consent, and report a transport failure as an empty
result (an empty list reads downstream as "the work was assessed and scored nothing").

## Lessons from the merge (2026-10-03)

- **`cargo build` is the conflict resolver git is not.** `fetch_user_rated_work_vectors`
  came out of the merge as thinkcentre's real *body* with master's *stub signature*
  (`_db`, `_account_id`, underscore-prefixed because on master the function was a stub and
  never used them). Text-clean, wrong, and invisible to `git status`. Same class hit
  master's arena tests: `ArenaCard` gained `vector` and `apply_arena_ballot` a 4th
  argument, and the tests still called the old shapes.
- **Never predict a conflict count; count it.** The plan said 2 files and named
  `limiter.rs` + `docs/handoff.md`. Neither needed manual resolution. The real 4 blocks
  were in three files the plan did not mention.
- **A signature change on one side orphans the other side's call sites silently.**
  Same lesson, different mask: git resolves text, and a call site is text that still
  parses.
- **`assert_ne!(1500.0)` is not an assertion.** `apply_arena_ballot_updates_elo` asserted
  only that Elo *moved*, which passes for a ballot that moves it the **wrong way** —
  the exact bug the merge existed to fix. It now asserts the direction, with a mirrored
  test for the other direction. Both proven: flipping the sign turns them red and the
  original assertion stayed green.
- **`warnings are denied` turns a dead-code warning into a build failure.** That is
  usually what you want, but it also means a merged-in helper nobody calls
  (`limiter::Bucket::take` once `take_cost` landed) blocks the build until it is
  `#[cfg(test)]` or removed.

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
- **Neither engine has a portable SQL function**: no `md5` on SQLite, no `log10`, and no
  shared JSON function. Compute the value in Rust and order there.
- **Prove guards by breaking them.** Removing the range check turns exactly
  `a_stored_score_out_of_range_is_dropped...` red; removing `ON DELETE CASCADE` turns
  exactly `deleting_a_work_deletes_its_reports` red. Green is not evidence until broken.

## Lessons from the store (cost time too)

- **Encode and decode are a pair, and testing only one half passes.** `encode_missing`
  wrote `{"abstained": "NotConfigured"}` (the `Debug` of the enum) while `decode_missing`
  read a bare string. The object form matched no case, so every abstain came back as
  `InvalidOutput` carrying the JSON text as its reason. Store the variant *name*: an
  `AiAbstain` carries a provider's error text that has no business being persisted as a
  type tag, and re-parsing a `Debug` dump is fragile.
- **A fixture writing the wrong JSON shape makes a test pass for the wrong reason.** The
  corrupt-row tests wrote `{"tone": 0.7}` where the store writes `{"tone": {"score": 0.7}}`,
  so the whole row decoded as empty. The "length is absent" assertion passed anyway; the
  "tone still loads" assertion is what caught it.
- **In a Rust string literal, `\` + newline strips the newline and `\\` + newline keeps a
  literal backslash.** The doubled form put `\` into the SQL at every wrap point and SQLite
  rejected it as an unrecognized token.
- **A test file that is uncommitted is one bad `open(p, "w")` from gone.** Two files were
  zeroed this way before being caught. The `write_file` tool refuses to overwrite a file
  changed since the last read, which is the only reason one was recoverable.
- **Never `pkill -f 'cargo test'`.** It kills the mutation harness mid-write and leaves a
  half-written source file (`.then_with(...)//! Recommendation strategies...` prepended to
  line 1 of `rec_strategy.rs`). Use `process(action='kill')`, which kills the shell rather
  than its children mid-write.
- **A SKIP from every mutation in a run means the file is broken, not the patterns.** The
  blind-date harness had `B1 date removed from seed` targeting `blind_date_seed(account,
  today)` and reported SKIP — correctly, because a contaminated commit had changed that
  exact line to `blind_date_seed(account, "")`. The harness caught a bug I had shipped.
- **Do not explain away a failure that matches what you expected.** A 5-second suite taking
  128 s was the same signal that had just correctly flagged a mutated file, and I attributed
  it to a race instead. Check the runtime before explaining a red test.
- **A stale GREEN(BAD) is worse than no result.** A batch of mutation-gate output arrived
  from harness revisions that had since been rewritten; several mutations were reported
  GREEN(BAD) that were in fact RED. Re-run the single mutation by hand before believing it —
  and restore the source afterwards, since the harness restores by string-replace and a
  concurrent edit breaks the match.
- **An empty result and a missing entry look the same in the message.** `TestDb::
  applied_migrations()` returned `[]`, which reads like "migration 0104 missing" and sent the
  investigation at migration 0111 rather than at the accessor. `--nocapture` printing the
  count distinguished them immediately. A count of 0 means the accessor is broken; a count of
  N with one entry missing means the migration is.
- **Never `git add -A <dir>` while a mutation gate is running.** The gate's mutations sit in
  the working tree between its edit and its restore, so a broad add stages whichever one is
  applied at that instant. This contaminated a commit: "Close the H9 GREEN(BAD)" also carried
  `blind_date_seed(account, today)` → `blind_date_seed(account, "")`, making every day return
  the same work. Stage explicit paths, and read `git show --stat HEAD` after committing.
- **Never run a test binary while a mutation gate is running.** The gate edits the file
  under test, so a concurrent `cargo test` compiles mutated source and reports the gate's
  intended RED as a failure. Check `git status --short <file>` before believing any failure;
  a modified source plus a red test says nothing about your code. (Observed as a clean-tree
  `1 failed` on `HEAD`; an 8-second suite taking 259 s was the tell.)
- **This project has no `_sqlx_migrations` table.** It has its own migration runner, so
  `MigrationReport::already_applied` is the authoritative record of what is applied. Reaching
  for sqlx's ledger is a dead end that compiles and fails at runtime.