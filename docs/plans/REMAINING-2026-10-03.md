# Lorehaven — remaining work, 2026-10-03 (end of session)

Base for this revision: `baf297d`, tagged **`v0.56-concierge`**. Everything below is
what is *still* to do, in the order it will be done.

## Environment this session needs before anything runs

| Fact | Value |
|---|---|
| Toolchain | `export PATH="$HOME/.cargo/bin:$PATH"` **first**. System `rustc` is broken (partial Arch upgrade, libLLVM 22.1 gone). |
| PostgreSQL | `sudo docker start lh-pg-test`. Container `lh-pg-test` on **127.0.0.1:55433**, user `lorehaven`, db `postgres`, password `lorehaven`. |
| Test URL | `export LOREHAVEN_TEST_PG_URL='postgres://lorehaven:lorehaven@127.0.0.1:55433/postgres'` — **both engines means running the suite twice**, once with this set and once without. `unset` it before the SQLite run; a leftover value sends every test to a host that does not resolve. |
| SQLite | Default. Unset the variable above and nothing else is needed. |
| Frontend | `cd frontend && node node_modules/vitest/vitest.mjs run <file>`. Not `pnpm test`. |
| btrfs | Metadata DUP pool ~12.6/13.0 GiB. Builds are SLOW, not stalled. Before believing a hang: `ps -eo pid,stat,etime,comm \| awk '$2 ~ /^D/'`. |
| Mutation runs | One `cargo test` on the app suite is ~10 s SQLite, ~190 s PostgreSQL. Seven mutations do not fit a 300 s cell — run them backgrounded via a script, not inline in `execute_code`. |

## Landed this session

| # | Work | Step | Evidence |
|---|---|---|---|
| 1 | M45-57 HTTP routes + §55.5 check | 7–8 | `source_adapter_routes.rs` 9/9, `curator.rs` 5/5, `lorehaven-scrapers` 364/364 |
| 2 | M45-22 migration 0114 | 2 | 114/114 apply on both engines |
| 3 | M45-22 domain + store | 3–4 | `concierge_store.rs` 13/13 both engines |
| 4 | M45-22 routes + WIP notify | 5–6 | `concierge_routes.rs` 11/11 **both engines** |
| 5 | M45-22 frontend | 7 | `Concierge.test.ts` 11/11 |
| 6 | `route_inventory` working | — | 3/3, first time green in the file's history |

### Defects found and fixed, each with its own proof

1. **PostgreSQL-only 500: `uuid = text`** in `duration_estimates` and
   `filter_by_mood`. Both bound work ids as strings, which is *correct* on SQLite
   (every id column is `TEXT`) and a 500 on PostgreSQL (they are `uuid`). Found
   **only** by running the suite with `LOREHAVEN_TEST_PG_URL` set. Fixed by
   `bind_work_ids!`; proved green on SQLite and red on PostgreSQL against a reverted
   bind.
2. **The same defect written twice**, one function apart — so fixing the first was no
   evidence about the second. That is why the macro exists and why
   `concierge_uuid_binds.rs` covers both call sites.
3. **`extract_handlers` had an infinite loop fixed in one branch and not the other.**
   Both branches carried a comment describing a step the code did not perform.
4. **`extract_handlers` returned METHOD names** where `ROUTE_TABLE` is keyed on
   HANDLER names. Invisible while the walk collected nothing.
5. **The route walk was line-at-a-time**, so a route call rustfmt wrapped across lines
   was invisible. Now parenthesised-delimited. **Found 19 routes across 9 modules that
   have been registered and served since those modules existed and were never
   tabulated.**
6. **§54.7 parity was broken: the concierge served the discovery feed exactly
   REVERSED.** `render_queue` called `rec_engine::generate_with_registry` (the
   pluggable path) while the default `rec_mode` is `legacy` and takes `blend()`. Fixed
   by extracting `discovery::build_candidate_engines` and calling it from both routes.
7. **Two route-inventory guards are defence-in-depth**, labelled as such in the file:
   removing either leaves the suite green (measured). Both were proved by corrupting
   their *subject* instead — a wrong method, a wrong path, a reintroduced duplicate.

## The list — what is left

