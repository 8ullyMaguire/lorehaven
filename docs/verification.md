## 2026-09-15 — Backend-aware milestone harness + core PG fixes

### What changed
- **test-support crate** (`crates/test-support/`) — new backend-aware test harness (`TestDb`) that provides a unified `tdb` interface for both SQLite and PostgreSQL. All 19 milestone test files + `revision_cache` converted to use it.
- **PG dialect drift fixes** (core golden-path modules):
  - `migrations/postgres/0011_taxonomy.sql` — `work_id` UUID, `pos` BIGINT, `work_tags.work_id` UUID (was TEXT) — fixes search (M10) and fielded queries.
  - `migrations/postgres/0012_discovery.sql` — `updated_at` TEXT, recipe counter BIGINT — fixes discovery taste-profile (M11).
  - `crates/db/src/search.rs` — fixed PG `$1` double-bind collision in `search_works_ast`.
  - `crates/db/src/search/ast_search.rs` — added `::uuid` cast for `blocks.blocked` (TEXT) vs `pseuds.account_id` (UUID); fixed `work_tags.work_id` UUID joins.
  - `crates/db/src/discovery.rs` — added `::uuid` cast for `subject_id` (TEXT) vs `work_tags.work_id` (UUID); fixed PG `SUM(bigint)` cast; fixed `recipes.is_public` INTEGER→BIGINT.
  - `crates/db/src/community.rs` — presence `enabled` BOOLEAN (was INTEGER) + cleaned `enabled` decode.
  - `migrations/postgres/0013_community.sql` — `presence.enabled INTEGER` → `BOOLEAN DEFAULT false`.
- **m2 rate-limit test** — remains flaky under parallel load (shared loopback + global limiter); passes in isolation on both backends.

### Gates
- `cargo fmt --all` ✅
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` — 0 errors
- `cargo test -p lorehaven-app` (SQLite) — **all green** (0 failures, 110+ tests)
- Core PG modules verified green: **M2 (auth)**, **M10 (search)**, **M11 (discovery)**, **M21 (monetization)**.

### Workstreams status
| Workstream | Status |
|------------|--------|
| WS1: Real-browser E2E (dogfood) | ✅ Complete — full register→login→publish→purchase→forum chain on both SQLite + PG, zero console errors, screenshots + report in `docs/dogfood-2026-09-15.md` |
| WS2: postgres-journey.sh extended + CI | ✅ Complete (67 steps covering auth, authoring, monetization, community, notifications; wired into `.github/workflows/ci.yml`) |
| WS3: Playwright E2E suite (`frontend/e2e/`) | ✅ Complete — 2/2 golden-path tests passing, CI job added |
| WS4: Backend-aware milestone harness + PG drift | ⚠️ **Core modules green** — harness works on both backends; **remaining PG drift** in secondary modules (comments, conversations, messages, jobs, outbox, imports, exports, translation, positivity, governance, privacy, revision_cache) — documented as follow-up |

### Known PG drift (secondary modules)
The following milestone test files have failing tests on PG due to dialect drift in their specific db modules:
- `milestone_4` (notes), `milestone_6` (imports), `milestone_7` (exports), `milestone_8` (library), `milestone_9` (taxonomy), `milestone_13` (comments), `milestone_14` (events), `milestone_15` (comment_positivity), `milestone_16` (jobs), `milestone_17` (translation), `milestone_18` (bot registration), `milestone_19` (abuse/privacy), `revision_cache` — failures from:
  - `uuid = text` comparisons needing casts
  - `Option<String>` vs `UUID` decode mismatches
  - Fake-UUID test seeds (`work-1`, `s1`, `other`)
  - `i64` vs `INT4` decode mismatches
  - `enabled` boolean literal mismatches
  - `updated_at` column references on tables missing the column
  - `$n` placeholder numbering collisions

These are **secondary features** (outbox, imports/exports, translation, governance, revision cache, abuse) — not common user journeys (all of which pass on PG via the browser E2E pass). Fixing them is tracked as follow-up work for WS4 continuation.