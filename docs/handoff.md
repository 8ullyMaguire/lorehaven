# Handoff — All M-series green; deploy or next feature

Date: 2026-09-21. Read with `docs/plans/junior-implementation-plan.md` and `docs/spec.md`.

## What just happened

All forum milestones (M31–M35), the resource directory (M39), fork/provenance (M40),
longevity signals (M41), export CTAs (M42), and ADR 0020 redistribution floor are
committed and passing.

The **M2 rate-limit test failures are fixed** (`7ee7ffd`) — the two tests that
specifically assert limiter refusal (burst 10 / 30) now configure their own tight
limits instead of inheriting the widened test defaults. Warnings are also cleared
across the workspace.

### Commits (newest first)

- `7ee7ffd` M2 rate-limit fix + widen all test rate limits; fix M6 AO3 wall + M7 retention default; clear all warnings
- `e660da1` docs: update handoff — M34/M39/M40/M41/M42 complete, M2 rate-limit test fix owed
- `72ac970` M15/ADR 0020: redistribution floor — two pools, graduated cap, AI declaration, processor-agnostic billing
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

**Total: ~500+ tests passing, zero failures, zero warnings.**

### Deployed
- thinkcentre `127.0.0.1:8081` (systemd --user `lorehaven` service, `--with-worker`)
- Docs system live: 7 help pages + Ctrl+K search
- Pawchive imports: 143 works completed, 10,136 total in library

### Known gaps (not stubs, intentionally deferred)

| Gap | Status | Plan ref |
|-----|--------|----------|
| M32/M33 frontend (vote bars, karma badges, thread mode picker) | Backends fully tested; Svelte components not built | plan §15a.2–3 |
| M31 E2E | 20 Playwright tests committed, not re-run after frontend | `frontend/e2e/extended.spec.ts` |
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

## Gotchas

- Spoilers routes passed `pseud_id` where DB FK'd `accounts(id)` — fixed with `RequirePseud { user, .. }` → `user.account_id`.
- Config `rate_limits` field has no top-level `burst`/`per_minute` — nested in each `Quota`.
- `_let` typo in `milestone_2.rs` broke compile — fixed.
- Comment POST returns **200** with `{id, receipt}`, not 201.
- `forum_categories` has no repo-level `create_category`; tests seed via raw SQL.
- Axum layer order: `.layer()` adds outermost-last; limiter must be added FIRST (innermost).

## How to resume

All tests green, warnings cleared. Options for next work:

1. **Deploy/tag release** — build binary on thinkcentre, swap, verify live
2. **M32/M33 frontend** — vote bars, karma badges, thread mode picker (backends ready)
3. **M31 E2E verification** — run Playwright suite on thinkcentre
4. **Scraper bot / Obscura** — port fanfic-archivist or integrate CF bypass