| # | Work | Done when |
|---|---|---|
| 1 | `spec.md` §54 → implementation note | The spec claims the concierge exists; a reader should be able to check that in one grep. |
| 2 | The pre-existing CI reds | **Four of five closed.** `check-sqlite-migration-syntax`, `check-pg-uuid-casts` and `find-single-backend-suites` at `c044870`; `check-pg-arm-uses-sqlite-pool` at `745a5cf`. Three were gates reporting correct code as defective; one was a suite claiming both engines while SQLite-only. |
| 2b | `check-uncast-pg-placeholders` | **Closed — and it found two real defects on the way, so the fix was NOT "make the gate quiet".** Measured on PostgreSQL 15.19, not reasoned about. **15 sites, 0 real.** Three rules were firing on correct code, and the semantics are now pinned in the file's self-test (34/34): **(a)** a uuid or NUMERIC placeholder needs no cast — PostgreSQL infers an untyped parameter from the column, proven with `PREPARE` against both a uuid and a text column; what actually fails is binding a **`&str`** to a uuid column (`incorrect binary data format`), a different fault the sites do not have because they bind a parsed `Uuid`. **(b)** `COALESCE(SUM(amount_bp), 0)` over NUMERIC decodes fine — `retention_proposals` 12/12, `preservation_ledger` 7/7, `payout_recalc_wiring` 12/12 on PG. **(c)** **assignment coerces, comparison does not** — `UPDATE t SET ts = $1::timestamptz` into a TEXT column returns UPDATE 1, `WHERE ts <= $1::timestamptz` errors with `operator does not exist`. Rule (c) was rewritten to comparison-only; (a) and (b) are left in place because deleting a rule that has caught real bugs needs a replacement that catches the same bug. **The two defects it DID find are fixed and committed:** `hit_rate.rs` (missing `FROM` alias — the gate named the file, not the fault) and `tasting.rs:1082` (a true INT4→i64 decode, which **no test could reach** — see row 2c). |
| 2c | `tasting.rs` INT4→i64 decode | **Closed.** `existing_match_state` decoded `arena_weights.matches_played` (INTEGER → INT8 on SQLite, INT4 on PostgreSQL) as `i64`, which fails on PostgreSQL with `mismatched types; Rust type i64 (as SQL type INT8) is not compatible with SQL type INT4`. Fixed with `CAST(... AS BIGINT)` — `::bigint` is PostgreSQL-only and the statement is spelled once for both backends. **No test caught it and none could**: the statement is reached from `POST /api/v1/tasting/respond` only once a response moves a weight, and an `if (next - current).abs() < f64::EPSILON { return Ok(false) }` guard above the call means every test that got that far returned early — `arena` 4/4 and `tasting_menu` 14/14 both pass on PostgreSQL *with the bug in place*. Found by reading plus a direct probe. `crates/db/tests/arena_weights_decode.rs` is the regression, and its load-bearing assertion is that **`src/tasting.rs` still spells the cast** — a hand-copied statement in a test drifts from the original, and the first version of that file passed happily while the real query had already been reverted. Verified by reverting: green, then red. |
| 3 | The 27 `planned` + 6 `specified` tracker rows | M45-23 north-star, M45-25 … M45-55, M46-05 search. Each is a multi-week spec of its own. Listed so the number is honest rather than implied. |
| 4 | Path B (WASM), §55.4 | Gated by §55.6. `scripts/check-wasm-gate.py` fails the build if a WASM runtime is adopted; `wasmi 0.4` does not compile on this toolchain. |

## What was verified, and what had never been run

The 2026-10-03 evening pass ran every gate this project has. Two of them had **never
been run at all**, and both found things.

| Gate | Before today | Result |
|---|---|---|
| `cargo test --workspace` | **never run** | **12–17 failures** → 4052/4052 after two fixes |
| `svelte-check` (`just check-frontend`) | **never run since the page shipped** | 2 errors → 0 errors, 0 warnings |
| Playwright E2E | run once | 84/84 → re-run green |
| frontend vitest | run | 439 → 445 |
| 8 static CI gates | 6 red | 5 closed, 1 non-failure; 2 gates still red on untouched files |
| clippy `--workspace --all-targets` | run | 0 |

### The workspace suite: 12 failures that were not bugs

```
panicked at crates/test-support/src/lib.rs: connect: connecting to SQLite at ...
Caused by: pool timed out while waiting for an open connection
```

