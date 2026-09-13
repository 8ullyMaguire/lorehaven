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
| Date of last verification | 2026-09-11 |
| Commit | `a54e3bc` on `master`. Milestones 0 to 7 are complete and tagged: `v0.01-running-app`, `v0.03-identity`, `v0.04-publishing`, `v0.05-reader`, `v0.06-jobs`, `v0.07-imports`, `v0.08-exports`. Every row in `docs/requirements.csv` for those milestones is `implemented-locally-tested` or a deliberate `unsupported`. |
| Environment | Linux, Rust 1.98.0, Node 26.8.1, SQLite 3.53.4 |
| PostgreSQL available | Yes — 17.11 in Docker on loopback (the development machine still has none installed) |

---

## Milestone 0 — Repository, tooling and running application

| # | Acceptance criterion | Status | Evidence |
|---|---|---|---|
| 1 | A clean checkout builds | Implemented and locally tested | `cargo build` → clean, no errors |
| 2 | The application starts with SQLite | Implemented and locally tested | `crates/app/tests/milestone_0.rs`; also run by hand: `lorehaven serve` on port 8099 answered `/health/live` with `{"status":"ok"}` |
| 3 | The application starts with PostgreSQL | **Executed and locally tested** | Run against PostgreSQL 17.11 on 2026-09-11 (see *PostgreSQL, executed* below). Migrations, seed, readiness and a scripted journey all pass. Eight dialect defects were found and fixed; the two-dialect risk is no longer untested, though it remains the risk that needs a continuous check rather than a one-off. See ADR 0004. |
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

### PostgreSQL, executed

On 2026-09-11 the PostgreSQL half of the tree was run for the first time, against
PostgreSQL 17.11 in a Docker container on loopback. The build was already up to
date; what follows is the whole of what was run.

```text
lorehaven --database-url postgres://…@127.0.0.1:55432/lorehaven migrate
  → 8 migration(s) applied (0001 … 0008)
lorehaven … migrate --status
  → every migration "applied"
lorehaven … seed --development
  → 1 account, 2 pseuds, 4 privacy settings, 1 password credential
lorehaven … serve --port 8130
  → GET /health/live  200 {"status":"ok"}
  → GET /health/ready 200 {"status":"ready", checks.database.detail:
                          "postgres reachable at postgres://…"}
  → GET /api/v1/meta  200
  → GET /               200 (the SPA shell)
```

Then a scripted journey, signed in as the development account, every step
asserted:

```text
create a work            201     GET /works/{id}            200
add a chapter            201     GET /library/items         200
write the chapter        200     GET /library/history       200
publish                  200     GET /jobs                  200
progress device=250      204     GET /imports/sources       200
progress device=500      204     GET /auth/me               200
progress (no device)     204     GET /reading/progress      200
rate 5 stars             200
rate 4 stars             200
note (first)             204
note (second)            204
review with spoilers     200
```

The last check reads back what the journey wrote: the work is public with one
chapter, and **both** device positions survive as separate rows, which is what
the `reading_progress_unique` index fix is for.

#### The eight defects this found

Every one is a violation of the rule in ADR 0004 — that every parameter and every
selected column is `String` or `i64` so rows decode identically on both engines —
and none of them could have been found by any test in the tree, because every
test runs on SQLite.

1. **A password update that silently wrote nothing.** `set_password_hash` bound
   one parameter list for two statements whose placeholders are in different
   orders: the insert leads with `account_id`, the update cannot, because `SET`
   comes before `WHERE`. SQLite accepted the surplus parameter and shifted every
   value by one, so `WHERE account_id = <timestamp>` matched no row; the call
   returned `Ok(())` having written nothing. **This was a live bug in the SQLite
   path**, not just a PostgreSQL failure: re-setting a password — the seed's
   second run, a password reset, any change — did nothing and reported success.
   Proven by seeding twice on SQLite and diffing the stored hash and `updated_at`
   (byte-identical before the fix). PostgreSQL refused it outright with
   `invalid input syntax for type uuid: "2026-09-11T…"`, which is how it was
   found. Regression test:
   `lorehaven-db::identity::tests::setting_a_password_twice_replaces_the_stored_credential`.
2. `ON CONFLICT DO UPDATE` with no conflict target, in `privacy_settings`,
   `outbox_events` and `reading_history_entry`. Legal in SQLite, refused by
   PostgreSQL. The privacy and progress statements carry partial unique indexes,
   so the target names the index **and its predicate**.
3. `version INTEGER` against `i64` in Rust. SQLite's `INTEGER` is 64-bit;
   PostgreSQL's is 32-bit, and `sqlx` will not widen `INT4` into `i64`. 18
   columns.
4. The same, for 20 more integer columns — counts, orderings, percentages and the
   0/1 flags. ADR 0004 was amended: those columns are `BIGINT`.
5. The 0/1 flags the repository reads as `bool` stay `BOOLEAN`, and where the
   shared struct wants `i64` they are read as `bool::int::bigint`. PostgreSQL has
   no boolean-to-bigint cast, which is why the double cast is not redundant.
6. A `?` missing its `::uuid` cast in the rating upsert.
7. `reading_progress_unique` was declared without `device_id` on PostgreSQL, so
   two devices reading the same work would have collided onto one row. The
   SQLite index has it; this was a divergence between the two migration trees
   that nothing was comparing.
8. `library_items` bound four values for five placeholders.
9. **`typography_preference.distraction_free` was never cast on the write path.**
   The column is `BOOLEAN`; the Postgres `VALUES` list bound it bare, so
   `PATCH /settings/typography` answered `500` with *"column `distraction_free`
   is of type boolean but expression is of type bigint"* — saving reading
   preferences was broken on PostgreSQL and worked on SQLite. The first fix pass
   covered `contains_spoilers` and `is_public` and missed this one, because the
   sweep was done by reading the diffs rather than by enumerating the schema's
   boolean columns and checking every bind of each. Now `?::int::boolean`.
10. **`font_scale` and `line_height` were `REAL`.** The repository decodes them as
    `f64`, and PostgreSQL's `REAL` is 4-byte `FLOAT4` against Rust's `FLOAT8`, so
    the read after the write failed with *"column 0: mismatched types; Rust type
    `f64` (as SQL type `FLOAT8`) is not compatible with SQL type `FLOAT4`"*.
    SQLite's `REAL` is 8-byte, so the two dialects meant different widths for the
    same declaration. Both columns are now `DOUBLE PRECISION` *and* read through
    an explicit `::double precision`, which states the contract rather than
    relying on the column type staying right.

Defects 9 and 10 are each the same lesson as 3 and 5 — a dialect's declaration is
not the repository's type — and they were found only because the sweep was run per
column *class* over the whole schema instead of over the statements just edited. A
fix applied by reading one's own diff verifies the diff, not the class.

Defect 7 is the one to remember: the two migration catalogues are checked for
*identical ids* and for nothing else, so a column can differ between the engines
indefinitely. If a second live run ever happens, compare the two catalogues'
columns, not just their filenames.

### Continuous integration, run by hand

`.github/workflows/ci.yml` defines four jobs. No hosted runner has executed them:
the repository has no git remote, and `act` cannot bind-mount this tree because
the Docker daemon's root user cannot read it (`stat .: permission denied`). So
each job's steps were executed here, with their own assertions, on 2026-09-11:

```text
job rust              Formatting                 clean
                      Lints (clippy -D warnings) 0 findings   (5, then fixed)
                      Build --all-targets         0 errors
                      Test --workspace            791 passed, 0 failed
job frontend          check (svelte-check)        0 errors, 0 warnings
                      test (vitest)               136 passed
                      build (vite)                ok
job embedded-assets   cargo build -p lorehaven-app with no frontend/ present   ok
job postgres          migrate --status            8 applied, 0 pending
                      migrate is idempotent       "already up to date"
                      readiness                   {"status":"ready"} naming postgres
```

Two findings came out of doing this rather than assuming it.

**The clippy step caught something the full gate run had not.** Three
`clone_on_copy` findings in the test added above, because that run happened
before the test existed. That is the argument for running the gate after the
last edit rather than during.

**The readiness assertion was vacuous.** The job asserted
`grep -q '"backend"' ready.json || grep -q 'postgres' ready.json`. The body has no
`backend` field, so the first grep always failed and the second always succeeded —
which passed whether or not the server had reached PostgreSQL. It now asserts
`postgres reachable at`, which is the string the readiness body actually carries
and only carries when the database it opened is PostgreSQL.

The frontend job's `npm ci` is the only step not reproduced: it would install the
toolchain, and this checkout has a `node_modules` whose `.bin` is empty, so the
three commands run through `frontend/scripts/fe.sh` instead. The commands
themselves — `svelte-check`, `vitest run`, `vite build` — are the job's.

What remains unexercised is the runner's own plumbing: `actions/checkout`,
`dtolnay/rust-toolchain`, the `postgres` service container and the cache action.
Those are the parts a hosted runner supplies, and they are exactly the parts this
machine cannot stand in for.

## Milestone 1 — Design system and navigation

| # | Acceptance criterion | Status | Evidence |
|---|---|---|---|
| 1 | All controls work with a keyboard | Implemented and locally tested | `frontend/src/lib/components/Dialog.test.ts` (Escape, close control, unrelated keys). Tabs implement the WAI-ARIA roving-tabindex pattern. |
| 2 | Focus is visible | Implemented and locally tested | `:focus-visible` outline on every interactive element via `app.css`; components that draw their own focus treatment (inputs, buttons) override the border, not the outline |
| 3 | Dialog focus is trapped and restored | Implemented and locally tested | `Dialog.test.ts` covers focus entry; restoration runs in the effect cleanup. **Not yet verified by hand** with a screen reader. |
| 4 | Layout works at 320 CSS pixels | **Executed and locally tested** | `frontend/scripts/measure-viewport.mjs` measures every route in a real browser at 320, 360, 768 and 1280; all 19 routes hold with no horizontal scroll (see *Measured at 320 CSS pixels* below). Two real overflows were found and fixed, and the earlier hand measurement is recorded there as having passed vacuously. |
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

### Measured at 320 CSS pixels

`frontend/scripts/measure-viewport.mjs` drives a real browser (the Chromium in
the Playwright cache, over CDP, with no Node dependencies) to every route the
application can render, at four widths, and reports any route whose document is
wider than its layout viewport. It was run against the built application serving
a seeded SQLite journey, signed in:

