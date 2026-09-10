# Verification log

This file is the honest record of what has been **executed**, not what has been
written. Spec §1.2 requires the distinction, and the reason is practical: a
skeleton that compiles is not a feature, and a claim that has not been run is a
claim that will be discovered as false at the worst moment.

Statuses used (spec §1.2):

| Status | Meaning |
|---|---|
| Implemented and locally tested | The behaviour runs and an automated test asserts it. |
| Implemented and fixture tested | Runs against stored fixtures rather than a live external system. |
| Implemented but not executed | Code exists; the path has never been run. |
| External integration not live verified | Depends on a third party that has not been contacted. |
| Partially implemented | Some of the described behaviour exists. |
| Unsupported | Not implemented. |

Method for every entry below: run the command in the Evidence column and read
the result. Where a claim could only be checked by hand, it says so.

---

## Repository state

| Field | Value |
|---|---|
| Date of last verification | 2026-09-10 |
| Commit | `v0.01-running-app` (`0d51c19`); the running binary reports `0.1.0+0d51c19` at `/health/live` |
| Environment | Linux, Rust 1.98.0, Node 26.8.1, SQLite 3.53.4 |
| PostgreSQL available | **No** |

---

## Milestone 0 — Repository, tooling and running application

| # | Acceptance criterion | Status | Evidence |
|---|---|---|---|
| 1 | A clean checkout builds | Implemented and locally tested | `cargo build` → clean, no errors |
| 2 | The application starts with SQLite | Implemented and locally tested | `crates/app/tests/milestone_0.rs`; also run by hand: `lorehaven serve` on port 8099 answered `/health/live` with `{"status":"ok"}` |
| 3 | The application starts with PostgreSQL | **Implemented but not executed** | Dialect SQL, migrations and a `postgres://` connection path exist. No PostgreSQL server is installed on the development machine, so **this has never been run**. This is the largest open risk in the project. See ADR 0004. |
| 4 | A frontend page loads from the Rust executable | Implemented and locally tested | Automated: `the_frontend_shell_is_served_from_the_binary`. By hand: loaded `http://127.0.0.1:8099/` in a real browser; the Svelte bundle executed, rendered the navigation and hero, and made its own calls to `/api/v1/meta` and `/health/ready`, whose results appear on the page. No CSP violation blocked the bundle. |
| 5 | `/health/live` checks process liveness | Implemented and locally tested | `crates/app/tests/milestone_0.rs`; by hand `curl -i /health/live` → `200`, body carries version, build, environment, uptime |
| 6 | `/health/ready` checks essential dependencies | Implemented and locally tested | Automated, including the negative case: an unmigrated database returns `503` and names the failing check. By hand it returned `{"status":"ready", …}` with database, migrations and storage all `ok`. |
| 7 | Production startup rejects unsafe development configuration | Implemented and locally tested | `crates/app/src/safety.rs` unit tests plus `production_startup_is_refused_with_development_settings`. By hand: production defaults produce five fatal findings naming the fix for each. |

Commands actually run, with their result:

```text
cargo build                                        # Finished, no errors
cargo test                                         # 91 passed, 0 failed
lorehaven doctor                                   # 17 checks, 1 failing (pending migrations) — correct
lorehaven migrate --status                         # 0 applied, 1 pending
lorehaven migrate                                  # applied 0001_identity
lorehaven seed --development                       # account + 2 pseuds created
lorehaven seed --development                       # second run: same account id, still 1 account / 2 pseuds
lorehaven serve --port 8099                        # listening; see endpoint results below
```

Endpoint results, verbatim:

```text
GET /health/live   → 200 {"status":"ok","version":"0.1.0","environment":"development", …}
GET /health/ready  → 200 {"status":"ready","checks":{"database":{"ok":true},"migrations":{"ok":true},"storage":{"ok":true}}}
GET /api/v1/meta   → 200 {"name":"Lorehaven","api_version":"v1","policy":{ … }}
GET /                → 200 (the SPA shell)
GET /assets/nope.js  → 404
```

Graceful shutdown was observed by sending SIGTERM: the log records
`received terminate` followed by `shutdown complete`.

## Milestone 1 — Design system and navigation