Every affected suite passes 100% alone — at `--test-threads` 16, 4 and 1, and two of
them concurrently also pass. So it was never a defect in any suite, and calling it
"flaky under load" would have hidden the next real failure of the same shape.

**Cause.** `cargo test --workspace` runs 198 suites; each `TestDb` clones a migrated
file and opens its own `SqlitePool`. 134 of the 182 test files never call
`TestDb::cleanup`, and nothing sweeps `TMPDIR`, so one run left **6,925 directories
totalling 29GB** on a filesystem already 15GB into swap. Pool construction under that
much pressure outran the 10s `acquire_timeout`.

**Two fixes, in the right order.**

1. `test_db_config()` widens the *test* acquire timeout to 60s. Production defaults
   untouched. Proven by control at a load high enough to reproduce:

   | `acquire_timeout` | suites failing | pool timeouts | wall |
   |---|---|---|---|
   | 10s (default) | 1 of 40 | 1 | 105s |
   | 60s | 0 of 40 | 0 | 73s |

   The first attempt at that control was worthless and is why the load is 40 and not
   4: four previously-failing suites run concurrently passed 81/81 **both before and
   after**, which would have "shown" the change doing nothing.

2. `TestDb` records its `sqlite_dir`; `cleanup()` removes it after closing the pool.
   Plus `sweep_stale_sqlite()` for the databases a failing test never returns from.
   Scratch went **29GB → 364MB**.

`sweep_stale_sqlite()` is **age-based, not liveness-based**, and the doc says so: std
cannot ask a SQLite file whether anyone has it open, whereas the PostgreSQL sweep asks
`pg_database` about attached backends. It is deliberately **not** wired into the test
run, because the export-worker and postgres-journey suites idle for minutes and a false
"dead" would delete a running test's database. A `Drop` impl is the right answer and is
named rather than attempted.

Its own test caught a bug in it first: "a directory with no database file is dead"
looked obviously right and is wrong, because `scratch_dir()` and `connect_with_dir()`
are a moment apart and the first version deleted starting tests' directories. Both
mutations CAUGHT.

**Not a complete fix, stated plainly.** With 60s the run still lost 2 tests at suite
131. The leak fix is what closed it — the final run is 198 suites / **4052 passed /
0 failed / 0 pool timeouts**, at default parallelism, exit 0.

### The type checker: a feature that was designed and never built

`svelte-check` reported `Concierge.svelte:59 'knownMoods' is declared but its value is
never read`. Following it found a whole unfinished feature rather than one dead
variable: the component declared `knownMoods` and `moodError`, set `moodError = null`,
rendered neither, carried a doc comment explaining that §54.2's mood vocabulary "comes
from the server's validation error instead, which is the only place it appears" — **and
had no input for a mood at all.** A budget selector and no way to ask for a mood.

Built, with six tests. Three exist because a mutation came back GREEN, and the third is
the one worth keeping:

| Mutation | Result |
|---|---|
| moods not parsed out of the message | 2 failed — CAUGHT |
| a 500 swallowed into the mood path | 1 failed — CAUGHT |
| learned list never cleared | 1 failed — CAUGHT |
| status gate widened `422` → `>= 400` | GREEN → then CAUGHT |

The widened gate was GREEN because the 500 fixture carries no `fieldErrors`, so it
cannot tell "422 about a mood" from "some other status about a mood". A 500 **with**
`fieldErrors.mood` is ordinary — a validation failing mid-handler, a wrapped upstream
error — and swallowing it into an inline mood sentence with no retry hides a real
failure. So the gate is exact-422, and there is now a test whose 500 carries
`fieldErrors.mood`.

Also fixed: **12 dead CSS custom properties** in `Concierge.svelte`. `--text-muted`,
`--border`, `--text`, `--surface-muted`, `--surface-hover` and `--danger` are not defined
anywhere in `tokens.css`, so every rule using them was silently unstyled. And
`PreReadPanel.test.ts:99`, the last svelte-check error in the tree, constructed
`new ApiError('boom', 'SERVER_ERROR', 500, 'server exploded')` against a
`(status, code, message, requestId)` signature — the assertion passed only because the
string was sitting in `requestId`.

### The static CI gates, run 2026-10-03

None of the eight `scripts/check-*.py` gates had been run this session. Six were red.
Four were fixed; one was a checker giving advice that made things worse; the rest is
old debt, named rather than absorbed.