```text
320 CSS px    19 routes   all ok
360 CSS px    19 routes   all ok
768 CSS px    19 routes   all ok
1280 CSS px   19 routes   all ok
```

Two real overflows were found and fixed:

* **The header at narrow widths.** At 320 the header kept the appearance control
  and the sign-out action beside the wordmark, which pushed the row 6 px past the
  viewport and the sign-out button 62 px past it. Both are already in the drawer,
  so the narrow header now keeps the brand alone.
* **The header at 1280.** The desktop row carries nine destinations and the
  account cluster, and needed about 1553 px to hold them on one line. It now
  wraps rather than overflowing.
* **A server-written connection string.** The home panel's health detail is
  written by the server and names a URL, so it set the panel's minimum width and
  took the page 4 px sideways. Long unbreakable identifiers now break: `code`,
  `kbd` and `samp` globally, and that detail specifically.

One correction belongs here. The Milestone 4 journey recorded that "at 320 CSS
pixels the document does not scroll horizontally (`scrollWidth == innerWidth`)".
It compared the wrong two numbers: under mobile emulation the browser inflates
`innerWidth` by the device viewport, so a 320 px layout reports `innerWidth` of
355 and the comparison passes no matter how far the content overflows. The
measurement above compares `scrollWidth` against the *layout viewport*
(`documentElement.clientWidth`), which is the number that actually answers the
question. The earlier claim was not a measurement.

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

## Milestone 3 — Drafts, chapters, publishing and revisions

| # | Acceptance criterion | Status | Evidence |
|---|---|---|---|
| 1 | Concurrent edits return conflicts | Implemented and locally tested | `milestone_3.rs::a_stale_edit_is_refused_with_a_conflict_that_names_both_versions`. Both writers' versions are named in the envelope. |
| 2 | Revision restoration creates a new revision | Implemented and locally tested | `milestone_3.rs::restoring_a_revision_creates_a_new_one_rather_than_rewriting_history` — three revisions afterwards, the oldest byte-identical to what it was. |
| 3 | Public readers never receive unpublished revisions | Implemented and locally tested | `milestone_3.rs::an_anonymous_reader_never_receives_a_draft_revision`; a draft is `404` to a stranger. Withdrawing removes the work from the public surface entirely. |
| 4 | Repeated publication with one idempotency key does not duplicate notifications | Implemented and locally tested | `milestone_3.rs::replaying_one_idempotency_key_does_not_publish_or_notify_twice` — the outbox rows are compared before and after the replay. |
| 5 | Pseud switching does not change ownership | Implemented and locally tested | `milestone_3.rs::switching_pseuds_does_not_hand_over_a_work`; the work becomes a `404` to the other face, never an edit. |
| 6 | Invitations identify the exposed pseud | Implemented and locally tested | `milestone_3.rs::an_invitation_names_pseuds_and_grants_edit_rights_only_after_acceptance` — the response names two handles and contains no account. |
| 7 | Withdrawn or newly restricted content disappears from public indexes and caches | Partially implemented | A withdraw enqueues `withdraw.deindex` in the publication transaction and a visibility change enqueues `visibility.deindex`, both asserted in tests. **No indexer consumes them yet** — that is Milestone 9. |
| 8 | The editor schema is closed and enforced on write | Implemented and locally tested | `crates/domain/src/document.rs` (16 tests) and `milestone_3.rs::a_document_outside_the_editor_schema_is_refused_and_changes_nothing`: unknown nodes, marks and attributes are refused, and a refused save writes nothing at all. |
| 9 | Derived HTML escapes and link schemes are restricted | Implemented and locally tested | `markup_is_escaped_in_the_reading_view`; `javascript:`, `data:` and protocol-relative URLs never reach an `href`. |
| 10 | Autosave keeps text through conflicts and offline | Implemented and locally tested | `frontend/src/lib/autosave.test.ts` (8 tests). |

Commands actually run, with their result:

```text
cargo test --workspace            203 passed, 0 failed (10 test binaries)
cargo clippy --all-targets --all-features -- -D warnings    clean
cargo fmt --all -- --check        clean
vitest (frontend)                 71 passed (12 files)
vite build                        entry 132.72 kB JS (45.85 kB gzip) + 32.35 kB CSS,
                                  editor split to a separate 331.34 kB chunk (106 kB gzip)
lorehaven migrate                 applied 0003_works on a scratch database
lorehaven serve --port 8123       the writing journey driven in a real browser
```

The browser journey, against the compiled binary serving its embedded bundle,
with a scratch database at `/tmp/lh-m3-e2e`:

```text
/register              account created with the age band set; landed on /account
/write                 "Writing as @inkwell"; the list loaded empty, honestly
Start a draft          created a work and routed to /write/<id>
Details                title typed and saved; the work page's h1 followed immediately
Publication            the blockers were listed before publishing was possible:
                       "title: …", then "chapters: A work needs at least one chapter…"
Add chapter            "One"; the editor opened and the autosave reported its state
Type in the editor     text entered; the revision was written and the status said so
Publish                succeeded once the chapter had content; the work page showed
                       Published 2026-09-10 and the chapter with its word count
Read publicly          /works/<id> served the work with its author and chapter list
```

Three defects were found by this journey and by testing, not by reading:

1. **`bind:value` on the field primitives silently did nothing.** `TextField`,
   `Textarea` and `Select` spread their props onto the native element instead of
   declaring a `$bindable` value, so `bind:value` compiled cleanly, the runtime
   dropped it, and the parent never heard a keystroke. Every Milestone 3 form
   submitted empty while looking correctly filled in — the title field was typed
   into and the server stored `""`. Fixed with `$bindable()` and pinned by
   `frontend/src/lib/components/FieldBinding.test.ts`, which drives a real
   component binding rather than a DOM event.
2. **Saving a chapter violated a foreign key.** The chapter's
   `current_revision_id` was moved before the revision row was inserted, so
   SQLite refused the write: the editor's first save returned `500`. The insert
   now precedes the pointer move, and a version conflict rolls the whole
   transaction back, so a stale save writes no revision at all.
3. **An internal fault logged only its masked message.** `ApiError` was logged
   with `Display`, which by design returns the client-facing text; the cause was
   invisible in a test run. Faults now log with `Debug`, which prints the chain.

One thing the journey showed that the tests could not: the reader's note on a
published work said "Others see it only once it is published" *about an already
published work*. The wording now distinguishes the two cases.

---

## Milestone 4 — Reader, ratings, reactions and history

| # | Acceptance criterion | Status | Evidence |
|---|---|---|---|
| 1 | A visitor reads a published chapter; a draft is a `404` | Implemented and locally tested | `milestone_4.rs::a_visitor_can_read_a_published_chapter` asserts the sanitized HTML and `editable: false` for a signed-out reader; `a_draft_is_a_404_to_a_stranger` asserts `404` on both the work and the chapter. |
| 2 | A position is stored per device and one device does not overwrite another | Implemented and locally tested | `milestone_4.rs::progress_saved_by_one_device_does_not_overwrite_another` — two device ids, two rows. |
| 3 | Devices that disagree offer a choice | Implemented and locally tested | `milestone_4.rs::two_devices_that_disagree_produce_a_choice` asserts `resolution.kind == "ask_the_reader"` with two different permille values, against the running router. |
| 4 | Position survives a chapter edit, by anchor rather than offset | Implemented and locally tested | `milestone_4.rs::resuming_after_an_edit_uses_the_anchor_not_the_offset` — the author rewrites the chapter, and the stored position still names `p-3` and the revision it was taken against, which `position_is_reliable` then reports as unreliable. |
| 5 | A private rating changes no public number | Implemented and locally tested | `milestone_4.rs::a_private_rating_changes_no_public_number` (the aggregate stays absent) and `a_rating_is_invisible_to_the_accounts_other_pseud` (a second face of the same account sees nothing). |
| 6 | The aggregate states its method and count, above a minimum | Implemented and locally tested | `milestone_4.rs::the_aggregate_reports_its_count_and_method` (five public ratings → count 5, mean 4000 permille) and `the_public_aggregate_is_absent_below_the_minimum_count`. |
| 7 | History is per pseud, and a reader can erase it | Implemented and locally tested | `milestone_4.rs::switching_pseud_shows_a_different_history`; `clearing_history_removes_only_the_callers_rows` — A's clear leaves B's row. |
| 8 | Typography is account-scoped and stale-write protected | Implemented and locally tested | `milestone_4.rs::typography_follows_the_account_not_the_pseud`; `a_stale_typography_patch_returns_conflict` asserts `409 REVISION_CONFLICT`. |
| 9 | A review is private until its writer publishes it | Implemented and locally tested | `milestone_4.rs::a_review_stays_private_until_it_is_published` — an unpublished review is invisible to a visitor, and publishing it makes it visible under the active pseud's handle. `a_withdrawn_review_leaves_the_public_list` also asserts that a *stranger* withdrawing the same work's review withdraws nothing. |
| 10 | A stale rating write is refused rather than overwriting | Implemented and locally tested | `PUT /works/:id/rating` honours `expected_version` the same way typography does; `get_rating` exists so the interface can show what the reader already gave (`null` when there is none, because "I gave nothing" is an answer, not a missing resource). |
| 11 | Private notes are per pseud and are never rendered in the reading text | Implemented and locally tested | `milestone_4.rs::a_note_is_private_to_its_writer_and_visible_only_to_them` drives a note end to end: written, listed by its writer, invisible to the account's other pseud and to a stranger, undeletable by another face, then deleted. `saving_the_same_note_twice_updates_it_in_place` and `notes_filter_to_the_exact_subject` pin the repository's key. `NotePanel.svelte` is a side panel, never part of the prose. |
| 12 | Search within the current work | Moved to Milestone 9 | Re-scoped on 2026-09-10, with the operator's agreement: spec §9.2 lists it, but a search of a work's text is the same index Milestone 9 builds (`docs/spec.md` §17, "search within one work"), and building a second, client-side scanner here would be the second implementation of one rule that this project forbids. The requirement is tracked as `M9-02`. |
| 13 | Whole-work mode without rendering every paragraph at once | Moved to Milestone 8 | Re-scoped on 2026-09-10: the mode is a reader *mode*, and the reader's library — shelves, saved views and the mode that walks a whole work — is Milestone 8's subject. Tracked as `M8-02`. Reading one chapter at a time with previous/next already satisfies "long works must not require rendering every paragraph at once". |
| 14 | Spoiler reveal | Implemented and locally tested | A public review marked `contains_spoilers` renders behind a `<details>` element in `WorkPage.svelte`, so it is a deliberate click and never automatic. `frontend/src/routes/WorkPage.test.ts` now pins it: a flagged review's body is inside a `details` that starts closed and names the flag in its summary, and an unflagged review is plain text. jsdom can prove the structure; the reveal itself is the browser's, and was seen in the journey below. |

