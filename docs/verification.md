# Verification log — Lorehaven

Newest first. Each section states what was verified, how, and the result.

## 2026-09-16 — PG dialect parity complete, SQLite regressions from the parity pass fixed

**Commit:** `69ee2d8` — "db: finish PG dialect parity and fix the SQLite
regressions it introduced" (30 files: 21 db modules, 9 test suites).

**Context.** The parity session's handoff claimed both gates green. An
independent re-run of every gate on the working tree disproved the SQLite
claim (1076 passed / 11 failed) and found clippy non-clean; the PG claim
held (381 passed / 0 failed). All 11 failures were regressions introduced
by the parity pass itself, in three classes:

1. **PG syntax in SQLite arms** — `exports::find_export` /
   `find_export_for` had `EXPORT_COLUMNS_PG` in the SQLite arm and
   `imports::job_for_import` carried `job_id::text` there; SQLite rejected
   each with `unrecognized token: ":"` (m6 ×4, m7 ×6 minus one overlap).
   Found by a balanced-paren scan over every `db.sql()` / `sql_owned()`
   call flagging `::` or a `_PG` constant in the SQLite argument.
2. **`\`-continuation glue** — the rewritten SQLite counter upsert glued
   `1` + `RETURNING` into `1RETURNING` (m15 `usage_counters…`); the PG
   twin survived only via hand-added trailing spaces. Both counter
   upserts (economy + admin) rewritten as honest multi-line literals on
   both dialects.
3. **clippy** — two `useless use of format!` warnings in `secrets.rs`;
   fixed by introducing `COLUMNS_PG` (matching the exports.rs
   convention) instead of `.to_string()`.

Also restored the live `?::uuid` case to the `rewrite_placeholders` unit
test that the sweep had overwritten (`cargo test -p lorehaven-db --lib` →
27 passed).

**Final gate evidence, run on the committed tree:**

| Gate | Command | Result |
|---|---|---|
| PG | `LOREHAVEN_TEST_PG_URL=… cargo test -p lorehaven-app --no-fail-fast -- --test-threads=4` | **381 passed / 0 failed** (24 binaries) |
| SQLite | `unset LOREHAVEN_TEST_PG_URL && CARGO_TARGET_DIR=~/.cargo-target/lorehaven cargo test --workspace --no-fail-fast` | **1087 passed / 0 failed** (42 binaries) |
| db lib | `cargo test -p lorehaven-db --lib` | 27 passed / 0 failed |
| fmt | `cargo fmt --all -- --check` | clean |
| clippy | `cargo clippy -p lorehaven-db --all-targets` | 0 warnings |
| FE | `fe.sh check` + `fe.sh test` | svelte-check 0/0, vitest 147/147 |

**Environment notes.** 149 orphaned `lh_test_*` scratch databases were
dropped before the PG run (panicking tests never reach `cleanup()`; the
harness sweep is still owed). Scratch PG: container `lh-review-pg` on
55432, URL from `~/.config/lorehaven/pg-env` via
`~/.hermes/plans/lhpg-env.sh`.

**Known limitations, on record.** The `col::text = ?` read conversion
defeats PG index usage on UUID PK lookups (correct, not fast); accepted
for now — there is no production deployment (verified: no service, unit,
container, cron or config on the ThinkCentre), so the inversion decision
is deferred to deployment planning. Follow-up plan:
`~/.hermes/plans/2026-09-16-lorehaven-pg-parity-followup.md` (consistency
stragglers, standing checks, ADR).

## 2026-09-15 (afternoon) — review pass over the 08:45–13:43 work

Scope: the 14 commits `999fe4c..01a6bcf` (notifications backend, PG dialect
fixes, forum pages, pricing UI, Playwright e2e, backend-aware harness) plus
the work session's gate-metrics commits. Everything below was re-run by the
review, not taken from session prose.

### Review fixes (committed by the review)

- `ci.yml`: the golden-path journey step carried literal `***` passwords
  (redaction leaked into the committed file — those jobs could never
  authenticate), and the push trigger listed `main` while the branch is
  `master`. Fixed both.
- `scripts/postgres-journey.sh`: the baked-in scratch-container credentials
  stopped working (the container's data dir was re-initialized, so neither
  the script's nor the container env's password matched). The script now
  requires `DATABASE_URL` and `PSQL` instead of failing mysteriously;
  CI passes both explicitly.
- `docs/requirements.csv`: 9 rows (M10-01..05, M11-05, M12-01, M12-02,
  M21-05) had been rewritten with `requirement="Done"`, `milestone=<date>`,
  `status="3/3"`, destroying the requirement text and the ledger schema.
  Restored from the pre-image; kept the honest evidence notes the rewrites
  had added; dropped M12-02's stale "Trust gate still TODO" tail (the gate
  is implemented and tested: `a_trust_gate_rejects_underleveled_posters`).
- `notifications` mark-read: a non-uuid path id returned 204 on SQLite but
  500 on PostgreSQL (`$2::uuid` cast). Now parsed first → 404 on both
  dialects.
- `Community.svelte`: `role="tablist"` moved from `<nav>` to a `<div>`
  (svelte-check a11y warning, pre-existing).

### Gates (all re-run on the reviewed tree + review fixes)

- `cargo fmt --all -- --check` ✅
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` ✅
- `cargo test --workspace` (SQLite) ✅ 1087 passed / 0 failed, 37 binaries
- frontend: `fe.sh test` ✅ 147 tests / 22 files; `fe.sh check` ✅ 0 errors,
  0 warnings
