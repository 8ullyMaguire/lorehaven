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

## The fifth red, closed (2026-10-04, pass nine)

The last of the pre-existing CI reds. `check-uncast-pg-placeholders` reported **15 sites**
and the standing note said they were "very likely wrong" — which was right, and the way to
settle it was to stop reasoning about PostgreSQL and ask it.

**Two of the fifteen were real defects. Both are fixed, and both are invisible to the
default engine.**

`hit_rate.rs` — the two PostgreSQL windowed metrics ended their outer `FROM` at a bare `)`.
PostgreSQL rejects an unaliased subquery outright (`42601 subquery in FROM must have an
alias`); SQLite accepts it. **10 tests red on PostgreSQL, green on SQLite.** The gate named
the *file* and gave the wrong *reason* (it called it an uncast placeholder), which is still
worth acting on: a report that is right about the file and wrong about the reason is a place
to look, and it is a cheaper use of the tool than either trusting or dismissing it.

`tasting.rs:1082` — `existing_match_state` decoded `arena_weights.matches_played` (INTEGER,
so INT8 on SQLite and INT4 on PostgreSQL) as part of an `Option<(f64, i64)>`:

    mismatched types; Rust type `i64` (as SQL type `INT8`)
    is not compatible with SQL type `INT4`

**No test could have caught this.** The statement sits behind
`if (next - current).abs() < f64::EPSILON { return Ok(false) }` on the way in from
`POST /api/v1/tasting/respond`, so `arena` 4/4 and `tasting_menu` 14/14 both pass on
PostgreSQL *with the bug in place*. Found by reading, confirmed by a direct probe, and the
reason it matters is the shape: this is the third defect in this repo that sat behind a
guard or an early return and therefore could not be found by the suite that covers it.

**The other thirteen were all false positives, each for a reason worth writing down:**

| rule | assumption | what PostgreSQL actually does |
|---|---|---|
| placeholder | a uuid column needs a cast | an untyped `$n` is **inferred** from the column — `PREPARE` accepts it. Only a `&str` bind fails (`incorrect binary data format`), and that needs the *bind* to decide |
| `SUM` | an INT4 summed into `i64` will not decode | `pg_typeof(COALESCE(SUM(amount_bp), 0))` is **`bigint`** — `sum()` widens. A bare *column* read into `i64` does fail; the aggregate is not the same trap |
| timestamptz | `::timestamptz` into a TEXT column is a fault | **assignment coerces** (`UPDATE 1`), **comparison does not** (`operator does not exist: text <= timestamp with time zone`) |

Gate: **15 → 0**, self-test **34/34** with a case for each side of each distinction. Proven
still live rather than merely silenced: reverting the `tasting.rs` cast makes the gate fire
on it again. The `SUM` rule and the uuid skip are left in the file, disabled and explained,
because a rule that found two real defects earns its keep and deleting one without a
replacement that catches the same bug trades a known quantity for a guess.

### Two lessons worth more than the fixes

**A regression test that copies the query proves only that PostgreSQL behaves like
PostgreSQL.** The first version of `crates/db/tests/arena_weights_decode.rs` hand-copied
the pre-fix and post-fix statements and asserted the first one fails. It stayed green while
the real query in `tasting.rs` had already been reverted. The assertion that catches
regression is a `read_to_string` of the module — verified by reverting the fix: green, then
red with `src/tasting.rs no longer casts matches_played`.

**A distribution assertion needs a threshold on the tail, not near the mean.**
`blind_date.rs` asserted `distinct.len() > 12` over 25 days drawn from 25 works. That is a
birthday problem — mean 16.0, sd 1.57 — so the bar sat ~2.5σ low and failed **1.2%** of
runs. It read as order-dependence, because it failed in a full-workspace run and passed
alone; it is not. The outcome depends on a hash of (account id, day) and the fixture mints
a fresh account per run. Simulated over 200,000 draws:

| threshold | fail rate |
|---|---|
| ≤ 10 | 0.018% |
| ≤ 12 | **1.204%** ← what it asserted |
| ≤ 14 | 16.704% |
| ≤ 16 | 62.805% |

Threshold 10 now, 20/20 green after, and the tree grepped for the same shape — one
instance, which is why it survived to be found by a failing gate.

### The trap that bit me while fixing my own fix