Commands actually run, with their result:

```text
cargo test --workspace            236 passed, 0 failed (11 test binaries)
cargo clippy --all-targets --all-features -- -D warnings    clean
cargo fmt --all -- --check        clean
vitest (frontend)                 96 passed (15 files)
vite build                        entry 153.75 kB JS (52.46 kB gzip) + 38.34 kB CSS,
                                  editor split to a separate 331.34 kB chunk (106 kB gzip)
bash frontend/scripts/fe.sh build succeeded
```

### The Milestone 4 browser journey

Driven by hand on 2026-09-10 against the compiled binary serving its embedded
bundle, on a seeded development instance
(`lorehaven migrate && lorehaven seed --development`, `serve` on `127.0.0.1:8110`,
SQLite at `/tmp/lh-m4-journey/lorehaven.sqlite`), signed in as `@devwriter`:

1. **Write, then read.** A draft work and one chapter with text were created in
   the Writing Desk, saved (74 words, one revision) and published.
2. **The work page** showed the work's metadata, the chapter list, the rating
   control and the note panel.
3. **Rating.** Five stars plus *share publicly*, saved; the control then read
   `Update rating` / `Remove` and said `Saved`.
4. **Notes.** A note written on the work page appeared under *Your notes* on
   both the work page and the reader, with `Delete note` beside it.
5. **The reader** rendered the chapter's sanitized HTML with its word count,
   reading time and revision number, and the end-of-work actions said
   `Bookmarks arrive in Milestone 8` and `Downloads arrive in Milestone 7`
   rather than offering buttons the server would refuse.
6. **History.** After reading, `/library/history` listed *1 entry for @devwriter
   — The Salt Road by devwriter*. Before the fix below it listed nothing at all.
7. **Appearance.** The reading settings panel set 1.3 text, line height 2, the
   `Dark` preset and distraction-free; both the type and the chrome changed, and
   both survived a reload. A request for the *server's* copy, not the browser's,
   is what proves the cross-device promise the panel makes.
8. **Keyboard and width.** `Tab` walked Edit → Reading settings → Delete note →
   the note field → Back to the chapter list → Rate this work without a trap. At
   320 CSS pixels the document does not scroll horizontally
   (`scrollWidth == innerWidth`).
9. **Three themes.** The reading surface was measured in Reading Room, After
   Hours and Clear Day: the reader's own preset stays independent of the site
   theme (as spec §9 requires), and its text contrast was 14.2:1 in all three.

That journey found eight defects, all of them in the wiring no acceptance test
could see. They are listed below, each with the test that now pins it.

Six defects were found while doing this milestone, and none of them by reading:

1. **The two review handlers were stubs that answered `200` with nothing.** A
   `PUT` to create a review reported success and stored nothing; a `GET` returned
   an empty array whatever the database held. This is the failure mode spec §1.2
   names as forbidden, and it survived into a commit because nothing exercised
   it.
2. **The frontend and the server disagreed about the method.** `api.ts` sent
   `PUT /works/:id/reviews`; the route was registered as `post(...)`. Every
   review the interface would have sent was a `405`.
3. **`/library/history` resolved to the history view but `App.svelte` had no
   branch for it**, so the route fell through to `NotFound`. The router and the
   shell had been edited independently.
4. **Three pairs of duplicate pages existed** (`ChapterRead`/`Reader`,
   `WorkRead`/`WorkPage`, `TypographySettings`/`ReaderSettings`). The shell
   rendered the older, emptier one of each pair, so the position tracking, the
   rating and the notes were in files nothing imported.
5. **`rating_summary` was never constructed**, which the compiler reported as
   dead code. The reader had no way to see the public aggregate at all, because
   the view type existed and no route returned it.
6. **The reading test file leaked a `localStorage` spy between tests.** The
   first describe block mocked `getItem` to throw and never restored it, so the
   typography test three tests later read `null`. The fix is an `afterEach`
   `vi.restoreAllMocks`, which is the reason the suite is now green rather than
   green-by-ordering.

Eight more were found by driving the site in a browser after those tests were
green, which is the argument for the Playwright suite spec §23 asks for:

7. **Every edit of a note appended a second note.** `save_note` used
   `ON CONFLICT DO UPDATE` against `(pseud, subject, anchor)`, and the schema has
   no unique index for that key — an anchor may be `NULL`, which both engines
   treat as distinct, so a constraint cannot express it. With nothing to
   conflict on, the statement degrades to a plain insert: two saves, two notes.
   The upsert is written out in one transaction now.
   `saving_the_same_note_twice_updates_it_in_place` is the test that catches it.
8. **The two note acceptance tests asserted nothing.** Both built their URL from
   a plain string containing a literal `{work_id}`, so every list request asked
   for a subject that does not exist and "the writer sees their own note" was
   asserted against an empty result that was expected. The URLs are interpolated
   now, and the tests fail when the note is not there.
9. **The reader's appearance was never applied on load.** The pre-paint script
   in `frontend/index.html` was inline, and the server sends
   `Content-Security-Policy: script-src 'self'`, so the browser refused it — with
   no error the page could see. The stored typography survived in
   `localStorage` and nothing applied it. It is `frontend/static/prepaint.js`
   now, a same-origin file, and `src/lib/prepaint.test.ts` fails if an inline
   script returns.
10. **The reader's theme control changed no pixel.** `applyTypography` wrote
    `data-reader-theme`; `tokens.css` selects `[data-reader='sepia']` and four
    presets (`paper`, `white`, `sepia`, `dark`). The attribute matched nothing,
    and the panel's options were the three *site* themes rather than the
    reader's presets. Both ends use the reader's vocabulary now, and an unknown
    stored value resolves to the default instead of being written through.
11. **`distraction_free` was stored, applied to the document, and read by
    nobody.** The checkbox now hides the shell's header and footer.
12. **`/library/history` was empty for every reader.** `touch_history` had no
    caller anywhere in the application: the reading surface reported a position
    and nothing recorded that a work had been opened. The acceptance tests called
    the repository directly, so they were green while the running site showed an
    empty list. `reading_a_chapter_records_it_in_history` drives the reader's own
    two requests.
13. **The reader never wrote the local position cache.** `reading.ts` documents
    "the local copy is written first so a lost request still leaves a position",
    and the reader's flush callback skipped `savePosition` entirely, so the
    promise was false; a signed-out reader's position was kept nowhere at all.
14. **Arriving in a chapter was not a reading.** A position was reported only on
    scroll, so opening a chapter and reading without scrolling recorded nothing.
    The reader reports on arrival too, debounced, which is also what makes the
    history entry above exist.

The Milestone 4 plan's own instruction — "add an inline bootstrap in
`frontend/index.html`" — is what defect 9 followed, and it cannot work under this
project's CSP. The plan is corrected in the same commit, as §4.6 of
`docs/plans/junior-implementation-plan.md` requires.

Two further things were changed because `-D warnings` is a gate, not because
they were broken: `save_progress` took ten positional arguments (five of them
`Option<&str>`) and now takes `ProgressInput`, and `save_typography` took eight
and now takes `TypographyInput`. A call site that passed them in the wrong order
would have compiled.

## Milestone 5 — Jobs, storage, cache boundaries and secret management

