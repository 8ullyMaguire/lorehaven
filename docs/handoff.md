# Handoff — M32/M33 frontend complete, E2E 70/73 passing

Date: 2026-09-22. Read with `docs/plans/junior-implementation-plan.md` and `docs/spec.md`.

## What just happened

M32/M33 frontend is built and deployed to production. E2E test suite improved
from 60 passed / 13 failed → 70 passed / 3 failing (subscription test fix just
deployed, awaiting final rerun).

### Commits (newest first)

- `b8fbbba` fix: E2E wait for Topics heading before filling form
- `dcb6d87` fix: E2E navigate directly to category URL for subscription test
- `214c03e` fix: E2E navigate to category page for topic creation
- `f6a40ea` fix: E2E topic-title ID, work page assertions, docs sections
- `8141054` feat: M32/M33 frontend — ForumVoteBar, VoteBudget, KarmaBadge + typed-vote API
- `7ee7ffd` M2 rate-limit fix + widen all test rate limits; fix M6 AO3 wall + M7 retention default
- `e660da1` docs: update handoff — M34/M39/M40/M41/M42 complete
- `72ac970` M15/ADR 0020: redistribution floor
- `457a4e7` M34: spoilers, warnings, readability — 7 tests + account_id fix
- `449f5c9` M41: longevity signals — half-life scoring + warmth tiers (4 tests)
- `14dba1a` M40: fork with provenance and permission statements (9 tests)
- `c743123` M42: export CTAs with curator quorum exemption
- `53c1318` M31: work-linked threads and reaction bar
- `6c23e3f` M32: typed votes with budgets and meta-moderation
- `43f7ed9` M33: thread modes (AMA, reading group, critique, wiki pin, collab, prompt, character voice)
- (M35 discovery/health — committed earlier)

## State

### Complete with passing tests (all green)

| File | Tests | Notes |
|------|-------|-------|
| lib.rs | 143 | core library |
| milestone_0 | 11 | boot/health |
| milestone_2 | 27 | auth + **rate-limit tests fixed** |
| milestone_3 | 15 | accounts/pseuds |
| milestone_4 | 19 | content CRUD |
| milestone_5 | 9 | search |
| milestone_6 | 6 | imports (AO3 fingerprint wall fixed) |
| milestone_7 | 7 | exports (retention default fixed) |
| milestone_8 | 7 | discovery |
| milestone_9 | 7 | library |
| milestone_10 | 6 | reading |
| milestone_11 | 6 | reactions |
| milestone_12 | 18 | forum base |
| milestone_13 | 9 | forum posts/replies |
| milestone_14 | 9 | forum votes |
| milestone_15 | 6 | forum moderation |
| milestone_16 | 7 | work discussion (M31) |
| milestone_17 | 6 | forum subscriptions |
| milestone_18 | 7 | forum search |
| milestone_19 | 7 | block enforcement |
| milestone_21 | 11 | TTS |
| milestone_22 | 16 | CTA + exports |
| milestone_24 | 11 | narration |
| milestone_25 | 9 | bulk export |
| milestone_26 | 12 | resource directory |
| milestone_31 | 6 | work discussion modes |
| milestone_32 | 8 | typed votes, budgets |
| milestone_33 | 5 | thread modes |
| milestone_34 | 7 | spoilers, warnings, readability |
| milestone_35 | 5 | discovery, health, UX |
| milestone_39 | 6 | resource directory |
| milestone_40 | 9 | fork/provenance |
| milestone_41 | 4 | longevity signals |
| **domain lib** | **358** | unit tests |

**Total: ~500+ backend tests passing, zero failures, zero warnings.**

### Frontend components (M32/M33)
- `ForumVoteBar.svelte` — per-post typed vote bar with counts, budget badge, transparency-aware name list
- `VoteBudget.svelte` — rolling-window budget display, compact + full modes
- `KarmaBadge.svelte` — pseud karma display (★ score)
- `ReactionBar.svelte` — work-level reactions (work page)
- `ThreadModePicker.svelte` — mode selector for topics (plain, reading_group, critique_circle, wiki_pin, prompt)
- Updated `api.ts` with typed-vote API functions and types

