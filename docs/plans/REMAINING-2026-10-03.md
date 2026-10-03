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
| 2 | The pre-existing CI reds | Three remain, all reproduced on a clean stash and named rather than quietly fixed. See the table below. |
| 3 | The 27 `planned` + 6 `specified` tracker rows | M45-23 north-star, M45-25 … M45-55, M46-05 search. Each is a multi-week spec of its own. Listed so the number is honest rather than implied. |
| 4 | Path B (WASM), §55.4 | Gated by §55.6. `scripts/check-wasm-gate.py` fails the build if a WASM runtime is adopted; `wasmi 0.4` does not compile on this toolchain. |

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