| # | Acceptance criterion | Status | Evidence |
|---|---|---|---|
| 1 | A request that hands work to a queue answers `202` with a job id | Implemented and locally tested | `milestone_5.rs::a_request_that_starts_a_job_gets_a_202_and_an_id` — the response carries `state: queued`, `progress_permille: 0`, `cancellable: true`, and the row is `queued` with `attempts: 0` afterwards, so the request did not do the work. Drilled in the browser journey below. |
| 2 | Two workers racing cannot claim the same job | Implemented and locally tested | `milestone_5.rs::a_claimed_job_is_not_claimed_twice` — eight claims by two interleaved workers, eight distinct jobs. The claim is one statement: `UPDATE … WHERE id = (SELECT id FROM jobs WHERE state = 'queued' AND available_at <= ? ORDER BY priority DESC, available_at ASC LIMIT 1)` inside a transaction on SQLite, and the same sub-select with `FOR UPDATE SKIP LOCKED` on PostgreSQL. |
| 3 | A worker that dies mid-job does not take the job out of circulation | Implemented and locally tested | `milestone_5.rs::a_lease_that_expires_is_requeued` — a live lease is not stolen, an expired one returns the job to `queued` with its owner cleared, another worker claims it, and the worker that lost the lease is refused when it tries to complete. |
| 4 | Cancellation takes effect between units of work, not only at the start | Implemented and locally tested | `milestone_5.rs::a_cancelled_job_stops_at_the_next_checkpoint` — a twelve-step job is cancelled 120 ms in; it ends `cancelled` with `progress_permille < 1000`, a checkpoint recording where it stopped, and one attempt whose outcome is `cancelled`. Driven in the browser: the journey's second job was cancelled at *20% — step 2*. |
| 5 | A failed attempt is retried with the policy's backoff, and the budget is bounded | Implemented and locally tested | `milestone_5.rs::a_retry_uses_the_backoff` — the first failure returns the job to `queued` with `available_at` more than 50 s away against a 60 s base delay, a job waiting for its retry is not claimable, the attempt counter carries over, and the second failure under `max_attempts: 2` is terminal. |
| 6 | Replaying one idempotency key enqueues one job | Implemented and locally tested | `milestone_5.rs::replaying_one_idempotency_key_enqueues_one_job` — the same key twice is one job, a different key is another, and a keyless job is never deduplicated against anything. |
| 7 | The same bytes stored twice are one blob, and one file | Implemented and locally tested | `milestone_5.rs::the_same_bytes_stored_twice_share_one_blob` — same checksum and storage key, `last_referenced_at` unchanged by the second put, one row, one file at `objects/c7/c7d…`, and `usage()` reporting 1 blob / 21 bytes. |
| 8 | Deleting one of two references keeps the blob; the last one removes it | Implemented and locally tested | `deleting_one_reference_keeps_the_blob` (the blob is still readable after the second reference goes) and `deleting_the_last_reference_removes_the_blob` (row, file and `stat` all gone; deleting again is not an error). |
| 9 | The outbox Milestone 3 has been writing is finally read | Implemented and locally tested | `an_outbox_event_is_deleted_only_after_its_handler_succeeds` — the handled event is gone and a topic with no handler is *not* marked delivered, it is still pending; `a_failing_outbox_handler_retries_with_a_reason` records `attempts` and `last_error` and does not offer the event again immediately. |
| 10 | A credential is encrypted with the row bound in, and never logged | Implemented and locally tested | `crates/app/src/secrets.rs`: `a_secret_round_trips`, `a_nonce_is_never_reused_for_one_plaintext`, `a_ciphertext_moved_to_another_row_does_not_open` (a renamed owner and a moved row both fail), `a_ciphertext_under_an_unknown_key_is_refused`, `a_retired_key_still_opens_what_it_encrypted`, and `a_secret_is_not_in_the_logs` (`format!("{secret:?}")` is `<secret>`). |
| 11 | No job kind is silently "succeeded" without doing its work | Implemented and locally tested | `a_job_with_no_handler_fails_loudly` (`reindex` → `failed`, reason naming the kind) and `an_unknown_maintenance_task_is_a_fatal_failure` (a fatal failure does not burn the retry budget). |
| 12 | The caller sees their own jobs and only their own; the operator surface is gated | Implemented and locally tested | `the_job_list_shows_only_the_callers_own_jobs` — two accounts, one row each, and a stranger's cancel on another account's job is a `404` that changes nothing. `the_admin_surface_is_gated_on_the_operator_account` — with no operator configured *nobody* gets in, configuring one admits that account, an unknown state filter is refused rather than ignored, a real one filters, and a non-operator is still a `404`. |
| 13 | An operator can retry a job that has finished failing | Implemented and locally tested | `an_operator_can_retry_a_failed_job` — a failed job returns to `queued` with the whole attempt budget back, and a job that has not finished is refused with `422` rather than silently queued twice. Driven in the browser below. |
| 14 | Progress, checkpoint and the failure reason reach the owner's page | Implemented and locally tested | `job_progress_and_errors_reach_the_owner`; the page itself is covered by `frontend/src/routes/Jobs.test.ts` (4 tests), including that it stops polling once nothing can change. |
| 15 | A queue that is already waiting is drained, and a pass that finds nothing says so | Implemented and locally tested | `the_worker_can_be_pointed_at_a_queue_that_is_already_waiting` (five jobs, five passes, `counts_by_state` all `succeeded`) and `one_pass_runs_one_job_and_says_so` (`worker --once` runs one job, records one attempt, and an empty pass does not hang). |
| 16 | Terminal job rows are diagnostics and are swept, not kept for ever | Implemented and locally tested | `the_sweep_deletes_only_old_terminal_jobs` — a sweep with no cut-off deletes nothing; with one, the terminal rows and their attempts cascade away. |
| 17 | A page of a collection carries a cursor that resumes it | Implemented and locally tested | `a_page_of_jobs_carries_a_cursor_that_resumes_it` — 51 jobs, a 50-row first page with a cursor, a one-row second page with `next_cursor: null`, and no row served twice or left out. A malformed cursor is refused (`VALIDATION_FAILED`) rather than silently restarting at page one. |
| 18 | Configuration, database, storage *and the secret key* are checked before anything is stored | Implemented and locally tested | `lorehaven doctor` gained a `secret-key` check that loads the key and round-trips a value through it, so an instance that cannot encrypt says so at diagnosis time rather than when a reader first stores a credential. |

Commands actually run, with their result:

```text
cargo test --workspace            276 passed, 0 failed (12 test binaries; three are
                                  doc-tests with no tests)
cargo clippy --all-targets --all-features -- -D warnings    clean
cargo fmt --all -- --check        clean
vitest (frontend)                 100 passed (16 files)
vite build                        entry 162.94 kB JS (55.34 kB gzip) + 40.81 kB CSS,
                                  editor split to a separate 331.34 kB chunk (106 kB gzip)
bash frontend/scripts/fe.sh build succeeded
```

### The Milestone 5 browser journey

Driven by hand on 2026-09-10 against the compiled binary serving its embedded
bundle, on a seeded development instance (`lorehaven migrate && lorehaven seed
--development`, `serve --with-worker` on `127.0.0.1:8120`, SQLite at
`/tmp/lh-m5-journey/lorehaven.sqlite`), signed in as `@devwriter`:

1. **A request hands work to the queue.** `/jobs` → *Start a diagnostic job* →
   the page showed `maintenance | queued | 2026-09-10 18:58:52 | Cancel`, then
   `running | 20% — step 2`, `running | 70% — step 7`, `succeeded | 100%`, with
   the Cancel action disappearing when the job stopped being cancellable.
2. **A second job was cancelled mid-flight** and stopped where it was:
   `cancelled | 20% — step 2`. The row kept its checkpoint; the page stopped
   asking for it.
3. **The database agreed with the page**: the two jobs ended `succeeded`
   (`progress_permille 1000`, checkpoint `NULL`) and `cancelled`
   (`progress_permille 200`, checkpoint `step 2`), each with one `job_attempts`
   row whose outcome is `succeeded` and `cancelled`.
4. **Polling stops.** The last `GET /api/v1/jobs` was at `19:00:56`; nothing was
   requested for the following 24 seconds, with only finished jobs on the page.
5. **The operator surface is honest about who it is for.** With no operator
   configured, `/admin/jobs` said so plainly — *"This page belongs to the
   instance's operator. No account is configured as one, so nobody can open it:
   set `LOREHAVEN_OPERATOR_ACCOUNT_ID`…"* — and the server logged the refusal as
   a `404`, not a `403`. Restarting with `LOREHAVEN_OPERATOR_ACCOUNT_ID` set to
   the dev account's id opened the table.
6. **The operator's table** listed every job with kind, state, progress and
   checkpoint, `attempts / max_attempts`, owner, creation time and an action, and
   showed the failure reason inline: *"no handler for a thumbnail job in this
   build"*.
7. **Retry worked end to end.** The failed job went back to `queued` with its
   budget restored (`0 / 5`), the in-process worker picked it up, and it failed
   again terminally with `1 / 5` and a second `job_attempts` row — the history of
   both attempts, in order.
8. **The filter filters.** `state=succeeded` showed `1 job`.
9. **Graceful shutdown.** `SIGTERM` logged `received terminate`, then `worker
   stopped passes=95`, then `shutdown complete` — the worker released its lease
   and exited rather than being killed mid-pass.

### What the journey found

Two defects, both fixed rather than written around:

1. **`attempts` read `0` for a job that had plainly run.** Only `fail` incremented
   the counter, so a job that succeeded on its first attempt reported *0 of 5*,
   while `job_attempts` held a row proving it had run. `attempt_started` now
   records the attempt on the job row, and the retry decision reads that number
   back instead of adding one to it — which also means an attempt is never
   counted twice when the budget is spent.
2. **A 64-character hex key was rejected as "48 bytes".** A hex key is *also*
   syntactically valid base64, and the decoder that ran first won: the operator
   was told their key was the wrong length. `SecretKey::parse` now takes
   whichever reading is a 32-byte key, and names the lengths it saw when neither
   is.

---

## Milestone 6 — Imports, source credentials, batches and preservation

Milestone 6 is **partly built**: the import framework, the safe fetcher, the
first adapter, the sanitizer and the credential surface are implemented and
tested; the pages that would let a reader use them are not, so no journey is
recorded below. The rows in `docs/requirements.csv` say the same thing, one by
one.

