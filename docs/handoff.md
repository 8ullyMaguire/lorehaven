# Handoff

Where the tree is, what is finished, what is open, and what a successor needs
before touching any of it. Long-form detail lives in the plan files listed at the
end; this file is the short version that must stay true to the commit it names.

## The tree

The application code is at **24b5ee2**; the commits after it touch `docs/` and the
e2e spec only. Working tree clean. (This line deliberately names the app commit
rather than its own, so it does not go stale the next time a document is added.)

Gates at this commit:

| Gate | Measured |
| --- | --- |
| `cargo fmt --all -- --check` | clean |
| `clippy --workspace --all-targets -- -D warnings` | 0 warnings, 0 errors |
| SQLite workspace suite | **1223 passed / 0 failed / 13 ignored** across 47 binaries |
| the 13 ignored | network tests in `crates/scrapers/tests/live_verification.rs`, run deliberately, not by default |
| Playwright e2e | **30 passed** across `frontend/e2e/` (27 use cases, 2 journeys, 1 media) — and **no `test.fail()` markers left**: the last two, 12b and 16b, now assert the fixed behaviour |
| `svelte-check --tsconfig ./tsconfig.json` | 0 errors, 0 warnings |
| frontend vitest | 153 passed / 25 files |
| PostgreSQL | **not run — blocked**, see Environment |

The commit messages in this series are not a reliable source of test counts
(three commits in a row say "136 tests pass" while the real number was 1217 at
the time). Trust `docs/verification.md`, this file, or a fresh run.

## Finished

- `29cc5b4` — every remaining route stub replaced with a real implementation.
- `d62f8cd`, `8bf4b38` — the leak round: `get_public_work` draft leak, the
  `my_audit_log` cross-account leak, the anonymous moderation-report queue, the
  anonymous abuse-status door; plus `crates/app/tests/route_inventory.rs`
  (237/237 handlers declare an audience extractor, was 65/237).
- `8cecc36` — `frontend/e2e/use-cases.spec.ts`, the twenty ordinary use cases in
  a real browser, with the findings below encoded as tests.
- `ac22a89` — pointed the public search at `works_index_terms` and added a
  version guard to `ReaderSettings`. Half of it held: the guard is verified by
  e2e test 17, which failed before it and passes now. The search half did not —
  see `768df88`.
- `768df88` — the three defects the search path still had, each with a test that
  fails without the fix: the reindex job's payload shape (nothing was ever
  indexed), a query naming a column no migration creates (500 on a fresh
  database), and no anonymous visibility predicate (drafts served to strangers).
  Verified live: publish → reindex succeeded → 20 index rows → anonymous search
  finds the work with a real word count; a draft with index rows present, and a
  restricted work with rows left behind, are both withheld.