| Gate | Was | Now | What it found |
|---|---|---|---|
| `check-snapshot-pii.py` | red | **green** | My two `account_id` columns. 14 policy decisions added; both `rekey_account`. |
| `check-pg-arm-uses-sqlite-pool.py` | 23 | 22 | My `exec()` branched on a `pg` bool; now matches `Backend::`. |
| `check-uncast-pg-placeholders.py` | 17 | 15 | **Mine, and the checker's fix was wrong** — see below. |
| `check-snapshot-channel.py` | "red" | **green** | Not a failure: it needs `--target`/`--file`, or `--self-test`. Passes. |
| `check-pg-backend-arm-placeholders.py` | green | green | — |
| `check-wasm-gate.py` | green | green | — |
| `check-pg-uuid-casts.py` | red | red | Pre-existing, `snapshot_anonymisation.rs:1023`, untouched since `f4cbbda` (2026-09-30). |
| `find-single-backend-suites.py` | 3 files | 3 files | Pre-existing: `m53_source_credentials.rs`, `canon_class.rs`, `query_fields_characters.rs`. |
| `check-sqlite-migration-syntax.py --self-test` | 5 cases | 5 cases | Pre-existing, untouched since `b1d1ce8` (2026-09-29). |

**The false lead, in full, because it is the kind of thing that gets repeated.**
`check-uncast-pg-placeholders.py` reports `INT4 column into i64` on both
`concierge_store.rs` SELECTs and tells you to widen to `::int8`. Doing exactly that took
PostgreSQL from **13/13 to 11/13**, with the mirror-image error:

```
Rust type `core::option::Option<i32>` (as SQL type `INT4`) is not compatible with
SQL type `INT8`
```

`SessionRow` declares `Option<i32>`, so the uncast INT4 was already correct — it is the
one narrowing the checker is willing to call right. The checker infers the Rust type
from the SELECT list and cannot see the `FromRow` struct. **Its read of the fault was
right; its advice about the fix was backwards.** Reverted, with a comment on the SELECT
so the next person does not "fix" it again.

### A stale token the E2E suite had been red on

`analytics.spec.ts` asserts `scroll-padding-top >= .site-header` height. It failed by
**3.59px**: the token said `4.0625rem` (65px) and the header measures **68.59375px**.

Not caused by the new nav entry, and this was measured rather than argued: `dist` built
from `a6d62a9`'s `App.svelte` (17 nav items) and from HEAD (18) both report
`header: 68.59375`, byte-identical, with `navItems: 18` in both because the probe counts
rendered `.desktop a` and the overflow container still lays them all out. `.bar` is
`min-height: 4rem` and the content decides the rest.

Fixed the token (`4.2875rem`) rather than the test. The shape of the bug is worth
naming: a fixed token compared against a **content-sized** element is a standing
invitation to drift, and it presents as an analytics bug because analytics is where the
test lives. The token's own comment already said to revisit it when the header grows.

### The full E2E run — 83 tests, 12.4 minutes

Single worker, `reuseExistingServer: false`, fresh scratch SQLite per run, against the
release binary with the embedded frontend.

| Run | Result |
|---|---|
| First full run (before the token fix) | **82 passed, 1 failed** — `analytics.spec.ts:307`, the `scroll-padding-top` comparison |
| Second full run, after the token fix + binary rebuild | **84 passed, 0 failed** (13.5m) — includes the new concierge journey |
| `analytics.spec.ts:307` alone, before the whole-suite re-run | 1 passed |
| `coverage.spec.ts` concierge test, alone | 1 passed (11.9s) |
| Same test with `/me/concierge` forced to 404 | **1 failed** — the mutation it exists to catch |

84 rather than 83 because the concierge journey was added between the runs.

**The token fix was invisible until the binary was rebuilt.** `rust-embed` bakes
`frontend/dist` into the binary at compile time, so a CSS-only change leaves the served
page stale: after editing the token, a fresh measurement still read `pad: 65px`. Only
`cargo build --release --bin lorehaven` made it real. Worth remembering as a class of
false negative — a correct fix that measures as a failed one.

### E2E coverage now includes the concierge

`coverage.spec.ts` gains a live test: register, publish three works, load `/concierge`,
and require the page to resolve to a real queue or a §54.6 explained-empty — never an
error summary, never a skeleton that never settles. This is the only test in the file
that would notice **the route existing while the endpoint behind it 404s**, which is
the exact shape of bug the page shipped with.