| # | Acceptance criterion | Status | Evidence |
|---|---|---|---|
| 1 | All controls work with a keyboard | Implemented and locally tested | `frontend/src/lib/components/Dialog.test.ts` (Escape, close control, unrelated keys). Tabs implement the WAI-ARIA roving-tabindex pattern. |
| 2 | Focus is visible | Implemented and locally tested | `:focus-visible` outline on every interactive element via `app.css`; components that draw their own focus treatment (inputs, buttons) override the border, not the outline |
| 3 | Dialog focus is trapped and restored | Implemented and locally tested | `Dialog.test.ts` covers focus entry; restoration runs in the effect cleanup. **Not yet verified by hand** with a screen reader. |
| 4 | Layout works at 320 CSS pixels | **Implemented but not executed** | Responsive rules exist and the mobile navigation switches at 48rem, but no measurement at 320 px has been taken. |
| 5 | Pages remain usable at 200% zoom | **Implemented but not executed** | Relative units are used throughout; not measured. |
| 6 | Reduced-motion preference is respected | Implemented and locally tested | Global override in `app.css` plus per-component overrides for the spinner, shimmer and progress bar |
| 7 | Errors are announced accessibly | Implemented and locally tested | `ErrorSummary` and `TextField`/`Select` errors use `role="alert"`; `ToastRegion` is a polite live region |

Frontend commands actually run:

```text
node node_modules/vitest/vitest.mjs run   # 5 files, 30 tests passed
node node_modules/vite/bin/vite.js build  # 133 modules; 65.7 kB JS (25.3 kB gzip), 17.2 kB CSS
```

The suite includes a shell-level test (`frontend/src/App.test.ts`) that renders
the application against mocked API responses and changes the appearance control.
It exists because of a real defect found while testing by hand: the selector used
`bind:value` *and* `onchange`, so persistence depended on which listener Svelte
attached first, and a chosen theme was applied for the session but never
remembered. The fix passes the value explicitly through one handler, and the
test now pins it.

Appearance persistence was also confirmed by hand in the browser: choosing Clear
Day stored `lorehaven.theme=clear-day`, and a reload came back with
`data-theme="clear-day"` and a white page background.

Note on this machine: the checkout lives on an NFS-mounted share where
`node_modules/.bin` symlinks cannot be executed (`Operation not permitted`), so
`npm run build` / `npm test` fail here. The underlying tools run correctly when
invoked through `node`, as shown above. CI on a normal filesystem uses the
ordinary `npm` scripts.

---

## Known limitations and open risks

1. **PostgreSQL has never been executed.** Every PostgreSQL statement is
   written but unverified. A CI job with a `postgres` service must run the same
   integration suite before PostgreSQL can be called supported.
2. **No browser-automation suite.** Milestone 1's accessibility criteria were
   checked with unit tests and one manual browser session, not with Playwright.
   Spec §23 asks for automated browser journeys; they do not exist yet.
3. **The command line panics on a closed pipe.** `lorehaven migrate | head`
   produces a broken-pipe panic, because Rust ignores `SIGPIPE` by default.
   It is cosmetic — the work completes — but it is a wart. Fixing it means
   restoring the default `SIGPIPE` disposition, which needs `unsafe`, and the
   workspace forbids `unsafe_code`; the trade-off has not been made yet.
4. **Fonts are not self-hosted.** `tokens.css` names Source Sans 3, Source Serif
   4 and Literata with system fallbacks, but no subset font files are shipped,
   so the intended typography only appears where those fonts are installed
   locally. The theme document asks for self-hosted subsets.
5. **Milestone 1 is incomplete against its own list.** Of the listed reusable
   components, the following do not exist yet: combobox, and the richer
   work-card variants. Fifteen of the listed primitives are implemented.
6. **No rate limiting.** The middleware stack has request IDs, security
   headers, CORS, timeouts and a body limit, but no rate limiter. Nothing is
   exposed publicly yet, so nothing is currently abusable, but this must land
   before any write endpoint does.

## What was *not* done, stated plainly

Milestones 2 through 18 are **not implemented**. `docs/requirements.csv`
records each as `unsupported`. No screen in the application displays mock data:
the pages that exist show real values from the server, and the routes that are
linked but unbuilt render an explicit "not built yet" panel naming the milestone
that will fill them.
