# Handoff — forum-first build (M31 complete, M32–M35 next)

Date: 2026-09-20. Read this with `docs/plans/junior-implementation-plan.md` §15a
and `docs/spec.md` §35.

## What just happened

The forum is being made a first-class surface of the creative ecosystem
(spec §35, new). The user's design decision, now spec'd in §35.0:

- **One conversation, one place.** Work pages get a typed-vote reaction bar
  and a "Discuss" link to a linked forum topic; no comment box in
  `ThreadOnly` mode (the new default for *new* works once an operator opts
  in; existing works keep `comments_only`).
- **Typed votes with meta-moderation** (Slashdot-style) replace generic
  likes on community surfaces.

### Commits (this session, oldest first)

- `e6411c9` spec: §35 + §17 revision note (work discussion modes)
- `60bfbe8` plan: §15a briefs for repo M31–M35
- `4f7bea4` ledger: M31-01..06 rows
- `10721b5`, `eb7d2d1` rate limiter fixes (share state, async middleware)
- `53c1318` M31 implementation (migration 0038, domain, db, routes, config)
- `43f7ed9` M31 acceptance tests 6/6 + **limiter middleware order fix**
- `7329d59` ledger: M31 flipped to implemented-locally-tested
- `6c23e3f` M31 frontend: reaction bar, Discuss link, backlink card, editor mode picker
- M32 implementation (migration 0039, domain, db, routes, config, 8 tests)
- M32 test fix: widened auth/write rate-limiter bursts in test config to
  avoid collision across 8 parallel tests sharing the process-global bucket
  map (limiter buckets keyed by 127.0.0.1).

## State

- **M31 complete** (backend + frontend): work discussion modes
  (thread_only/comments_only/both), reaction bar (one vote per pseud,
  changeable/retractable), linked topics (idempotent per work),
  comment→topic migration tool (authorship/order/timestamps preserved,
  idempotent), server-side refusal of comments on ThreadOnly works.
- **6/6 acceptance tests pass** (`cargo test --test milestone_31`).
- **M32 complete** (backend only): typed votes with per-category taxonomy,
  per-account 24h rolling budget scaled by trust level, meta-moderation
  by TL4+ with vote-weight decay (never removes voting ability),
  transparency tiers (aggregates-only by default, individual votes visible
  to author/moderator/TL4+), karma derived from weighted votes with
  monthly inactivity decay (display-only — never gates trust/ranking/
  credits), and a workspace-grep containment test proving karma rows are
  only read by storage + the display route.
- **8/8 acceptance tests pass** (`cargo test --test milestone_32`).
- Workspace `cargo check --workspace` clean, `cargo fmt` applied, working
  tree clean at `6c23e3f`.
- E2E suite (frontend/e2e/extended.spec.ts, 20 tests) passed earlier
  against the thinkcentre scratch server — not re-run after M31 frontend.
- M32 frontend (`ForumVoteBar.svelte`, `VoteBudget.svelte`,
  `KarmaBadge.svelte`) is planned (plan §15a.2) but not yet implemented;
  the backend is fully tested and ready for it.

## What's left in the forum-first arc (plan §15a)

1. **M31 E2E (owed):** run the extended E2E suite on thinkcentre to cover
   the new frontend surfaces (reaction bar, Discuss link, editor picker).
2. **M32 frontend:** extend `ForumTopic.svelte` with a vote bar, add
   `ForumVoteBar.svelte` / `VoteBudget.svelte` / `KarmaBadge.svelte`,
   add vote endpoints to `api.ts`.
3. **M33** thread modes (AMA, reading group, critique circle, wiki pin,
   collab fiction + promote-to-work, prompt, character voice) — migration
   0040.
4. **M34** spoilers/warnings/readability — migration 0041 (post drafts and
   scheduled posts tables already exist from M12; verify before adding).
5. **M35** discovery/health/UX/federation — migrations 0042–0043. Federation
   scope first (constrains outbound payloads).
6. Tag per milestone (`v0.31-work-threads` … `v0.35-forum-federation`),
   update §17's revision note when M35 lands, keep `docs/verification.md`
   current.

## Environment quirks (unchanged, still true)

- `~/code` is an SSHFS symlink to thinkcentre: builds/tests are slow
  (a full `cargo test --test milestone_31` cycle ≈ 15–25 min; run as
  background process with notify, poll the log file). Never create venvs or
  try to execute `node_modules/.bin` through the mount.
- Playwright E2E must run ON thinkcentre over SSH (see
  `frontend/e2e/serve-scratch.sh`; the config spawns the debug binary on a
  scratch SQLite db, port 8173).
- Deployed instance: thinkcentre `127.0.0.1:8081` (systemd --user service
  `lorehaven`), worker running, admin credentials in `~/.hermes/.env` on
  thinkcentre. The deployed binary predates M31 — rebuild + redeploy when
  the user wants the new endpoints live.
- Lint false positive: the write_file/patch tool's linter runs rustc with
  Rust 2015 edition and reports `async fn` errors on every file — ignore
  those; `cargo check` is the real gate.

## Gotchas discovered this session

- **Axum layer order in `classified()`**: `.layer()` adds outermost-last.
  The limiter must be added FIRST (innermost) so it runs after the
  `Classified` extension is inserted. Getting this backwards makes the
  limiter fail every request closed with "no declared rate-limit class".
- Comment POST returns **200 with `{id, receipt}`**, not 201; the receipt
  carries no `created_at` (read it back from the comments list if needed).
- `forum_categories` has no repo-level `create_category`; tests seed it
  with raw SQL (see `milestone_31.rs::seed_category`).
- Python-in-heredoc through the terminal tool chokes on `§` and em-dashes
  inside Rust comments — write patch scripts to /tmp files instead.
- A stray `~/` directory appears in the repo when cargo writes to a bad
  path over SSHFS; delete it with `rm -rf './~'` (quoted, from inside the
  repo) before committing.
- **SSHFS compilation is I/O-bound**: a rustc process can sit at 0.2% CPU
  for 15+ min. Don't kill it — just wait, or commit and push to let the
  user run CI on thinkcentre.
- **Rate limiter buckets are process-global statics** (`GLOBAL_BUCKETS` in
  `limiter.rs`), keyed by IP address. Parallel integration tests sharing
  127.0.0.1 exhaust the default auth burst (10) and write burst (20).
  Test harnesses must widen `config.rate_limits.auth` and `.write` when
  registering multiple accounts or casting many votes across tests.

## How to resume

1. `cd /home/alvaro/code/rust/lorehaven`, confirm clean tree at `6c23e3f`.
2. Run the M31 E2E suite on thinkcentre (item 1 above).
3. **M32 is complete** (backend only): `cargo test --test milestone_32`
   passes 8/8, `cargo test --test route_inventory` passes 2/2,
   `cargo test --workspace --doc` clean.
4. Proceed to M32 frontend (extend `ForumTopic.svelte` with vote bar,
   add `ForumVoteBar.svelte` / `VoteBudget.svelte` / `KarmaBadge.svelte`,
   add vote endpoints to `api.ts`) and then M33 following plan §15a.3,
   migration 0040 in both dialects (domain, db, routes, tests).