- The overnight round of 2026-09-18 — `6358720` (tell the author when a public
  review is delivered), `fb717fc` (a History link in the desktop nav),
  `fee36d6` (a guard against the settings panels discarding an in-flight edit) —
  reviewed claim by claim, and two of the three needed work:
  - the review notification is live and correct (author's inbox gets `kind:
    review` with the work id; a held review notifies nobody; a private one
    notifies nobody) **but fired on every delivered save**, so an edit
    re-notified the author. Fixed in `24b5ee2`; `milestone_12`'s
    `editing_a_public_review_does_not_notify_the_author_again` was seen failing
    without the guard (two identical items from one post plus one edit).
  - the History link is real: at 1280 px signed in, `nav.desktop` holds all
    eleven destinations including `/library/history`, visible (58 × 43 box).
    The claim looked false for a while because the instance serving it had been
    built *before* the frontend was rebuilt, so it was serving the older bundle
    — see Environment.
  - the panels guard did **not** hold: `edited` was set on change, consulted by
    the effect, and cleared at the end of that same effect run. Writing a value
    the effect reads re-queues it, so the flag survived exactly one flush and the
    next run re-seeded over the reader's edit. Three component tests pin both
    directions now (the edit survives a late server copy; a copy arriving after a
    save is still adopted), and the flag is cleared only by a save.
- Earlier in the same day: M26 TTS narration (`6b6c684`), M25 residuals
  (`81c6cf9`), shelf CSV imports and the tutorial book (`4b33acb`).

## Open, in the order I would take it

1. **Phase 1c — make the audience a declared value.** Extend
   `route_inventory.rs` into an explicit `(method, path) → audience, scoping`
   table with a spec citation per entry and assert the registered set equals the
   tabled set. The current harness measures *presence* of an extractor, not the
   *right* audience (see the plan's N2).
2. **N2b — decide the audience of the browsing doors.** Anonymously:
   `GET /works/{published}/comments` → 401, `/forums` → 401, `/topics/{id}` →
   401, `/groups` → 401, `/taxonomy*` → 401, while `/search` → 200 and a
   published work → 200. Either those six become `optional` (amend the spec to
   say community reading is public) or the spec says reading them needs an
   account and the registration message stops claiming "Reading is unaffected".
   A decision, not a patch.
3. **Index hygiene is still the worker's job, not the route's.** The route now
   refuses to serve a non-public work whatever the index holds, which is the
   safety net. The race underneath remains: a `Reindex` job that lands after a
   withdrawal repopulates rows for a work nobody may see, and the deindex event
   is a best-effort second. Consider having the reindex handler skip a work that
   is not published+public, so the index stops carrying rows it must never serve.
4. **Search semantics worth pinning down.** Multiple words are OR-ed
   (`term LIKE 'a%' OR term LIKE 'b%'`) while `score` counts the matched terms,
   so a two-word query ranks by how many words hit — intended, or should it be
   AND? And matching is prefix-per-word, so "light" finds "lighthouse" while
   "house" does not.
5. **N5** — `check_abuse_status` is session-gated, not operator-gated: any
   account can probe any key's counter and block state. Wants the operator role
   plus an audit row (§11, §19).
6. **N6** — unknown `/api/v1/*` paths answer `200 text/html` with the index page
   instead of the JSON error envelope (§3.3); it also hides client bugs. (Cost
   this review a probe: a mistyped path answered with HTML 200 rather than a
   404, which reads as success to a careless `curl | jq`.)
7. **Phase 2** — the search half is done (`public_search` takes a real query and
   filters by lifecycle and visibility). Left: four `/me/*` doors answer 422
   where the convention is 401, and `/extensions` demands a session with 422, so
   the public gallery is closed to visitors.
8. **Phase 3** — `create_bounty`/`list_bounties`/`claim_bounty` are the only
   dialect-unguarded functions in `crates/db/src/economy.rs` (they panic on
   PostgreSQL); no escrow, no credits check, `claim_bounty` reports success on a
   zero-row update.
9. **A held review that a later edit gets delivered notifies nobody.** The
   notification now waits for a review that was not already public, so the
   sequence "held write, then a delivered edit" is silent until the reviewer
   edits again. Narrow (it needs the gate to reverse itself for the same
   reviewer), and the fix is to compare against the stored delivery outcome
   rather than `is_public` — noted in the code comment. Low priority.
10. **Search cannot find a work by its own title.** `rebuild_work_index` takes one
    `body_text` string and indexes it (`crates/db/src/search.rs:34`), so the term
    table holds revision prose only — no title, summary or tag. On the demo
    instance, `?q=cartographer` finds nothing although a published work is called
    *The Cartographer's Apprentice*, while `?q=quiet` finds *The Use Case
    Chronicle* because "quiet" happens to occur in its text. A reader looking for
    a work by name is the most ordinary search there is. Extend the indexed text
    (title and summary into the term table) or add a weighed title predicate to
    the query; decide which with the user, since it changes ranking.
11. **Phase 4 — bookkeeping.** `~/.config/lorehaven/pg-env` is missing, so no PG
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

- **A freshly migrated database is the only honest yardstick.** The `works`
  table has **no `word_count` column** (chapter counts live on
  `chapter_revisions`), and no migration ever added one. An agent debugging a
  failing query may bolt a column onto its scratch instance with an ad-hoc
  `ALTER TABLE`, after which every check against that instance passes while a
  fresh install errors on the same code — which is exactly how a `500 INTERNAL`
  on every non-empty search query shipped as "verified". Compare
  `PRAGMA table_info(works)` between the instance you are told about and a fresh
  `migrate` before believing an on-instance verification, and prefer a test that
  builds its own database.
- **`worker` alone does not deliver outbox topics.** The topic handlers that turn
  `publish.index` into a reindex job are registered by `serve --with-worker`
  (`crates/app/src/server.rs`). A `serve` plus a standalone `worker` runs job
  kinds but defers every topic forever, so nothing is ever indexed. Run the local
  instance with `serve --with-worker`.
- **Two cargo target dirs.** `~/.cargo-target/lorehaven` for ordinary work;
  `~/.cargo-target/lorehaven-review` for the release binary the e2e server runs
  (`~/.cargo-target/lorehaven-review/release/lorehaven`). The binary embeds
  `frontend/dist` at build time, so **build the frontend before the binary** or
  the browser tests run against a stale interface:
  `frontend/scripts/fe.sh build` then
  `CARGO_TARGET_DIR=~/.cargo-target/lorehaven-review cargo build --release`.
  This bit the overnight round anyway: its instance answered with the *pre-change*
  bundle for nine hours, and a `document.querySelectorAll` probe against it
  "disproved" a fix that was in fact correct. Check which bundle is being served
  before believing a UI probe — `curl -s localhost:8180/ | grep -o
  'assets/index-[^"]*\.js'` against `ls frontend/dist/assets/index-*.js`; equal
  names mean the embed is current.
- **A restarted instance is not necessarily the one you started.** `kill <pid>`
  on a stale pid leaves the old process holding the port while the new one exits
  silently on a bind error, and the old one will happily serve the old build.
  After every restart confirm both: `ss -ltnp | grep 8180` names the listening
  pid, and `/health/ready` names the build it is running (a `.dirty` suffix means
  uncommitted changes were compiled in). This is how the stale bundle above was
  caught.
- **A background cargo build can be stopped, not slow.** Started through the
  tool's `bash -lic` wrapper it may sit in state `T` (`ps -eo pid,stat,args |
  grep -E 'cargo|rustc'`) and never resume; a release build waited ten minutes
  that way. Redirect stdin — `cargo build --release < /dev/null > /tmp/build.log
  2>&1` — and it runs normally.
- **Never `pkill -f lorehaven-review/release/lorehaven`.** The pattern matches
  the shell that runs the command, so it kills the caller; if a test run is in
  flight it also kills the e2e scratch server and the rest of the run fails with
  connection refusals. Kill by pid, or bracket the first letter
  (`pkill -f '[l]orehaven serve'`).
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
  PG results — mark them unverified instead. The two search SQL strings were
  changed in `768df88` and are therefore **SQLite-verified only**.
- **Docker needs sudo** and cannot bind-mount the repo (the daemon's root cannot
  read `~/code`, which is a symlink into `~/mnt/thinkcentre`), so `act` cannot
  run the GitHub workflow locally. Run the workflow steps directly.
- **A local instance for hands-on checking** is in `/home/alvaro/lorehaven-demo`:
  <http://localhost:8180>, `dev@lorehaven.local` / `lorehaven-dev`, `./run.sh` to
  start and stop it, one SQLite file, fixture catalogue seeded through the public
  API by `seed-demo-content.py` (six published works with chapters, ratings,
  reviews, two forum topics with replies). It runs `serve --with-worker` now, so
  publishing indexes and the public search returns results; `run.sh` was updated
  to start that one process. Throwaway; its `README.md` says what is in it.

## Conventions this work is held to

- A fix is not finished until its test has been **seen failing without it**.
- A guard is not finished until its test covers **both** directions: the thing it
  protects (the edit survives) and the thing it must not block (a later update is
  still adopted). One direction alone lets a guard that never fires pass.
- A test that can pass without the behaviour is not evidence. Two of this series'
  defects shipped under green tests: a query that could not return rows, and an
  assertion matching the label of the state *before* the action.
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
- Commit often, in small surgical changes, one theme per commit.

## Where else to look

- `/home/alvaro/.hermes/plans/2026-09-17-lorehaven-stub-followup-plan.md` — the
  long-form plan this file summarises: status table, gate table, the phase plan
  (1c, 2, 3, 4), the leak round's lessons, the review of `ac22a89` (§3c) and the
  review of the overnight round (§3d).
- `/home/alvaro/.hermes/plans/2026-09-17-lorehaven-remediation-plan.md` — the
  earlier remediation plan.
- `docs/spec.md` — the authority the code is measured against;
  `docs/requirements.csv` — requirement status; `docs/verification.md` —
  evidence; `docs/sessions/` — per-session records (`2026-09-18.md` is the
  overnight round and its review).
- `frontend/e2e/use-cases.spec.ts` — the twenty use cases; 12b (History on a
  desktop) and 16b (a review notifies its author) now assert the fixed behaviour
  and pass; 17 passes since the ReaderSettings guard landed.
