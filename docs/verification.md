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
| Commit | tag `v0.04-publishing` (Milestone 3). Previous checkpoints: `v0.03-identity` (Milestone 2), `v0.01-running-app` (Milestone 0). |
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
| 14 | Spoiler reveal | Implemented; rendered, not automatically tested | A public review marked `contains_spoilers` renders behind a `<details>` element in `WorkPage.svelte`, so it is a deliberate click and never automatic. Driven in the browser journey below; still no automated test covers the rendering. |

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
10. **Nothing consumes the outbox yet.** Milestone 3 writes `chapter.revised`,
    `publish.notify`, `publish.index`, `withdraw.deindex`,
    `visibility.deindex` and `rating.reindex` rows in the transactions that
    cause them, and they are asserted in tests. The worker that delivers them
    arrives in Milestone 5, so no notification has ever been delivered and no
    index has ever been updated.
11. **Chapter deletion and ordering are one-way.** Deleting a chapter
    soft-deletes it and reordering rewrites position keys, but no page offers
    either operation, and there is no undo. Both routes exist and are tested
    only through the repository.

## What was *not* done, stated plainly

Milestones 5 through 18 are **not implemented**. Milestone 4 is complete for the
criteria it still owns: two of its original rows were re-scoped with the
operator's agreement on 2026-09-10 — search within a work to Milestone 9, which
builds the index it needs (`M9-02`), and whole-work mode to Milestone 8, the
reader's library (`M8-02`). `docs/requirements.csv` records
each as `unsupported`, and `docs/plans/` is the build plan for them. Within
Milestone 2, block and mute primitives are still tables with no behaviour
(M2-06). No screen in the application displays mock
data: the pages that exist show real values from the server, and the routes that
are linked but unbuilt render an explicit "not built yet" panel naming the
milestone that will fill them.
