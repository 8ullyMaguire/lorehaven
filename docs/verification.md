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
| Commit | tag `v0.03-identity` (Milestone 2, the name `docs/tutorial/README.md` reserves for it). The previous checkpoint was `v0.01-running-app` (`0d51c19`). |
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

## Milestone 2 — Accounts, pseuds, privacy and age policy

| # | Acceptance criterion | Status | Evidence |
|---|---|---|---|
| 1 | Registration and login work | Implemented and locally tested | `crates/app/tests/milestone_2.rs`; driven through a real browser: filled the form, the shell routed to `/account` and the header changed to the new pseud |
| 2 | Sessions are opaque, server-managed and cookie-borne | Implemented and locally tested | `crates/app/src/auth.rs`, `crates/db/src/sessions.rs`. Tokens exist only as SHA-256 hashes in the database; `HttpOnly`, `SameSite=Lax` and the cookie flags are asserted |
| 3 | CSRF is required for cookie-authenticated writes | Implemented and locally tested | Automated in `milestone_2.rs`; also observed against the running server: `POST /api/v1/pseuds` without the header → `403 ACCESS_DENIED`, with it → `201` |
| 4 | An account cannot touch another account's pseud | Implemented and locally tested | `an_account_cannot_edit_another_accounts_pseud` |
| 5 | Pseud linkage is absent from public responses | Implemented and locally tested | `pseud_linkage_is_absent_from_every_response`; the public profile returns no owner, no version and no session |
| 6 | A hidden pseud is reported as absent | Implemented and locally tested | `a_hidden_pseud_is_not_publicly_visible`; confirmed in the browser — a listed profile renders, a hidden one says "No pseud called @quill" |
| 7 | Session revocation takes effect | Implemented and locally tested | `sessions_are_listed_and_individually_revocable`. By hand: revoked a second session from the account page, then `curl` with its cookie returned `401` |
| 8 | A device's pseud choice does not change another device's | Implemented and locally tested | `activating_a_pseud_is_per_session`; the active pseud lives on the session row |
| 9 | Stale edits are refused rather than overwriting | Implemented and locally tested | `editing_a_pseud_with_a_stale_version_is_a_conflict` |
| 10 | Minor-protective defaults are stored from onboarding | Implemented and locally tested | `onboarding_stores_the_protective_defaults`, `a_minor_gets_stored_protective_defaults_and_no_public_identity` |
| 11 | A preference cannot exceed the policy ceiling | Implemented and locally tested | `content_settings_never_exceed_the_policy_ceiling`, `a_minor_cannot_widen_past_the_policy_ceiling` |
| 12 | Login and reset do not reveal whether an address has an account | Implemented and locally tested | `login_accepts_the_right_password_and_refuses_the_wrong_one`, `password_reset_is_indistinguishable_for_unknown_addresses` (including timing: a dummy hash is verified when there is no account) |
| 13 | A reset token is single-use and ends every session | Implemented and locally tested | `a_reset_token_is_single_use_and_ends_every_session`, `requesting_a_second_reset_invalidates_the_first_link` |
| 14 | Repeated attempts are rate limited | Implemented and locally tested | `repeated_login_attempts_are_rate_limited`, plus the fail-closed case when a route declares no class |
| 15 | No response or log contains a credential | Implemented and locally tested | `no_response_contains_a_credential`; the module note above records that production logs the reset *request* and not the token |
| 16 | The pages exist and use the real endpoints | Implemented and locally tested | `frontend/src/routes/*.test.ts` and the browser journey below; no page shows mock data |

Commands actually run, with their result:

```text
cargo test --workspace                          163 passed, 0 failed
cargo clippy --all-targets --all-features       clean, with -D warnings
cargo fmt --all -- --check                      clean
node node_modules/vitest/vitest.mjs run         57 passed (10 files)
node node_modules/vite/bin/vite.js build        160 modules; 103.6 kB JS (36.8 kB gzip), 28.5 kB CSS
lorehaven migrate                               applied 0001_identity, 0002_sessions_and_settings
lorehaven doctor                                17 checks: 0 failing, 1 warning
lorehaven serve --port 8123                     driven in a real browser (see below)
```

The browser journey, against the compiled binary serving its embedded bundle and
a fresh SQLite file at `/tmp/lh-e2e`:

```text
/                         header offers Sign in and Register while anonymous
/register                 filled and submitted; routed to /account; header became "Writing as @quill"
/account (Sessions)       the live session listed as "Chrome on Linux · This device" with real timestamps
/account (Reading)        ceiling reported from the policy; changed the preference to Mature, saved,
                          reloaded the page and the value came back as mature
/account (Privacy)        four account-scoped keys rendered from the server's own schema; changed
                          messaging_policy and saved
/pseud                    created a second pseud (@inkwell), switched to it; the header followed, and
                          the pseud-scoped privacy panel showed exactly the two pseud-level keys
/pseud/inkwell            the public profile, with the real bio and creation time
/pseud/nobody-here        "No pseud called @nobody-here"
Edit a pseud              changed the display name and set discoverability to Hidden; the list reflected both
/pseud/quill (hidden)     reported as absent, exactly like a pseud that never existed
/sign out                 header returned to Sign in and Register; /account asked for a session
/sign in                  a wrong password produced the same non-committal answer as an unknown address
/password-reset           the token step appeared, prefilled, with the reason it is visible
```

Two defects were found by this journey and fixed before committing:

1. **The shell kept claiming a session after a password reset.** Redeeming a
   reset token revokes every session on the account, but the header still said
   "Writing as @quill". The reset page now refreshes the session store, and
   `frontend/src/routes/PasswordReset.test.ts` pins it.
2. **A failed sign-out threw.** The store cleared local state and then re-threw
   the network error, which the caller could do nothing with; it now records the
   failure so the page can say the session may still be live on the server.

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
   work-card variants. Eighteen of the listed primitives are implemented.
6. **Template type-checking is not part of the gate.** `svelte-check` cannot
   start in this checkout — it reports "No Svelte configuration found in vite
   config" for every component, including the ones that predate Milestone 2 — so
   what actually protects the frontend is the Vite build (which compiles every
   template) and the Vitest suite. A mistake that only a type-checker would
   catch, such as a prop of the wrong shape, would not be caught here.
7. **There is no email transport.** A password reset returns its token in the
   response outside production and logs only the request inside it. That is
   honest — the flow cannot silently pretend a message was sent — but it is not
   a shipped flow. SMTP is spec §2.2 optional infrastructure and has not been
   chosen.
8. **The account and pseud pages have no automated browser coverage.** They were
   driven by hand (the journey above); Vitest covers the store, the router, the
   API client and two of the pages. A Playwright suite is still missing
   (spec §23).
9. **Two tabs can still collide on the settings forms.** The server refuses a
   stale version and the interface shows the conflict with a reload, but the
   loser of that race has to re-apply their change by hand.

## What was *not* done, stated plainly

Milestones 3 through 18 are **not implemented**. `docs/requirements.csv` records
each as `unsupported`. Within Milestone 2, block and mute primitives are still
tables with no behaviour (M2-06). No screen in the application displays mock
data: the pages that exist show real values from the server, and the routes that
are linked but unbuilt render an explicit "not built yet" panel naming the
milestone that will fill them.