Selector note worth keeping: `ErrorSummary` renders `role="alert"` on `.summary` and has
no `.error-summary` class. A guard selector written from the component's *filename*
matches nothing and silently never fires.

## Out of scope here, on purpose

- **§36.11 mood journal.** §54 reuses §15.8's mood *vocabulary*; the journal is
  separate work and the plan says so.
- **Operator affinity / theme gravity in the concierge.** Discovery applies both;
  the concierge does not, because §54 gives it no `sort` parameter. On an instance
  that has affinities set, the two feeds may legitimately differ. Stated in the code
  rather than hidden — see `routes/concierge.rs` step 3.
- **The legacy blend's missing seen-exclusion.** `RecContext.seen`
  (`rec_engine.rs:66`) only reaches the pluggable path; `blend()` applies none, so a
  reader is served already-read works. Real, predates §54, and "fix it here" would
  smuggle a ranking change into a concierge fixture. Recorded in
  `concierge_routes.rs`'s parity test.

## Trap log for whoever continues

These are all measured this session, not folklore.

- **sqlx does not translate `?1` → `$1` for PostgreSQL.** Worse:
  `rewrite_placeholders` (`db/src/lib.rs:462`) renumbers every `?` it meets *in order*
  and ignores the digits, so `VALUES (?3#t, ?3#t)` becomes `$3, $4` and the statement
  asks for a bind it was never given (42P18). Use the fold in
  `concierge_store.rs`'s `exec`, or the slot-aware `exec` in `concierge_routes.rs`.
- **A repeated slot needs its own bind only if the rewrite expands it.** Bind count
  and placeholder count must be checked against the *actual* rewrite, not the source.
- **Bind ids as `Uuid` on PostgreSQL**, never as text: `uuid = text` (42883). And a
  malformed id must be bound as **NULL, not omitted** — PostgreSQL binds
  positionally and a mismatched count is 08P01, so "skip this one" is an error rather
  than a filter.
- **`rating.id` has no default on PostgreSQL** while SQLite defaults it. Name the
  column; the row is rejected with 23502 otherwise.
- **`chapters.current_revision_id` REFERENCES `chapter_revisions(id)`** — insert the
  chapter with NULL, then the revision, then `UPDATE`. The other order fails on
  PostgreSQL only.
- **`chapters.order_key`, not `position`.** Spaced by ten so an insert between two
  chapters is one write.
- **`rating.stars`, not `rating.value`.** `pseud_id` is NOT NULL and `updated_at` too.
- **`chapter_revisions.created_by_pseud_id` is NOT NULL** and FK-constrained.
- **A scratch PostgreSQL URL must REPLACE the last path segment**, not append:
  `postgres://…/postgres` + `/lh_…` parses as a unix-socket host and fails with
  `failed to lookup address information`.
- **`work_tags.work_id` has a real FK on PostgreSQL and none on SQLite.** Tagging a
  work before inserting it passes on SQLite and fails on PostgreSQL.
- **`moods_in_use` and `filter_by_mood` both require `lifecycle = 'published'`, but
  only the blend requires `visibility = 'public'`.** So a published-but-unlisted work
  is the one shape where validation sees a mood and the blend never serves it — which
  is what makes "matched nothing" testable. Lifecycle will NOT do it: it removes the
  mood from the available list too, and the request comes back 422-unknown.
- **A per-file SQL helper is a liability.** Three `exec` helpers now exist in this
  repo and each has a bug fixed in a sibling. One shared helper, or an honest comment
  saying why not.
- **`git reset --hard` to an older sha destroys commits that reflog still holds.**
  I did this while trying to squash WIP commits and lost two; `git reflog` recovered
  them. Use `reset --soft` and verify with `git diff <sha> HEAD --stat` before
  committing.

## The three reds, closed (2026-10-03, third pass)

Each had been carried as "pre-existing, named rather than quietly fixed" for two passes.
All three were real, and **two were gates that could not do their job**.