### E2E (Playwright on thinkcentre)
- **70 passing, 3 failing** (in progress — subscription test fix just deployed)
- Remaining failures (pre-existing, not from M32/M33 work):
  1. `exports: queue an EPUB, the worker makes it, and the download is a real EPUB` — 1m timeout
  2. `exports: a finished export can be deleted (forgotten)` — 25s timeout
  3. `forum: a signed-in user subscribes to a topic and sees the unread count` — fix deployed, awaiting rerun

### Deployed
- thinkcentre `127.0.0.1:8081` — updated with M32/M33 frontend (hash `8141054`)
- Docs system live: 7 help pages + Ctrl+K search
- Pawchive imports: 143 works completed, 10,136 total in library

### Known gaps (not stubs, intentionally deferred)

| Gap | Status | Plan ref |
|-----|--------|----------|
| Scraper bot adaptation | Explored, not ported | spec §37, plan §15c |
| Obscura integration (CF-protected sites) | Not started | — |
| 40k rescrape of failed links | Not started | — |
| Webnovel-scraper port (novelfull, readlightnovel, novelupdate) | Awaiting scope decision | — |

## Environment quirks (unchanged)

- **Work in local clone** `~/code-local/rust/lorehaven`. `~/code/rust/lorehaven` is SSHFS — never run git/cargo/npm through it.
- **Daily sync**: `lorehaven-sync.timer` enabled, 09:00.
- Playwright E2E runs ON thinkcentre over SSH (`frontend/e2e/serve-scratch.sh`).
- Deployed instance: thinkcentre `127.0.0.1:8081`, admin credentials in `~/.hermes/.env`.
- Lint false positive: the write_file/patch tool's linter runs rustc with Rust 2015 edition and reports `async fn` errors — ignore those; `cargo check` is the real gate.
- **Rate limiter buckets are process-global** (`GLOBAL_BUCKETS` in `limiter.rs`), keyed by IP. Tests that assert limiter refusal configure their own tight limits; all other suites use the widened test defaults.
- Build on thinkcentre: `pkill -9 cargo` first; binary swap needs `pkill -9 -f "lorehaven serve"`.
- Argon2 params: m_cost=19456, t_cost=2, p_cost=1.
- **Frontend builds need to run ON thinkcentre** — the embedded bundle (`frontend/dist/`) is built into the binary with `rust-embed`. Local `vite build` updates the local copy but thinkcentre needs its own build.

## Gotchas

- Spoilers routes passed `pseud_id` where DB FK'd `accounts(id)` — fixed with `RequirePseud { user, .. }` → `user.account_id`.
- Config `rate_limits` field has no top-level `burst`/`per_minute` — nested in each `Quota`.
- `_let` typo in `milestone_2.rs` broke compile — fixed.
- Comment POST returns **200** with `{id, receipt}`, not 201.
- `forum_categories` has no repo-level `create_category`; tests seed via raw SQL.
- **NewTopicForm** input ID must be `#topic-title` (tests expect this).
- **Work page** doesn't show full chapter text — it shows chapter titles in a list; clicking opens the reader.
- **Docs pages**: sections appear in both body and TOC, so `getByText('...')` may match multiple elements — use `.first()`.
- **Community category navigation**: direct URL `/community/forums/<id>` works but requires the page to load (wait for `Topics` heading).
- Package-lock.json can be reformatted by `npm install` — git checkout to reset if needed.

## How to resume

1. Verify E2E subscription test result (just deployed fix — may already be green)
2. Fix the 2 export E2E failures (worker timing, download verification)
3. Once all E2E green, decide next scope with user:
   - **A.** Tag release (`v1.0.0`) and deploy to production
   - **B.** Scraper bot adaptation or Obscura integration
   - **C.** 40k rescrape of failed links
   - **D.** Webnovel-scraper port