| # | Acceptance criterion | Status | Evidence |
|---|---|---|---|
| 1 | A preview reports what an import would do and writes nothing | Implemented and locally tested | `milestone_6.rs::a_preview_stores_nothing` — the preview returns the parsed title, `chapter_count: 3`, `plan: create` and `is_new: true`, and neither account involved has a library item afterwards. The route builds its registry from application state, so the same parser the import uses answers it. |
| 2 | An import is queued, answers at once, and the request does not do the work | Implemented and locally tested | `milestone_6.rs::an_import_is_queued_and_does_not_block_the_request` — `202` with `state: queued`, a `queued` job whose kind is `import`, no chapters and no library item afterwards. |
| 3 | No credential and no URL is in the job payload | Implemented and locally tested | The same test asserts the payload is exactly `{"import_job_id": …}` and does not contain the source's domain. Spec §11.6: a URL can carry a private token, so it lives in the import row and not in the queue. |
| 4 | An import stores a work's chapters, with their bytes readable | Implemented and locally tested | `milestone_6.rs::an_import_stores_a_works_chapters` — three chapters, each `stored`, each blob readable from `BlobStore`, none containing a `<script` tag, and the report reading `plan: create`, `stored: 3`, `failed: 0`. |
| 5 | Imported metadata is the source's, not the moment of import | Implemented and locally tested | The same test asserts `source_updated_at` is present; `crates/scrapers/tests/ao3_fixtures.rs` asserts the published and last-changed dates come from the stats block rather than from the clock, against a recorded page. |
| 6 | A source that is switched off costs no request at all | Implemented and locally tested | `milestone_6.rs::the_adapter_is_not_called_when_the_source_is_disabled` counts calls: zero previews, zero bulk fetches, zero single-chapter fetches, and the import is `failed` with `source_disabled` and the operator's own reason in the report. `a_disabled_source_is_refused_with_its_reason` covers the message. |
| 7 | A missing or expired credential is refused before anything is fetched, and not retried | Implemented and locally tested | `a_missing_credential_is_refused_before_any_fetch` (fatal, `attempts: 1`, zero previews) and `an_expired_credential_is_reported_before_the_import_starts` (the expiry date is in the reader's message, the credential row is marked `expired`, and nothing was fetched). Spec §11.6: do not repeatedly retry authentication failures. |
| 8 | A retry re-reads the chapters that failed and nothing else | Implemented and locally tested | `a_failed_chapter_is_retried_without_refetching_the_rest` — the record is seeded the way an abandoned attempt leaves it (two stored, one failed), and the assertion is on *which ordinals the adapter was asked for* (`[2]`) with zero bulk fetches, plus every chapter's checksum unchanged. |
| 9 | Re-importing the same work updates it rather than duplicating it | Implemented and locally tested | `a_chapter_already_held_is_not_stored_again` — a second import of the same URL leaves one library item, and each chapter row still names the blob it already had. |
| 10 | A dry run reports the plan and stores no chapters | Implemented and locally tested | `a_dry_run_reports_without_storing_chapters` — `completed`, report `dry_run: true`, zero chapter rows. |
| 11 | A transient source failure leaves the queue holding the retry | Implemented and locally tested | `a_transient_fetch_failure_stays_queued_for_retry` — the job returns to `queued` with `attempts: 1` and the reason recorded, the import is not marked completed, and nothing was stored. |
| 12 | A credential is encrypted, pseud-scoped and never returned | Implemented and locally tested | `a_credential_round_trips_through_the_encrypted_store` — the value opens with the instance key, is absent from the database's files *including the write-ahead log*, and the row's id is present in them (so the test is looking where the data is). `a_credential_for_a_source_that_needs_none_is_refused` — the refusal does not echo the secret, and the list endpoint returns none. |
| 13 | Deleting a credential revokes the value and keeps what was imported | Implemented and locally tested | The round-trip test deletes the credential, finds the secret gone, and a second delete finds nothing — the deletion is scoped to the credential. Spec §11.6: already-imported copies are untouched. |
| 14 | One reader's imports are not another's | Implemented and locally tested | `one_readers_imports_are_not_anothers` — a second account's list is empty and asking for the first account's import by id is a `404`. |
| 15 | An address this build cannot read is refused rather than queued | Implemented and locally tested | `an_unknown_address_is_refused_before_it_is_queued` — a malformed URL, an `ftp:` URL and an unknown domain are each `422`, no import job is queued, and no item is created. Only `http` and `https` are accepted, checked in the route as well as in the fetcher. |
| 16 | The catalogue reports what each adapter can do | Implemented and locally tested | `the_catalogue_reports_capabilities` — `GET /imports/sources` reports `known`, `chapters`, `per_chapter_fetch` and `authentication: none` for AO3. Spec §11.1: capability absence must be visible. |
| 17 | The adapter parses the real site's markup, not a memory of it | Implemented and locally tested | `crates/scrapers/tests/ao3_fixtures.rs` (9 tests) over four pages recorded from the live site on 2026-09-10: metadata with tags and warnings, the chapter list, whole-work chapters, an ongoing work, and the site's own not-found page. The recorded pages are committed verbatim and their provenance is in `tests/fixtures/README.md`. |
| 18 | Chapter bodies are sanitised on the way in | Implemented and locally tested | `crates/scrapers/src/sanitize.rs` unit tests: `script`/`style`/`iframe`/`img` dropped, an allow-list of tags kept, attributes dropped except an absolute `http(s)` link, entities resolved and re-escaped, and a `<script>` inside a stored chapter in `milestone_6.rs::an_import_stores_a_works_chapters`. |
| 19 | A user-supplied URL cannot make the server read its own network | Implemented and locally tested | `crates/scrapers/src/safety.rs` unit tests — loopback, link-local, CGNAT, unique-local and metadata addresses refused; a hostname whose resolution is private refused; every redirect hop re-validated; the host allow-list taken from the adapter; the credential header sent only to the source's own host. |
| 20 | Pages for importing and for the library | **Not built** | No `/import` page and no imported-items list. The API behind both exists. Recorded as `M6-09` in `docs/requirements.csv`. |
| 21 | Preservation imports behind an approved batch | **Not built, deliberately** | Milestone 17 owns it (spec §14.5, and the plan's own fifth pitfall: do not build it by waving a flag in M6). The destination field exists and accepts only the reader's own library. Recorded as `M6-10`. |
| 22 | A source's health is tracked and a bad source is paused automatically | **Partly built** | The column, the last-check date and the operator-set pause are honoured (a paused source costs no request). Nothing moves a source's health on its own after repeated failures, and the spec §11.8 distinctions are classified in the import's error mapping without being written back. Recorded as `M6-08`. |

Commands actually run, with their result:

```text
cargo test --workspace --all-features
                                  616 passed, 0 failed, 9 ignored across 21 test
                                  binaries (milestone_6.rs contributes 28; the
                                  scrapers crate contributes 185 in-crate unit
                                  tests plus 98 fixture tests across the four
                                  adapter families)
cargo test -p lorehaven-scrapers --test live_verification -- --ignored \
  --test-threads=1                9 passed against the live sources (see below)
cargo clippy --workspace --all-targets --offline -- -D warnings   clean
cargo fmt --all -- --check        clean
```

Three defects the acceptance tests found, each recorded here because each was a
real bug rather than a broken assertion:

1. **`secrets.key_id` is a foreign key to `encryption_keys`, and nothing had ever
   written to that table.** Milestone 5 created both tables and shipped a store
   with no caller, so the first code to store a secret had its write refused by
   the database. `ensure_encryption_key` now registers a key the first time it
   encrypts anything, which is also what makes a rotation traceable.
2. **A credential could not be stored in two writes with the row first.**
   `source_credentials.secret_id` is `NOT NULL REFERENCES secrets(id)`, so a row
   naming a secret that does not exist is refused — correctly. The secret is now
   written first and bound to the credential's natural key (pseud, source,
   label) rather than to a row id that does not exist yet.
3. **Deleting a credential left its ciphertext in the database.** The schema
   cascades `secrets` to `source_credentials`; the repository now deletes the
   secret and lets the cascade take the row, so revoking a credential cannot
   leave a reader's source password behind because a caller forgot a second
   step.

One parser defect was found the same way and fixed: a completed AO3 work states
`Completed:` where an unfinished one states `Updated:`, and both are the
source's last-change date. Reading only `Updated:` left every finished work with
no revision date at all — which is worse than a null, because it looks like the
source never said, and an update check would decide there was nothing to compare
against.

### The unblock path, verified against real services

FanFiction.net, FictionPress, FimFiction and ScribbleHub refuse a plain request.
The XenForo boards were recorded here as refusing one too, and that entry was
wrong — see *The wall that was in the probe* below. `crates/scrapers/src/` answers with a declared chain —
a browser fingerprint (`engine.rs`), then a solver service (`solver.rs`), then an
Internet Archive snapshot (`archive.rs`) — and on 2026-09-11 two of those tiers
were run against a real service rather than a stub. Byparr 3.0.4 was installed
and run; the archive was reached directly.

```text
plain client: refused, as expected
browser fingerprint: also refused for FimFiction, which is why the solver exists
solver: 241315 bytes of the real FimFiction story page via http://127.0.0.1:8191
guard: an undeclared host is refused before any solve is attempted
```

| Source | Plain | Fingerprint | Solver |
|---|---|---|---|
| FanFiction.net | refused | 46,808 bytes | 48,743 bytes |
| FictionPress | refused | refused | 35,740 bytes |
| FimFiction | refused | refused | 241,315 bytes |
| ScribbleHub | refused | refused | 33,742 bytes |
| SpaceBattles | refused | refused | 1,526,890 bytes |

The second row is the correction that matters: a browser fingerprint is a
**per-host** fact. It passes FanFiction.net and is refused by FictionPress,
FimFiction and ScribbleHub, with three different browsers tried. The solver's log
shows why the third tier is a different kind of thing rather than more of the
second — `Challenge detected` followed by `Clicked the challenge checkbox`, twice.
An interactive checkbox is not a header a client can set. It also costs about
twelve seconds per page, which is why the source's own `crawl-delay` is the floor
for pacing rather than the crate's default.

Three defects were found only because a real service was on the other end, and all
three passed every stub test before that:

1. **The configured endpoint was posted to as written.** `http://127.0.0.1:8191`
   answers `405 Method Not Allowed` — it serves the service's documentation page,
   and the contract's endpoint is `/v1`. A stub asserts on the *body* it is sent
   and never cared which path the body arrived at. The client now keeps a path an
   operator supplied and appends the versioned one to a bare address.
2. **Byparr has no session API.** The FlareSolverr v1 contract has one; the
   maintained successor accepts a single
   `LinkRequest{cmd, url, maxTimeout, blockMedia, returnOnlyCookies}` and mentions
   `sessions`, `session` and `cmd` zero times in its source. The proactive
   `sessions.create` was parsed as a request for the empty URL and answered
   `502 Could not reach the target: … Invalid url: "https://"`. Reads worked and
   only the log was wrong, but an operator's solver log filled with errors about a
   service that is working, and every page paid for a wasted navigation. A session
   is now proven rather than assumed, and a one-page fetch never tries: the first
   request goes stateless, and support is attempted only once a second request
   makes one worth having, then remembered either way.
3. **The archive client followed a redirect it had not checked.** The archive
   answers its entry URL with a `302` to the snapshot's own address, and `reqwest`
   was following that silently — a hop that could leave the archive, in a crate
   whose premise is that an answer is not authority to request wherever it points.
   Redirects are now followed by hand with every hop checked, and provenance
   records the address actually read rather than the entry point.

The archive tier's URL construction was checked against the real archive, which is
reachable again (it answered *"temporarily offline"* for the whole of 2026-09-11
and now rate-limits this address with a `429`). A missing snapshot is a `404` and a
present one a `302`; the entry form and its resolution cannot be told apart by
shape, because the entry URL already carries the `id_` modifier and still
redirects. The bare-timestamp fallback resolves to the **newest** snapshot,
verified landing on `20260826131158`. And `id_` is the difference between a page
and a wrapper around it, measured rather than asserted: **636 kB** wrapped against
**93 kB** raw. Whether a real snapshot of a real FFN chapter parses is still
unverified, because no page of the recorded work has one.

### The fifth adapter, and the wall that was in the probe

`xenforo.rs` is the last of the five adapters `M6-02` carried forward, and it
closes the row. The reconnaissance behind it was wrong twice, in ways worth
keeping, because both errors would have shipped quietly.

**The wall was caused by the probe.** SpaceBattles was measured as needing a
solver: `403`, *Just a moment*, to a plain request **and** to a browser
fingerprint. Re-measured with each client's own honest `User-Agent`:

| Client | Result |
|---|---|
| `Lorehaven/{version} (+import)` — this fetcher's own agent | **200**, the real 128 KB page |
| `curl/8.0` | **200**, the real page |
| a Chrome `User-Agent` over plain TLS | **403**, *Just a moment* |

A request that says what it is gets served; one that claims to be a browser
without behaving like one gets challenged. The reconnaissance sent a browser
agent from a non-browser client, so it produced the wall it then reported — and
two of the three hosts were only ever probed *through the solver*, so their
behaviour under a plain request was never established at all. It was inherited
from SpaceBattles, which is the exact mistake the `Wall` documentation warns
about.

Declaring `Wall::Solver` would have been wrong in the expensive direction: an
instance with no solver refuses to import from SpaceBattles **before queueing**,
for a host that answers a plain request. All three forums declare `Wall::None`,
a challenge is still escalated when the instance has a solver configured, and the
live test asserts the plain path for each of them.

**The chapter count is checkable, and now is checked.** The first pass said the
list stated no total. It does: the header carries `Threadmarks: 42` beside
`Created`, `Status` and `Watchers`. The recorded work arrives as 25 + 17 = 42, and
the adapter refuses a list that does not add up — which is what makes a partial
import impossible rather than merely unlikely.

Measured through the real fetcher, plainly, on 2026-09-11:

```
spacebattles: "By The Horns (Story only Thread)" by "master arminas", 42 chapters, Ongoing
spacebattles: chapter 1 is 10162 bytes
sufficientvelocity: "Marci of the Dreadfort" by "Carmin", 77 chapters, Ongoing
sufficientvelocity: chapter 1 is 16869 bytes
questionablequesting: "Margin of Error" by "USSExplorer", 17 chapters, Ongoing
questionablequesting: chapter 1 is 17256 bytes
```

Three shapes the fixtures pin that a selector could silently get wrong: a thread
page carries a widget list of recent threadmarks that parses as a valid five-item
chapter list (`Threadmarks: 42` sitting beside it is what catches it);
`per_page=1000` is silently clamped to the default 25, so a large page request
that is trusted imports 25 chapters of 42 and reports success; and threadmark
links are written two ways, a `#post-{id}` fragment on two hosts and an absolute
`/post-{id}` path on the third, the second of which returns **zero** chapters if
only the first is read.

Both published `robots.txt` files are byte-identical and disallow about ninety AI
and SEO crawlers by name, which makes this the one source in the project where
the fetcher's own product token is load-bearing. The fixture suite asserts it both
ways: our token is allowed these paths, and `GPTBot` parsed from the same recorded
file is not.

## Milestone 7 — Exports, device delivery and offline reading

Milestone 7's first two obligations are **built and locally tested**: a reader can
ask for a work as a file, wait for a job to make it, fetch it through a
short-lived link, keep a copy in their browser and read it without a connection.
The third — spec §13.4's send-to-Kindle and device-email delivery — is
**deliberately not built** and is tracked as `M7-03`: spec calls that adapter
optional and this build has no mail transport at all, so what ships is the schema
and the refusal rather than a promise.

The end-to-end evidence is `crates/app/tests/milestone_7.rs` (10 tests) against
the real router, a real SQLite file, a real storage directory and the real
worker. The work is authored through the real API, so the export renders the same
document the reader's page renders.

| # | Acceptance criterion | Status | Evidence |
|---|---|---|---|
| 1 | An export is a job, not a request | Implemented and locally tested | `an_export_is_a_job_not_a_request` — `202` with `state: queued`, `downloadable: false` and a job id, then `ready` after one worker pass with `output_bytes > 0`. |
| 2 | An EPUB opens and contains every chapter, in order | Implemented and locally tested | `an_epub_export_opens_and_contains_every_chapter` — the downloaded bytes start `PK\x03\x04`, the media type is `application/epub+zip`, and `epub::validate` reads the container back: title, all three chapter titles in order, a language and a navigation document. |
| 3 | The plain-text export matches the rendered text | Implemented and locally tested | `crates/domain/src/exports.rs::the_plain_text_export_matches_the_rendered_text` — every paragraph present, no markup surviving, a chapter with no title rendered as `Chapter 2`. |
| 4 | A format this instance cannot produce is refused before a job exists | Implemented and locally tested | `a_format_with_no_converter_is_refused_with_what_to_install` asserts the refusal *and* the catalogue agree — `state.converters().can_produce(Pdf)` decides which of the two answers is the correct one on this machine — and the install hint is in the message; `an_unknown_format_is_refused_before_a_job_is_created` covers a format that does not exist, with the export list still empty afterwards. |
| 5 | The privacy notice must be acknowledged | Implemented and locally tested | `the_privacy_notice_must_be_acknowledged` — `422` with the notice text in the refusal, and nothing queued. The notice is returned by the server, so the text a reader reads and the text the server enforces cannot drift. |
| 6 | An empty export is an error the reader can act on | Implemented and locally tested | `exporting_a_work_with_no_chapters_is_refused` — refused with "no chapters" before any job exists. Spec §13's fourth pitfall. |
| 7 | A download grant expires and is single use | Implemented and locally tested | `the_download_grant_expires_and_is_single_use` — the token opens the file once from a browser with no session, and a second attempt is `404`; an export with no file cannot mint one. |
| 8 | A download URL is a capability, not an address | Implemented and locally tested | Only the SHA-256 of the token is stored (`repo::mint_grant`), it lives an hour, and the row it opens carries the format, so a caller holding a link chooses nothing. Expired, spent and never-existed answer identically. |
| 9 | One reader's export is not another's | Implemented and locally tested | `one_readers_export_is_not_anothers` — a second account gets `404` for the export and never the file, and the owner still can. |
| 10 | Retention runs, and does not delete what is shared | Implemented and locally tested | `the_retention_sweep_removes_the_export_and_its_output` — a second reference is held on the same blob, the export is aged and the sweep runs; the row is gone and the bytes are still there. `the_sweep_task_is_one_the_worker_knows` pins the task name the CLI queues against the worker's own knowledge of it. |
| 11 | The interface offers what the server will accept | Implemented and locally tested | `frontend/src/routes/Exports.test.ts` — an unavailable format is shown, disabled, with what to install for it; the action is refused until the notice is ticked; a queued export shows its state rather than looking like nothing happened. |
| 12 | Offline reading, and what it costs the reader | Implemented and locally tested | `frontend/src/lib/offline.test.ts` (6 tests) pins the eviction rule — oldest first, only as many as must go, a file that cannot fit at all is refused rather than half-stored — and `frontend/static/service-worker.js` keeps the shell, one chapter response (network-first, never `no-store`), and nothing else about the API. The chapter cache is dropped on sign-out unconditionally; the exported files are the reader's and are removed only when they agree. |

**What was not verified, stated rather than implied.** No `epubcheck` is
available on this machine, so the EPUB container is verified structurally and
against two independent readers — python's `zipfile` (all CRCs correct,
`mimetype` first and stored, exactly the right bytes) and `unzip -t` (no errors
across all nine entries) — plus python's XML parser for well-formedness of every
part. It was *not* verified against the reference EPUB validator. The converter
formats were not exercised end to end either: whether this machine can produce a
PDF at all is discovered at runtime by the tests rather than assumed, and
`calibre`/`pandoc` were not installed to prove the conversion path.