| Gate | What it reported | What was actually true |
|---|---|---|
| `check-sqlite-migration-syntax` | 5 of 14 self-test cases failing | **The gate was reading the wrong SQLite.** `bundled_sqlite_version()` globbed `libsqlite3-sys-*/sqlite3.h` and took `vendored[0]` — whichever sorted first. That is **3.51.3 from `libsqlite3-sys 0.37.0`, a crate this project does not depend on**; the lockfile pins 0.30.1, which bundles 3.46.0. `forbidden_construct()` returns the rules active BELOW the given version, so reading high made **all four rules inactive**. |
| `check-pg-uuid-casts` | `snapshot_anonymisation.rs:1023 id:: text on accounts` | A **correct** statement. `CAST_AFTER` was `\$N::(\w+)`, so on `WHERE id = $1::text::uuid` it read the **first** cast in the chain (`text`) and compared it to a uuid column. The value is a uuid. |
| `find-single-backend-suites` | `m53_source_credentials.rs` hardcoded `sqlite://` | **A real defect.** `TestDb::connect_with_dir` honours `LOREHAVEN_TEST_PG_URL` and on PG creates a scratch database while ignoring `dir` — so the file's config pointed at a SQLite file that did not exist, next to a live PG pool. |

The first is the one worth keeping. A gate whose self-test fails 5 cases is not noise —
it is the gate saying it has been configured into uselessness — and both halves agreed
because both read the same wrong number. **A checker that cannot detect its own
misconfiguration is worse than no checker**, because it converts a red build into a
green lie. Its self-test failing was the only honest signal it had left, and I had
written it off twice.

Deferring was the error, not the fixing. Two of the three carried a reason I had
written for myself: *"needs the snapshot masking schema understood"* (it needed a
regex read properly) and *"5 self-test cases, named not absorbed"*. Plausible-sounding
reasons are not the same as reasons.

### What was NOT a false positive

`check-pg-arm-uses-sqlite-pool` (22 sites) and the remaining 15 of
`check-uncast-pg-placeholders` are still red. None is in a file this work touched, and
neither is demonstrably wrong, so both stand as real work rather than being silenced.

`check-uncast-pg-placeholders` also had **2 false positives of its own**, removed:
`SessionRow.budget_minutes` and `.truncated_at` are `pub Option<i32>`, and the int4
rule could not see a `#[derive(FromRow)]` struct. Its advice would have made sqlx fail
to **compile** the query — the same failure mode as the `::int8` finding one pass
earlier. It now resolves the struct per column, proven by mutating both fields to
`Option<i64>` and watching the 2 findings return.


## The fourth red, closed (2026-10-03, pass four)

`check-pg-arm-uses-sqlite-pool` had 22 findings. I had twice recorded them as "in files
this work never touched -- real work rather than noise." **All 22 were wrong**, and the
gate had been unable to do its job since the commit that introduced it.

`scan()` tracked brace depth treating both `"` and `'` as string delimiters. Rust has no
single-quoted string literal -- `&'a SqlitePool` is a lifetime. An apostrophe in a `//`
comment opened a phantom string that closed on the `'` inside a SQL literal 23 lines
later, swallowing the SQLite arm's braces; depth never returned to zero and the
depth -> header map went stale for the rest of the file. Every `sqlite_pool()` after that
point was reported inside `Backend::Postgres =>`.

The gate reported the **one** real defect it was written for (`thread_modes.rs`, in the
same commit) alongside **18** invented ones -- the true finding was 1 part in 19.

Three smaller blind spots in the same gate: `is_postgres()` was not recognised as a
backend branch (the spelling 17 test files use); a function's span ran to the next `fn`,
sweeping in the next function's doc comment; and a file opening its own `Database` from a
hardcoded `sqlite://` was judged as though it had an unported arm.

The gate now has a 6-case self-test wired into CI before the gate itself, and every fix
is verified by reverting it. The apostrophe case reads a **verbatim 310-line prefix** of
the real file, kept as `scripts/fixtures/quote_in_comment.rs`, because three
hand-written miniatures of that shape all came back *correct* against the broken scanner.

In production the pass also found `retention_proposals.rs` using two sequential
`if let Some(pool)` blocks -- correct today, and a silent no-op if neither pool matched,
which is the failure a third backend would inherit. The house idiom is
`match db.backend()` (1039 arms against these 5 outliers, and that ratio is what
identified them as the anomaly). Converted, along with
`preservation_ledger.rs::seed_work`, which does run on both engines.

Verified: SQLite 59/59 across the retention and preservation suites; **PostgreSQL 52/52**,
plus `preservation_ledger` 7/7 on PostgreSQL.
