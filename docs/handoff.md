# Handoff

Where the tree is, what is finished, what is open, and what a successor needs
before touching any of it. Long-form detail lives in the plan files listed at the
end; this file is the short version that must stay true to the commit it names.

## The tree

Code at **8cecc36**; every commit after it touches only `docs/`. Working tree
clean. (This line deliberately names the code commit rather than its own, so it
does not go stale the next time a document is added.)

Gates at this commit:

| Gate | Measured |
| --- | --- |
| `cargo fmt --all -- --check` | clean |
| `clippy --workspace --all-targets` | 0 warnings, 0 errors |
| SQLite workspace suite | **1217 passed / 0 failed / 13 ignored** (measured at `8bf4b38`; no Rust source has changed since) |
| the 13 ignored | network tests in `crates/scrapers/tests/live_verification.rs`, run deliberately, not by default |
| Playwright e2e | **26 passed / 1 failed / 0 skipped** (`frontend/e2e/use-cases.spec.ts`, 27 tests) |
| `svelte-check --tsconfig ./tsconfig.json` | 0 errors, 0 warnings |
| PostgreSQL | **not run — blocked**, see Environment |

The commit messages in this series are not a reliable source of test counts
(e.g. three commits in a row say "136 tests pass" while the real number was
1217). Trust `docs/verification.md`, this file, or a fresh run.

## Finished

- `29cc5b4` — every remaining route stub replaced with a real implementation.
- `d62f8cd`, `8bf4b38` — the leak round: `get_public_work` draft leak, the
  `my_audit_log` cross-account leak, the anonymous moderation-report queue, the
  anonymous abuse-status door; plus `crates/app/tests/route_inventory.rs`, which
  now requires every handler holding `State<AppState>` to declare an audience
  extractor (237/237 handlers, was 65/237).
- `8cecc36` — `frontend/e2e/use-cases.spec.ts`, the twenty ordinary use cases in
  a real browser, with the findings below encoded as tests.
- Earlier in the same day: M26 TTS narration (`6b6c684`), M25 residuals
  (`81c6cf9`), shelf CSV imports and the tutorial book (`4b33acb`).

## Open, in the order I would take it

1. **The reader's typography choice can be silently discarded** (new, found by
   test 17, which fails today). Change the theme while the settings panel is
   still loading and press Save: the PATCH carries the *previous* theme, the
   row's `version` still increments, and the choice is gone. The panel renders
   its controls before its own `GET /settings/typography` lands, and two loads
   fire per panel (mount, then again when the session settles). Fix in
   `frontend/src/lib/components/ReaderSettings.svelte`: ignore a load response
   that arrives after the reader has edited, or hold the controls disabled until
   the first response lands. Evidence: the run-5 trace (`reader_theme: "sepia"`
   in the request body while the panel showed Dark) and `docs/verification.md`.