- `scripts/postgres-journey.sh` against live PostgreSQL ✅ 67 steps, 0 failed
- Playwright e2e (`fe.sh e2e`, real Chromium, scratch SQLite) ✅ 2 passed

### True PostgreSQL milestone matrix (backend-aware harness, `--no-fail-fast`)

`LOREHAVEN_TEST_PG_URL=<admin url> cargo test -p lorehaven-app --no-fail-fast`
against the live PG 17 container:

- 23 targets: **333 passed / 48 failed**
- fully green (7): app lib unittests, milestone_0, milestone_3 (auth),
  milestone_10 (search), milestone_11 (discovery), milestone_14 (events)
- partial (16): milestone_12 (8/9), milestone_13 (8/1), milestone_15 (4/2),
  milestone_16 (5/1), milestone_17 (3/3), milestone_18 (5/2),
  milestone_19 (3/2), milestone_2 (26/1), milestone_21 (22/1),
  milestone_4 (18/2), milestone_5 (19/3), milestone_6 (27/7),
  milestone_7 (4/6), milestone_8 (9/1), milestone_9 (5/2),
  revision_cache (5/5)
- fully red: none — every suite makes progress on PG

The earlier claim "core PG modules verified green: M2, M10, M11, M21" was
true for M10/M11 but optimistic for M2 and M21 (one failure each — M2's is
the documented rate-limit flake, which passes in isolation, re-verified:
27/27). The drift list below replaces the previous one, which named
milestone_14 (now green) and missed milestone_2/5/12/21.

### Known PG drift (remaining 48 failures, by defect class)

Plan file: `~/.hermes/plans/2026-09-15-lorehaven-pg-parity.md`. Classes:

- community.rs comment/conversation/message/mute queries — 500s
  (milestone_12, 9 tests) — the casts scoped in the work session, not yet
  landed
- `ON CONFLICT DO UPDATE SET count = count + 1` — `count` ambiguous on PG
  (42702) in abuse/usage counter upserts (milestone_15, milestone_19)
- translation `shared`/`case_sensitive` INTEGER-vs-BOOLEAN (42804)
  (milestone_17)
- exports uuid decode — String vs UUID (milestone_7, milestone_6)
- single-test 500s in the same cast class (milestone_4 notes, milestone_13
  challenge enter, milestone_16 listings, milestone_9 taxonomy)
- milestone_5 job cancel/checkpoint semantics on PG (needs investigation,
  not just casts)
- milestone_18 bot/api-scope db path reaches `sqlite_pool()` under PG
  (code defect)
- test-side: milestone_12 category seed 42601; milestone_21 direct
  `sqlite_pool()` in a test
- milestone_2 rate-limit test: parallel-load flake (green in isolation)

## 2026-09-15 (morning) — backend-aware harness + core PG fixes

What the work session landed (verified by the afternoon review above):

- **test-support crate** (`crates/test-support/`) — backend-aware test
  harness (`TestDb`); all 19 milestone test files + `revision_cache`
  converted. Setting `LOREHAVEN_TEST_PG_URL` runs the same suite on
  PostgreSQL with a fresh scratch database per test.
- **PG dialect fixes** in migrations 0011/0012/0013 (uuid/bigint/boolean
  columns), `search.rs` `$1` double-bind, `ast_search.rs` uuid casts,
  `discovery.rs` casts + bigint decode, `community.rs` presence boolean.
- **m2 rate-limit test** — flaky under parallel load (shared loopback +
  global limiter); passes in isolation on both backends.
- **Dogfood pass** (docs/dogfood-2026-09-15.md): nine findings, eight fixed
  with tests — notifications had no backend at all, pricing accepted any
  caller (security), money ledger wrote a fake "platform" account, forum
  category links 404'd, PG dialect drift across 0021/0022/monetization,
  discovery rendered raw uuids, forum authors rendered as uuids, no author
  pricing UI; service-worker staleness documented for operators.

## 2026-09-16 (afternoon) — M23 Media query engine and API doors

What was implemented and verified:

- **R2 — Media query engine** (`crates/db/src/media.rs`): Complete rewrite of `find_media`, `list_media_filtered`, `list_creators`, `list_distributors`, `list_collections`, and write doors (`post_media_query`, `post_creator`, `post_distributor`, `post_media_collection`, `patch_creator`, `put_media_collection`). `MediaRecord` extended with `owning_account_id`. Cursor-based pagination added to `list_media_filtered`.
- **R3 — Remaining write doors**: Replaced `patch_creator` and `put_media_collection` 501 stubs with real update functions. `canon_media` and `space_media` kept as 501 (no corresponding schema tables yet).
- **R4 — Eligibility checks**: All read doors now use `MaybeSession` to get `account_id` from session. Public visibility always accessible; restricted/private only visible to the owning account. All write doors now use `RequireSession`.
- **R5 — ETag/304**: `get_media` now returns `ETag` header with `version` and serves `304 Not Modified` on matching `If-None-Match`.
- **R5 — Atom/RSS**: Added `GET /api/v1/media/feed` endpoint returning Atom XML.
- **Contract tests updated**: `milestone_22.rs` updated — implemented write doors removed from 501 test; read doors test still asserts 501 for `canon_media` and `space_media`.

Gates: SQLite 110P, PG 110P, clippy 0, fmt clean.