The `hit_rate.rs` fix carried the PostgreSQL error text as an **indented** doc-comment block,
which rustdoc collects as a *Rust* code block — so `cargo test --workspace` ran it as a
doctest and rustc read `42601` as an item. Fenced as ```` ```text ````. Worth recording
because the only reason it was caught is that the full-workspace run was happening anyway,
which is the argument for keeping doing it.

---

## Sixth pass — the same defect twice, and a gate that could not see its own input

Two PostgreSQL defects, found by a working gate, plus two silent gates that had been
reporting nothing while looking healthy.

### `analytics.rs` — a masked 500 on every reader's own reading totals

`READING_TOTALS_POSTGRES` had two unaliased subqueries in `FROM`:

```
42601 subquery in FROM must have an alias
DETAIL: For example, FROM (SELECT ...) [AS] foo.
```

SQLite accepts it. So the statement compiled, all 13 SQLite tests passed, and a reader
asking for their own reading totals got HTTP 500 on PostgreSQL. Four `analytics_gate`
tests were red for this — `an_own_metric_is_reachable_at_its_own_level`,
`the_response_states_the_floor_and_the_definition`,
`the_response_never_contains_a_credential_or_an_identifier`, and
`a_gallery_instance_withholds_the_community_surface`.

This is the **third** occurrence of this fault in the repository, after `hit_rate.rs`.
Now `scripts/check-pg-subquery-alias.py`, with a self-test.

**Diagnosis cost is the real lesson.** `ApiError::Internal` reports "Something went wrong
on our side" and nothing else, and `RUST_LOG=debug` on the failing test produced no output
at all. What resolved it was asking the server directly — `psql` reproduced `42601` on the
exact `words_read` subquery — because nothing in the test output could say it.

**The checker took four passes, and the self-test is what caught each:**

| # | Defect | Symptom |
|---|---|---|
| 1 | `if not CLAUSE_END.match(after)` | empty input is not a clause keyword, so end-of-statement read as **exempt** — a false negative on the simplest possible fault |
| 2 | added `AS\b` to fix #1 | vetoed a **real** alias: `FROM (VALUES (0),(1)) AS seq(value)`. Replaced with a positive `ALIAS_TOKEN`: an alias, or a finding. No negatives |
| 3 | `EXTRACT(EPOCH FROM (...))` | the `FROM` belongs to the function. 2 of the first 6 findings |
| 4 | `ON` is a bare identifier | `FROM (...) JOIN (...) ON true` read the second operand as aliased. An alias is a **name**, not a keyword |

The gate then reported **the file it had just repaired**, because my explanation of the
fix was written as SQL `--` comments inside the PostgreSQL arm. SQL-comment stripping
exists so a fix can explain itself in place; the explanation moved to a doc comment above
the const, where it belongs. The self-test routes through `scan`'s own preprocessing, so
that case cannot pass while the gate still reports it.

Final: 13 findings → 0. Self-test 10/10.

### `spoilers.rs` — TEXT compared against timestamptz, and a schema reader with a hole

`list_due_scheduled_posts` bound `$1::timestamptz` against `forum_posts.scheduled_at`,
which migration 0041 declares **TEXT**:

```
HINT:  No operator matches the given name and argument types.
```

Confirmed against the server rather than inferred: `text <= timestamptz` errors, and the
same comparison with `$1::text` returns the row. Same defect class as
`fix-timestamptz-binds.py` — the fixer that is *disabled* precisely because the timestamp
columns here are TEXT. This call site survived it because it was never on the fixer's
list. `$1::text` is the fix.

It was the one function in `spoilers.rs` with **no test at all**, which is why thirteen
passing M34 tests never saw it: they call `schedule_post`, which *writes* the column with
no cast and no comparison. The new test asserts the comparison, including `<=` rather
than `<`, since a cast silently moves that boundary. SQLite 19/19; PG confirmation below.

### Why a working gate found it

`check-uncast-pg-placeholders.py` had been reporting this all along, but its schema
reader only parsed `CREATE TABLE`, so it never saw the column. Same shape of bug as the
checker's own documented history, one level down: **a checker that reads its own input
too narrowly reports nothing and says nothing.**

`load_schema` now handles `ALTER TABLE ... ADD COLUMN` — how every column added after its
table's own migration arrives. Found by writing migration 0115
(`recommendation_slots.mechanism`) and asking the schema what it thought: `None`.

That surfaced two more sites, and the second is the more interesting one:

| site | verdict |
|---|---|
| `spoilers.rs:353` | **real defect**, fixed above |
| `canon_agnostic.rs:304` | **false positive** — the code was right |

`canon_agnostic` decodes into `CanonRowPostgres`, which declares both columns `Option<i32>`
with a doc comment explaining exactly why (sqlx checks INT4's width). `decoded_as_i64`
could not read a tuple alias, so it returned "cannot tell", which its own contract says
must report. Added `tuple_alias_fields`: positional, resolving the Postgres-named alias.

Two errors of my own inside that, both **silent** — no exception, just a wrong answer:

- tested `column in INT4_COLUMNS`, but that tuple holds *types* (`integer`, `smallint`),
  not column names — always False, so it reported the site anyway.
- then `_KNOWN_SCHEMA`, a name that does not exist. Would have raised `NameError`.

Both were found by calling the function directly rather than reading the diff. The schema
is now threaded through `decoded_as_i64` as a parameter rather than closed over.

Result: 2 sites → 0, self-test 34/34, **306** tables read.

### A third silent gate

`check-pg-uuid-casts.py` printed **nothing** when clean, which is indistinguishable from a
checker that has stopped reading its input. It now reports what it read: 499 files, 2225
columns.

### Correction: a claim in this session was not supported by the command shown

I reported "`just check`: all static CI gates green". **`just` is not installed on this
host** — the recipe could not have run. The gates are real and each has now been run
directly, but the claim was not backed by the command cited.

Run individually, all 16 green; `cargo fmt --check` clean; `clippy -D warnings` clean.

### Verification

| gate | result |
|---|---|
| `analytics_gate` on **PostgreSQL** | **13/13** (270.6s), was 9/13 |
| `analytics_gate` gate mutation | aliases removed → gate reports both sites |
| `canon_agnostic_store` on PostgreSQL | **12/12** (271.7s) — confirms the false positive |
| `milestone_34_spoilers` on SQLite | **19/19** (was 18) |
| `milestone_34_spoilers` on PostgreSQL | see below |
| `hit_rate`, `migration_catalogue` on SQLite | 10/10, 6/6 |
| E2E (Playwright, release binary) | **84/84** (13.4m) |
| static gates | 16/16, fmt clean, clippy 0 |

---

## M45-23 implemented — steps 2 and 4, with three more PostgreSQL-only defects

Step 1 (record the mechanism at serve time) shipped at `v0.58`: migration 0115, the
`mechanism` field on `SlotRecord`, `SlotMechanism` in the domain, and both serve paths in
`discovery.rs` populating it. Steps 2 and 4 are done; step 3 (the route) is not.

### The three defects

All three passed on SQLite and failed only on PostgreSQL. All three were reproduced with
`psql` **before** being fixed, not inferred from the error string.

| # | error | cause | fix |
|---|---|---|---|
| 1 | `operator does not exist: text <= timestamp with time zone` | the correlated subquery compares `recommendation_slots.created_at` (TIMESTAMPTZ) against `rating.created_at` (TEXT) | cast the **TEXT** side |
| 2 | `42803 subquery uses ungrouped column "r.created_at" from outer query` | `GROUP BY r.work_id` leaves the correlated reference ungrouped | `GROUP BY r.work_id, r.created_at` |
| 3 | `42883 operator does not exist: timestamp with time zone >= text` | the slot-count window filter bound bare `$1`/`$2` against a TIMESTAMPTZ column | `$1::timestamptz` |

Note the direction of #1. `spoilers.rs` — fixed earlier in this same pass — had the
**mirror image**: `$1::timestamptz` bound against a TEXT column. The rule that settles
both: **cast the TEXT side to `timestamptz`, never the reverse.** The column's own type
decides what the other side may be.

#3 looked arbitrary until the two columns were read side by side:

```
reading_status.updated_at         TEXT          -- a bare $1 works
recommendation_slots.created_at   TIMESTAMPTZ   -- a bare $1 is 42883
```

### Two errors of my own, both silent

- **One SQL fragment shared between two alias scopes.** The correlated subquery aliases its
  table `s2`; the outer join aliases `s`. A single `cmp` fragment mentioning `s` produced
  `AND s2.created_at <= ... AND s.created_at <= ...` with `s` out of scope inside the
  subquery. SQLite reported `near "s": syntax error` — the least informative message a SQL
  engine has. Found by **printing the generated statement**, because `s` is a legal alias
  everywhere else in the query.
- **`unattributed` was in `by_mechanism` *and* a top-level field.** That double-counted it,
  so `shares_account_for_everything()` failed on a metric that was arithmetically correct.
  The invariant is "mechanisms **plus** unattributed"; putting unattributed inside the
  mechanisms breaks its own definition.

### And the doctest trap, twice, in consecutive passes

`media_resilience.rs` — my fix comment quoted the `42601` error as an **indented** doc
block, which rustdoc collects as a *Rust* code block, so `cargo test` ran it as a doctest
and rustc read `42601` as an item:

```
test crates/db/src/media_resilience.rs - media_resilience::count_well_mirrored (line 3161) ... FAILED
```

Identical to the `hit_rate.rs` trap from the previous pass, one commit earlier — where I
had already written down how to avoid it. Knowing the rule and applying it are separate
skills; fencing the block was the whole fix.

### Five FK failures that named nothing

`FOREIGN KEY constraint failed` names no column. Every test had minted its **own** account
with a slightly different label from the seed's, so each rating referenced an account that
did not exist. The error sent me looking at `works` and at primary keys first. What found
it was an explicit per-target existence check:

```
FK target missing: accounts a6b5b79b-… is not in the table
```

The account now comes from the seed, so a test cannot invent a second one.

### Verification

| gate | SQLite | PostgreSQL |
|---|---|---|
| `north_star` (7 plan cases) | **7/7** | **7/7** (110.3s) |
| `north_star_arithmetic` (13 degenerate inputs) | **13/13** | n/a (pure) |
| workspace `cargo test` | see below | — |
| doctests | clean | — |
| fmt, clippy `-D warnings` | clean | — |

The 13 arithmetic tests call the **real** functions, which are `pub` for that reason. An
earlier draft re-implemented `median` locally — the same mistake
`arena_weights_decode.rs` records, where a hand-copied query stayed green while the real
one in `tasting.rs` had been reverted.

### Step 3 — the route, and a committed fix that was silently lost

`GET /admin/metrics/north-star`, operator-only, registered in `route_inventory.rs` as
`Audience::Operator` so it is *declared* as well as mounted.

The plan said to copy M45-18's route constraints rather than reinvent them. That route is
`flows.rs`, and its three constraints are copied with their reasons:

| constraint | why |
|---|---|
| **404, not 403** | for an operator view the *existence* is the disclosure — 403 answers "yes, and you may not" |
| **no per-account detail** | §53.2, asserted at the type level *and* by walking the response body |
| **no target, no grade** | §53.5: "The number is read, not chased." |

Case 4 of the route tests is the one that cannot be written by copying a sibling: the rule
is an **absence**, so it is tested by walking every key in the body at every depth and
banning `target`, `goal`, `grade`, `expected`, `threshold`, `benchmark`. Asserting
`body["target"]` is absent would pass just as well with a field named `goal_rate`.

#### The lesson that cost the most: a committed fix that was silently lost

The `GROUP BY r.created_at` fix for PostgreSQL's `42803` **was committed**, and then lost:

1. a mutation run reverted it,
2. `cp` put back a whitespace-different copy,
3. a later `git checkout` — intended *only* to undo trailing whitespace — restored an
   **earlier commit's** version, dropping the edit.

Nothing failed. SQLite stayed 7/7, fmt and clippy stayed clean, and `git status` showed a
clean tree — **because the lost edit was the committed one.**

It surfaced as four 500s from the route test, masked behind "Something went wrong on our
side". Cost: a detour through `psql` and three temporary diagnostics before running the
obvious check — *is the fix still in the file?*

> **After any mutation-and-restore cycle, `grep` for the fix.** `git status` clean means
> "matches HEAD", not "correct". The reason is now written into the source at the fix.

#### Mutation testing is easy to do vacuously

Four attempts to prove the 404 rule, and the first three proved **nothing**:

| # | mutation | outcome |
|---|---|---|
| 1 | remove the gate call | did not compile — unused-variable warnings are denied |
| 2 | `AppError::Forbidden` | no such variant |
| 3 | `AppError::AccessDenied { resource }` | that variant takes no field |
| 4 | `AppError::AccessDenied` | **403 → exactly case 1 red, other three green** |

A mutation that does not compile is not a verdict. Check that the run actually executed.

#### One test-helper bug

`account_id_by_handle` decoded `accounts.id` as `String` on both engines; it is **UUID** on
PostgreSQL, so all four tests 500ed there with a `ColumnDecode` error naming no column.
Per-engine decode now, the way `flow_dashboard.rs` already does it.