2. **Phase 1c — make the audience a declared value.** Extend
   `route_inventory.rs` into an explicit `(method, path) → audience, scoping`
   table with a spec citation per entry and assert the registered set equals the
   tabled set. The current harness measures *presence* of an extractor, not the
   *right* audience (see the plan's N2).
3. **N2b — decide the audience of the browsing doors.** Anonymously:
   `GET /works/{published}/comments` → 401, `/forums` → 401, `/topics/{id}` →
   401, `/groups` → 401, `/taxonomy*` → 401, while `/search` → 200 and a
   published work → 200. Either those six become `optional` (amend the spec to
   say community reading is public) or the spec says reading them needs an
   account and the registration message stops claiming "Reading is unaffected".
   A decision, not a patch.
4. **A public review notifies nobody.** Only purchases, sales and forum replies
   call `notifications::notify` (`crates/app/src/routes/monetization.rs`,
   `community.rs`); the loudest thing a reader does to a work lands in silence.
   Test 16b is marked `test.fail()` and will go red the day it lands.
5. **Reading history has no door on a desktop.** `/library/history` works, but
   its only link is in the mobile "More" drawer: 0 `a[href="/library/history"]`
   elements exist in the DOM at 1280 px, signed in or out. Test 12b documents it.
6. **N5** — `check_abuse_status` is session-gated, not operator-gated: any
   account can probe any key's counter and block state. Wants the operator role
   plus an audit row (§11, §19).
7. **N6** — unknown `/api/v1/*` paths answer `200 text/html` with the index page
   instead of the JSON error envelope (§3.3); it also hides client bugs.
8. **Phase 2** — `public_search` ignores its query and can only return `[]`
   (`routes/external.rs:56`, `search.rs:144`); `/extensions` demands a session
   and answers 422, so the public gallery is closed; four `/me/*` doors answer
   422 where the convention is 401.
9. **Phase 3** — `create_bounty`/`list_bounties`/`claim_bounty` are the only
   dialect-unguarded functions in `crates/db/src/economy.rs` (they panic on
   PostgreSQL); no escrow, no credits check, `claim_bounty` reports success on a
   zero-row update.
10. **Phase 4 — bookkeeping.** `~/.config/lorehaven/pg-env` is missing, so no PG
    claim in either doc can be reproduced; restore a PG path, then update
    `docs/requirements.csv` (M15, M18, M19) and the counts in
    `docs/sessions/2026-09-17.md` §0.

Also worth a look while in the files: `IdentitySwitcher.svelte` is imported
nowhere (the switcher readers use is "Act as this" on each pseud card);
`POST /api/v1/exports` answers `privacy_acknowledged: false` even when the caller
acknowledged, because the response is built before the acknowledgement is
written; `check_abuse_status` derives `blocked` from `blocked_until.is_some()`
without comparing to now; malformed ids on public doors return 422 where unknown
ids return 404.

## Environment

Things that will cost a morning if discovered late.

- **Two cargo target dirs.** `~/.cargo-target/lorehaven` for ordinary work;
  `~/.cargo-target/lorehaven-review` for the release binary the e2e server runs
  (`~/.cargo-target/lorehaven-review/release/lorehaven`). The binary embeds
  `frontend/dist` at build time, so **build the frontend before the binary** or
  the browser tests run against a stale interface:
  `frontend/scripts/fe.sh build` then
  `CARGO_TARGET_DIR=~/.cargo-target/lorehaven-review cargo build --release`.
- **Never `pkill -f lorehaven-review/release/lorehaven`.** The pattern matches
  the shell that runs the command, so it kills the caller; if a test run is in
  flight it also kills the e2e scratch server and the rest of the run fails with
  28 connection refusals. Kill by pid, or match a narrower string.
- **The e2e scratch server runs `serve` without `worker`**
  (`frontend/e2e/serve-scratch.sh`), so queued exports and imports stay queued
  there. Export/import assertions must be about the request being listed, not
  about the file existing.
- **Invoke Playwright through node**: `node ./node_modules/@playwright/test/cli.js
  test use-cases.spec.ts --reporter=list` from `frontend/`. The
  `node_modules/.bin` shims are not executable on this filesystem.
- Browsers are installed under `~/.cache/ms-playwright` (chromium).
- **Rate limits are real** (`crates/app/src/limiter.rs`): auth 10 burst / 30 per
  minute per address, writes 20 / 60 per account, address-keyed buckets ×4. A
  seeding script that writes quickly gets `RATE_LIMITED` and a half-built
  fixture; sleep between writes or raise the limits in the instance config.
- **PostgreSQL cannot be verified here**: `~/.config/lorehaven/pg-env` does not
  exist and the `lh-review-pg` container has no reachable URL, so do not report
  PG results — mark them unverified instead.
- **Docker needs sudo** and cannot bind-mount the repo (the daemon's root cannot
  read `~/code`, which is a symlink into `~/mnt/thinkcentre`), so `act` cannot
  run the GitHub workflow locally. Run the workflow steps directly.
- **A local instance for hands-on checking** is in `/home/alvaro/lorehaven-demo`:
  <http://localhost:8180>, `dev@lorehaven.local` / `lorehaven-dev`, `./run.sh` to
  start and stop server + worker, one SQLite file, fixture catalogue seeded
  through the public API by `seed-demo-content.py` (six published works with
  chapters, ratings, reviews, two forum topics with replies). Throwaway; its
  `README.md` says what is in it.

## Conventions this work is held to

- A fix is not finished until its test has been **seen failing without it**.
- Findings are verified with the user and confirmed before implementation,
  especially for scraping and integration work; test approaches against the real
  thing before writing code.
- Keep `docs/verification.md` as the running evidence log (literal output, not
  summaries) and `docs/sessions/<date>.md` as the session record.
- Report literal measurements. When a claim cannot be reproduced, say so rather
  than inheriting a number.
- Never commit credentials, tokens or keys; redact them.
- Clear the pre-existing warnings in whatever tree you touch, not only the new
  ones; a red gate that predates the work is part of the work.
- Commit often, in small surgical changes.

## Where else to look

- `/home/alvaro/.hermes/plans/2026-09-17-lorehaven-stub-followup-plan.md` — the
  long-form plan this file summarises: status table, gate table, the phase plan
  (1c, 2, 3, 4) and the lessons from the leak round.
- `/home/alvaro/.hermes/plans/2026-09-17-lorehaven-remediation-plan.md` — the
  earlier remediation plan.
- `docs/spec.md` — the authority the code is measured against;
  `docs/requirements.csv` — requirement status; `docs/verification.md` —
  evidence; `docs/sessions/` — per-session records.
- `frontend/e2e/use-cases.spec.ts` — the twenty use cases, with the findings
  above encoded as tests (12b and 16b fail on purpose; 17 fails until item 1 is
  fixed).