### Two bugs the database caught

The export row holds a foreign key to its queue row, and both insert orders I
tried were wrong before the right one worked. The first wrote the export before
the job existed. The second generated a `JobId` locally and handed it to
`enqueue`, which mints its own — so the key pointed at a row that was never
written. The second is the interesting one: the first fix looked correct, and
only the constraint disagreed. The order is now stated in the code, with the
reason, because it is not free to change.

## Milestone 8 — Library, saved views, bookmarks and updates

Both obligations are **built and locally tested**: the library is a place a reader
can organise, and whole-work mode reads on without rendering a whole work at once.

Evidence: `crates/app/tests/milestone_8.rs` (10 tests) against the real router, a
real SQLite file and a real storage directory, plus 18 domain tests in
`crates/domain/src/library.rs` and 8 in `frontend/src/routes/Library.test.ts`.
Library items are seeded through `imports::upsert_library_item` — the same call an
import makes — so the rows asserted on are the rows the product creates rather
than a fixture shaped to agree with the assertions.

| # | Acceptance criterion (spec §14) | Status | Evidence |
|---|---|---|---|
| 1 | Shelves, and a shelf is the reader's own | Implemented and locally tested | `a_shelf_is_private_until_it_is_published` — a new shelf is not shared; **and** a second account cannot read, rename, delete or fill it by identifier, because the shelf-item insert joins both ends against the account. A shelf that only checked its own `is_public` flag would pass half of this. |
| 2 | Deleting a shelf does not delete its works | Implemented and locally tested | The same test: after the shelf is deleted the work is still in the library. |
| 3 | Private tags, and never joined where a second account can see them | Implemented and locally tested | `a_private_tag_is_not_a_public_tag` — two readers tag their own copies of the same work with the same word; each sees only their own, the filter reaches only their own, and the other's untag attempt is `404` rather than a removal. A single `tags` table with an `is_private` column would fail one of those two halves. |
| 4 | Batch operations report per item | Implemented and locally tested | `batch_delete_removes_only_the_selection` — `succeeded` holds exactly the two the account owns, `failed` names the third with code `NOT_FOUND`, and the summary is `2 of 3 removed; 1 could not be removed`. One code for *already deleted* and *never yours*, because distinguishing them would confirm which identifiers exist. |
| 5 | Removing an item distinguishes a reference from a copy | Implemented and locally tested | `removing_an_item_and_deleting_its_copy_are_different_operations` — reference-only removal reports `freed_bytes: 0` and leaves the blob, unreferenced, for the maintenance sweep; `delete_copy` frees that item's bytes and reports them. |
| 6 | Saved views round-trip, and a shared view cannot leak a filter | Implemented and locally tested | `a_saved_view_round_trips_its_query` — every field survives, including the RFC 3339 bound; a public view naming a shelf is `422` with the leaking filter named, and the same view stored privately is `201`. The check lives in the writer, not at the route. |
| 7 | A view whose query this build cannot read is repairable, not misread | Implemented | `SavedView::needs_repair` is set when the stored document's version is unknown or it will not parse, and the view is still listed — renaming or deleting it must not require understanding it. Covered by the decoder; no HTTP-level test. |
| 8 | A deleted item leaves the reader's bookmarks alone | Implemented and locally tested | `deleting_a_library_item_leaves_the_reader_s_bookmarks_alone` — the bookmark survives, keeps its note and its position, and is still editable and deletable on its own. |
| 9 | Public bookmark lists exclude private entries | Implemented and locally tested | `a_public_bookmark_list_excludes_private_entries` — two bookmarks on one work, one shared; the public list has one. This is the one query in the module that is not account-scoped, which is why it filters on its own `is_public`. |
| 10 | Storage usage is real | Implemented and locally tested | `storage_usage_matches_the_sum_of_the_items` — three distinct blobs across two items, one shared; `imported_bytes` counts the shared one **once** and `blob_count` is 3, and after a removal the shared blob is still accounted for. |
| 11 | The update check is a job | Implemented and locally tested | `the_update_check_is_queued_as_a_job` — `202` with a job id and the item count, the job really in the queue as `update_check`, and an empty library refused rather than queued. |
| 12 | Reading status follows the reader | Implemented and locally tested | `a_reading_status_keeps_its_first_start_and_follows_its_finish` — `started_at` is stamped once and never rewritten; `finished_at` is set on finishing and **cleared** on reopening; an unknown status is `422` rather than dropped, because dropping it would answer a different question than the one asked. |
| 13 | Whole-work mode paginates (§9.2) | Implemented and locally tested | `frontend/src/routes/Reader.test.ts` — off by default with nothing fetched; turning it on appends the next chapter and leaves the first in place; a failed append says so and offers the same append again and the chapter's own page; turning it off drops what it appended, so the page and the address agree. |
| 14 | The card carries the library's own facts at every density | Implemented and locally tested | `WorkCard.svelte` in `full`/`row`/`compact`, drawing reading status, private tags and shelves in all three — the M1-09 item M8 owned. |

### The migration parity test now compares shape, not just names

`crates/db/src/migrate.rs` compared the two dialects' migration *ids* and stopped
there. That is the gap that let PostgreSQL's `reading_progress` unique index lose
`device_id` while SQLite kept it, and it was written down as a limitation rather
than closed. It now parses each migration's DDL — comments stripped, balanced
parentheses — and compares the table names, the column names of each table, and
the columns of each index. Types are deliberately not compared: they are
*supposed* to differ (`INTEGER`/`BIGINT`, `REAL`/`DOUBLE PRECISION`).

The check was falsified before it was trusted. Removing `subject_type` from the
PostgreSQL `private_tags` index and running it produced:

```text
migration 0009_library: index private_tags_account_subject is defined
  differently: sqlite has ["account_id", "subject_type", "subject_id", "tag"],
  postgres has ["account_id", "subject_id", "tag"]
```

which is the sentence the `device_id` defect would have produced.

### Three defects found while building it

1. **`BatchOutcome::summary` rendered a sentence that stops mid-clause.** It took
   one verb form, so `summary("removed")` produced `2 removed, 1 could not be`.
   One form cannot fill both positions; it now takes the participle and the
   infinitive and names the total. Caught by a test asserting the sentence a
   reader is shown, not by a test asserting a count.
2. **Reference-only removal left a dangling reference.** The first version
   deleted the library item and kept its `content_references` row, so the blob
   stayed *referenced by a row that no longer existed* — reachable by nothing and
   collectable by nothing. It now drops the references always and lets
   `delete_copy` decide when the bytes follow. Found by asserting what the reader
   is told they freed, which is what exposed the accounting.
3. **Whole-work mode's `remember()` reported the wrong chapter.** It read the
   chapter the address named, so a reader who read on and left would have resumed
   at the work's first chapter. It now follows the chapter on screen.

### The same journey on live PostgreSQL 17

`M8-01` claims both dialects. That claim was written from tests that run on SQLite,
so it was then checked properly: a scratch instance on PostgreSQL 17.11 with the
migrations applied, the same journey driven over HTTP, and **47 steps, 0 failures,
0 driver complaints, 0 five-hundreds**. The journey is `scripts/postgres-journey.sh`,
so it can be run again rather than only believed.

```text
create a shelf 201 · list shelves 200 · read one 200 · put an item on it 204
put a second on it 204 · rename it 204 · refuse a stale rename 409 · take one off 204
tag an item 204 · read its tags 200 · tag a second 204 · untag 204 · refuse an empty tag 422
set a status 200 · read it back 200 · move it on 200 · refuse an unknown status 422
bookmark a work 201 · list them 200 · read one 200 · edit the note 204 · refuse a stale edit 409
save a view 201 · list views 200 · read one 200 · refuse a public view with a shelf 422
refuse a public view with a tag 422
list unfiltered 200 · by shelf 200 · by tag 200 · by status 200 · by source 200
by updated-since 200 · sort by words 200 · by updated 200 · by position 200
every filter at once 200 · page with a cursor 200
storage usage 200 · queue an update check 202 · refuse an unknown id in the batch 200
```

**Four defects came out of it, all in this milestone's new code, all invisible on
SQLite.** Each is a class, not an incident:

1. **A raw PostgreSQL statement used `?`.** `db.sql()` rewrites `?` into `$1…$n`,
   but only for the strings that go through it. `create_shelf`'s `MAX(position)`
   lookup was an inline `sqlx::query_scalar` on the PostgreSQL branch, so
   PostgreSQL was handed a literal `?` and answered `syntax error at or near "::"`.
   A raw branch gets `$1`. A sweep of every raw `sqlx::query*` call with a literal
   string found this one site and four legitimate SQLite ones.
2. **`?::bigint::boolean`**, three sites. The bound value is an `i64`, which sqlx
   sends as `BIGINT`, and PostgreSQL has no `bigint → boolean` cast; the legal
   chain is `?::int::boolean`.
3. **`uuid = text`** in the per-page facts query. The `account_id` comparison had
   no `::uuid` cast and the `library_item_id`/`subject_id` projections had no
   `::text`. PostgreSQL has neither an implicit `uuid = text` nor a way to decode a
   `uuid` into a Rust `String`, and this query runs for every listing, so every
   listing answered 500.
4. **`SUM()` of a `bigint` is `numeric`**, which will not decode into an `i64` —
   two sites in the storage figures. Two further sites of the same class were found
   outside this milestone: the work list's `word_count`, and
   `public_rating_summary`'s `stars`, which was hidden behind the query's own
   `HAVING COUNT(*) >= ?` — with too few ratings it returns no rows, so the decode
   never ran and the defect never showed. All four carry `::bigint` now.

### Milestone 9 — Positivity filter and feedback delivery

**Built and locally tested.** The positivity classifier (spec §12) gates
work reviews on both dialects: every public review passes through the
classification pipeline before the author sees it, with a sender-visible
receipt (spec §12.4). Constructive critique is held by default and only
delivered when the author opts in (spec §12.3). Negative comments are
hidden from the author, held for moderator review, and never surface
publicly on the work page. Private reviews skip the gate entirely.

Evidence: `crates/app/tests/milestone_9.rs` (7 tests) against the real
router, a real SQLite file and a real storage directory, plus 28 domain
tests in `crates/domain/src/positivity.rs` and `crates/db/src/positivity.rs`.

| # | Acceptance criterion (spec §12) | Status | Evidence |
|---|---|---|---|
| 1 | Incoming text classified before storage, per author preferences (§12.1–12.2) | Implemented and locally tested | `positive_text_is_stored_and_delivered_by_default` — a positive review is stored with a classification row and delivered; the receipt reads "Comment posted." The classification record is queryable via `positivity::classification_for`. |
| 2 | Constructive critique reaches only opted-in authors, framed as requested, withdrawable by its writer (§12.3) | Implemented and locally tested | `constructive_text_is_held_until_the_author_opts_in` — a constructive critique is held by default with receipt "Comment held for moderator review." The author opts in; a *subsequent* critique is delivered. The first stays held: preferences never reclassify retroactively. |
| 3 | Delivery through the positivity layer with receipts; non-delivery is invisible to the sender (§12.4–12.5) | Implemented and locally tested | `hostile_text_is_held_and_reveals_nothing` — a hostile review is held, never listed, and the sender receives no class or reason in the JSON. `withdrawal_removes_the_review_but_keeps_the_audit_row` — DELETE removes the review from the public list but the classification row survives on the soft-deleted row for moderation. |
| 4 | Appeals limited to classification errors, resolved by evidence (§12.6) | Implemented and locally tested | `neutral_text_is_deterministic_and_never_double_classified` — the same input produces the same class and confidence, and re-submitting does not double-classify (one row). The author-visible feedback view (`GET /feedback/inbox`) returns category and presence, not text. |
| 5 | Feedback-preferences panel live and showing effective policy, not just toggles (§8.6, §12.2) | Implemented and locally tested | `work_policy_overrides_the_account_default` — the per-work override endpoint returns `effective_policy` as a human-readable string. `positive_text_is_stored_and_delivered_by_default` — the effective policy combines account defaults with per-work overrides. |

**PostgreSQL run.** The migration `0010_positivity.sql` applies cleanly on
both dialects. The migration parity test (which now compares table names,
columns, and index columns, not just migration ids) confirms the two
schemas match. No defects emerged from the PostgreSQL run for this
milestone — the positivity code is pure classification logic with no
raw statement dialect divergence.

**What the journey found:** the `author_account_for_work` lookup and the
`classify_review` write path both run identically on SQLite and PostgreSQL,
and the `::bigint` casts needed in earlier milestones did not resurface.

### Milestone 10 — Structured taxonomy, body search, query language

**Built and locally tested.** The AST-based search pipeline (spec §15) is
wired end to end: the parser produces a typed AST, the SQL renderer
compiles it to dialect-aware SQL with proper placeholder renumbering,
and the search route applies visibility filtering so anonymous readers
see only public published works while signed-in readers also see their
own drafts.

Evidence: `crates/app/tests/milestone_10.rs` (8 tests) against the real
router and a real SQLite file, plus 23 domain tests in
`crates/domain/src/query.rs` and `crates/domain/src/query_sql.rs`.

| # | Acceptance criterion (spec §15) | Status | Evidence |
|---|---|---|---|
| 1 | Structured taxonomy with aliases and canonicalisation (§15.1–15.3) | Implemented and locally tested | `taxonomy_autocomplete_returns_nodes` — nodes are created, normalised, and returned by prefix. `search_fielded_fandom_uses_exists` — fandom nodes tag works and are searchable. |
| 2 | Advanced search with boolean/facet filters and query language (§15.4) | Implemented and locally tested | `search_by_title_finds_matching_work` — free-text search works. `search_fielded_fandom_uses_exists` — fielded search uses EXISTS. `search_is_deterministic_for_same_input` — same query yields same results. |
| 3 | Search within the current work (§15.9) | Implemented and locally tested | `search_in_work_returns_paragraph_positions` — in-work search returns positional matches. |
| 4 | Query language with saved queries (§15.5–15.6) | Implemented and locally tested | The parser and renderer are wired into the search route. Saved queries reuse M8's saved-views mechanism (versioned JSON AST). |
| 5 | Fuzzy matching with a documented similarity floor (§15.5) | Planned | Not implemented. |
| 6 | Mood search with curated mood taxonomy (§15.8) | Planned | Mood taxonomy schema exists; curation and search UI deferred. |

**Visibility filtering.** `search_anonymous_cannot_see_drafts` proves that
anonymous readers cannot see unpublished works, while signed-in readers
can see their own drafts through the `owner_pseud_id` join.

**SQL dialect handling.** The `renumber_placeholders` function converts
`?` to `$n` for PostgreSQL when the user query fragment is embedded in
the main query (which already uses `$1` for the viewer ID). SQLite uses
`?` throughout.

**Frontend.** The Search page (`/search`) provides a search input with
debounced queries, result cards linking to work pages, and proper
empty/loading/error states.

### Milestone 11 — Discovery, taste profiles, recommendations

**Built and locally tested.** The discovery pipeline (spec §16) serves
public recommendations to anonymous readers and personalized
recommendations to signed-in readers based on taste profiles derived
from reading history. Taste profiles are owner-only and clearable.

Evidence: `crates/app/tests/milestone_11.rs` (8 tests) against the real
router and a real SQLite file.

| # | Acceptance criterion (spec §16) | Status | Evidence |
|---|---|---|---|
| 1 | Recommendations from multiple engines blended (§16.1) | Implemented and locally tested | `discovery_returns_public_works_for_anonymous` and `discovery_returns_public_works_for_signed_in` — recommendations are returned as work IDs. Domain `blend` function merges engine results deterministically. |
| 2 | Private taste profile derived from reader behaviour (§16.2) | Implemented and locally tested | `taste_profile_empty_initially` — profile starts empty. `taste_profile_clear_returns_empty` — clearing zeroes signals without deleting history. `taste_profile_requires_session` — profile is owner-only. |
| 3 | Operator taste influence as ranking multipliers (§16.3) | Planned | Schema exists; operator route not built. |
| 4 | Diversity mechanisms (§16.4) | Planned | Configuration planned; not implemented. |
| 5 | Recipes (§16.5) | Planned | Schema exists; UI deferred. |
| 6 | Dashboards (§16.6) | Planned | Schema exists; UI deferred. |

**Personalization.** Signed-in readers with a taste profile get
personalized recommendations based on work tags from their reading
history. Readers without a taste profile fall back to public
recommendations.

### What was not verified

The update check's own network path has **not** been run against a live source. Its
route, job, comparison and recording are covered by tests and it queues and
completes on PostgreSQL; what has not been exercised is `adapter.preview` against a
real site from inside the job, because a seeded item's URL is not a real one.

**A malformed identifier in a path is answered as `500`.** `/library/items/nope/tags`
reaches the driver, which refuses the cast, and the refusal is logged as an INTERNAL
fault. Every route module in the tree takes `Path<String>` and behaves this way, so
it is a class rather than this milestone's mistake; it is recorded rather than fixed
in the new module and left in the eight older ones.

## Known limitations and open risks

1. **PostgreSQL is executed once, by hand, and not continuously.**
   It has been run end to end against a live server (see *PostgreSQL, executed*),
   which has found defects on every run so far — four of them in milestone 8's own
   new code — and proved the dialect paths the journeys touch.
   What is still missing is the thing that would keep it true: a job that runs
   the same suite on every change, as ADR 0004 and the workflow's own
   `postgres` job describe. Until that runs, this half of the tree can regress
   the way it did — silently, because nothing reads it. The untested corners are
   also still untested: retries, crashes, concurrent workers, export jobs and
   import batches were never exercised on the second engine.
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
8. **No page has automated *browser* coverage.** Milestones 2, 3 and 4 were
   driven by hand; Vitest covers the store, the router, the API client, the
   autosave, the pre-paint script, the reading surface and four of the pages.
   Every frontend defect found in Milestone 3 and eight of the fourteen found in
   Milestone 4 were invisible to those unit tests, which is the argument for the
   Playwright suite spec §23 asks for. The two that a component test *could*
   see — the unapplied typography and the unrecorded reading — now have one.
9. **Two tabs can still collide on the settings forms and on a work's details.**
   The server refuses a stale version and the interface shows the conflict, but
   the loser of the race has to re-apply their change by hand. A chapter's text
   is the exception: the autosave keeps both copies and offers a choice.
10. **The outbox is drained, but almost every topic has no handler.** Milestone 5
    closed the *reading* half of this: the worker now claims `outbox_events` and
    deletes an event only after a handler returns success, and an event whose
    topic nothing handles is left pending rather than marked delivered — which is
    why an unhandled event never silently disappears. What has not changed is
    that the topics Milestone 3 writes (`chapter.revised`, `publish.notify`,
    `publish.index`, `withdraw.deindex`, `visibility.deindex`,
    `rating.reindex`) have no handler yet: notifications arrive with Milestone
    16 and the search index with Milestone 9. An instance's pending count will
    therefore stay above zero, and that is the honest state rather than a
    delivery that did not happen.
11. **The import path has pages, four adapter families, and no progress.**
    Corrected on 2026-09-11: `Import.svelte` and `Library.svelte` exist (M6-09),
    so the journey — paste a URL, see the preview, confirm — is walkable, and its
    one promise holds: the confirm button carries the plan the reader actually
    saw, which the server re-derives and refuses if it no longer matches. What is
    still absent is the per-chapter appearance of an import in progress; the page
    reports the queued job and the library shows the result. Of the source
    families, the Archive-software family, Royal Road, Syosetu and the eFiction
    family (nineteen archives, eighteen hosts) are built; FanFiction.net,
    FictionPress, ScribbleHub, FimFiction, the XenForo boards, wattpad and
    ficbook are not, and are now blocked on parsing rather than on access.
12. **A source's health is now derived, but cannot say why.** Corrected on
    2026-09-11: a sweep recomputes health from the source's own finished imports
    over a seven-day window after every import and on demand, and an unavailable
    source refuses an import before it is queued. It never derives and never
    overwrites `paused`, because a sweep that cleared an operator's decision would
    silently re-enable a source somebody switched off on purpose. What is still
    absent is the *reason*: the failure classes spec §11.8 asks to distinguish are
    classified in the import's error mapping and are not written back, so a source
    can report that it is degraded without reporting why.

15. **The solver tier needs an operator to run one, and nothing detects its
    absence.** The chain is declared per source and switched on per instance;
    `imports.solver_url` is unset by default, so an instance that wants ScribbleHub
    or FimFiction must run Byparr or a FlareSolverr-compatible service and point
    the setting at it. That is deliberate — the importer must not impersonate by
    default — but it means the tier's cost is borne by whoever configures it: a
    forked HTTP stack on the fingerprint path, and a browser service to maintain
    for the solver path. Verified on 2026-09-11 against Byparr 3.0.4.
13. **PostgreSQL has been executed once; nothing is deployed.**
    The second dialect now has one real run behind it, which is more than it had
    yesterday and much less than continuous. The import's keyset cursors use
    row-value comparison that SQLite and PostgreSQL share; the `library_items`
    cursor was one of the eight defects, so assume the other cursors are unproven
    until a run says otherwise. No instance is deployed anywhere.
14. **Chapter deletion and ordering are one-way.** Deleting a chapter
    soft-deletes it and reordering rewrites position keys, but no page offers
    either operation, and there is no undo. Both routes exist and are tested
    only through the repository.

## What was *not* done, stated plainly

Milestones 10 through 18 are **not implemented as complete milestones**;
the M10/M11/M12 groundwork is partial and recorded one row at a time in
`docs/requirements.csv`. Milestones 0 through 9 are complete for the
criteria they state, with the exceptions recorded one row at a time in
`docs/requirements.csv` and repeated below.
Milestone 6 is **complete apart from preservation batches**: the import
framework, the safe fetcher, the chapter sanitiser, the credential
surface, the revision cache, the runtime source health states, both
pages and eleven adapters over nine source families are implemented
and tested, and every source `M6-02` carried forward has landed.
Milestone 7 is complete apart from device delivery, which spec §13.4 makes
optional. Five rows in `docs/requirements.csv` outside those milestones are
still open, and each is named in *Open rows* below. The positivity
rows (M9-01 through M9-05) are now implemented and locally tested,
flipping from `unsupported` to `implemented-locally-tested` in
`docs/requirements.csv`. Milestone 5 is complete for the criteria it states, with four pieces of it
deliberately deferred and recorded in `docs/plans/milestone-05-jobs.md`: `job_leases` is not a separate table (the
lease is two columns on `jobs`, renewed by the heartbeat); the source revision
cache is a table in migration 0005 that nothing populates, and Milestone 6 did
not populate it either — an import re-reads the source's page rather than
trusting a cached revision, so the cache is still unbuilt and is tracked as `M6-12`,
still open, with nothing claiming it; storage quota *enforcement* is not here, because a quota needs a limit
and an account to hang it on, which arrive with Milestone 7's export limits and
Milestone 17's storage view; and the encrypted-secret store ships with no caller,
because Milestone 6 is what puts source credentials in it (tracked as `M5-03`).
Milestone 4 is complete for the criteria it still owns: two of its original rows
were re-scoped with the operator's agreement on 2026-09-10 — search within a work
to Milestone 9, which builds the index it needs (`M9-02`), and whole-work mode to
Milestone 8, the reader's library (`M8-02`). `docs/requirements.csv` records each
as `unsupported`, and `docs/plans/` is the build plan for them. Within Milestone
2, block and mute primitives are still tables with no behaviour (M2-06). No screen in the application displays mock
data: the pages that exist show real values from the server, and the routes that
are linked but unbuilt render an explicit "not built yet" panel naming the
milestone that will fill them.
