# Handoff — M56-01 instance accessibility mode; build recovered; no 1.0 tag

Date: 2026-09-25 (tip `5fb778e`). Previous handoff
(`2026-09-23T112310+0200-m51-media-resilience-m12-mentions-m7-device-delivery-handoff.md`)
remains the ADR-0024 baseline. `docs/handoff.md` carries the standing
milestone history.

## Read this first: the tree was broken and has been repaired

Three commits on `master` between `921a59d` and the tip — `cbd2aae`, `dcc7498`,
`72ee3e3`, `e994e34`, `827e25a` — left the workspace **uncompilable**. The
causes, so nobody repeats them:

- `72ee3e3` removed `rec_enabled_strategies` from `DiscoveryConfig` while
  `crates/app/src/rec_engine.rs` still read that field at two call sites. The
  substitution (`preferred_rec_engine: String`) was a half-finished rename of a
  field that has two consumers, not a replacement.
- `827e25a` added `pub mod middleware;` to `lib.rs` with no
  `crates/app/src/middleware.rs`, and edited `InstanceConfig` to drop
  `preset` in favour of `mode` while `FileConfig`'s loader still read
  `preset`.
- `dcc7498` carried a `pub mod shadow;` line for a `shadow.rs` that was
  never written. This is **M52-08 shadow-mode evaluation, still open.**

Repair: `git reset --hard 921a59d` (the last green M47 commit), then
`git cherry-pick cbd2aae dcc7498` to recover the M45-21 DNF feature, then
removed the dangling `pub mod shadow;` (`001431d`). DNF content was verified
byte-identical between the local recovery and the remote's copies before the
history was rewritten. `72ee3e3`, `e994e34` and `827e25a` were superseded and
the push was `--force-with-lease`.

**Nothing from those three is lost.** The recommendation-engine intent
(spec §16.1.5) is now written up properly as M52-08 in
`docs/plans/remaining-work.md` and in `docs/requirements.csv`, with the design
decisions that matter (per-pseud, not per-account; a `user_settings`
key/value table, not a column; write-time validation against the enabled
strategy set).

## What is now true

**M56-01 — instance accessibility mode** (`public` | `walled_garden` |
`private`) is implemented, tested, and documented.

- `InstanceMode` in `crates/app/src/config.rs`, default `Public`, parsed
  from `[instance] mode`. An unrecognised value **stops startup** rather than
  falling back to `public` — a misspelling that silently opened an instance
  would be found by a stranger.
- The mode is translated once, by `InstanceMode::access_policy()`, into the
  `AccessPolicy` whose `anonymous_reading_enabled` flag the existing
  eligibility service (`can_access_content`, spec §7) already reads. This is
  the load-bearing decision: three handlers had a hard-coded
  `AccessPolicy::default()` and were silently ignoring any instance setting.
  Those three — `routes/works.rs` (the reading decision), `routes/media.rs`
  (direct-door eligibility), `routes/meta.rs` (the published summary) — now
  read the configured policy.
- `settings.rs` and `auth.rs` also call `AccessPolicy::default()`, but for
  rating *ceilings*, which the instance mode does not change. Left alone
  deliberately; do not "fix" them into the mode by reflex.
- `/api/v1/meta` reports `policy.instance_mode` alongside the existing
  `policy.anonymous_reading`, so a client can tell a walled garden from a
  private instance without inferring one from the other.
- Documented in `docs/spec.md` §0.4.7 and `lorehaven.toml.example`.

**M45-21 — structured DNF reasons** recovered and green: 3/3 tests in
`crates/app/tests/m45_21_dnf.rs`.

## Verification

**This is the first session in which the full test suite has ever run to
completion.** All four layers are green:

| Layer | Result |
|---|---|
| `cargo test --workspace` | **1685 passed, 0 failed** (79 binaries) |
| `npx vitest run` (frontend) | **300 passed, 0 failed** (58 files) |
| `npx playwright test` (E2E) | **73 passed, 0 failed** |
| `cargo check -p lorehaven-app` | clean (warnings only, all pre-existing) |

- `cargo test -p lorehaven-app --lib config::` — 30/30, including 8 new
  instance-mode tests.
- `cargo test -p lorehaven-app --test instance_access_mode` — 5/5 through
  the real router.
- `cargo test -p lorehaven-app --test m45_21_dnf` — 3/3.
- `cargo test -p lorehaven-domain --lib` — 482/482 in **0.30s**, where it
  previously never returned.

## Three defects fixed beyond the accessibility mode

### 1. The search parser hung the entire test suite (`ff9fff9`)

`crates/domain/src/search.rs` had three independent bugs, all pre-existing,
all covered by tests that were *failing* rather than passing:

1. **Infinite loop on any parenthesised query.** `parse_terms` broke only on
   EOF. A group whose last term was followed by `)` left the cursor on the
   bracket — neither EOF nor a term — and the free-text fallback never
   advanced. `(tag:romance OR tag:angst)` spun forever. Earlier handoffs
   recorded this as a "slow `parse_range` test"; it was a hang, and it is
   why `cargo test --workspace` had never finished.
2. **`words:>10000` and `kudos:>=100` parsed as `Equal`** with the operator
   glued into the value. The grammar is `field : [op] value`; `:` was the
   equality operator with no look past it.
3. **`date:2026-01..2026-06` parsed as an equality on the whole string**,
   because `.` was not a value terminator.

`parse_terms` now breaks on `)` as well as EOF, the separator and the
operator are parsed as two steps, and `..` terminates a value while a lone
`.` does not — so `2026-01` and `0.5` survive.

**If a suite ever appears to hang again, check the search parser first.**

### 2. The Media unit test asserted link text the component does not render (`0575c22`)

The test looked for a link named "Atom"; the component says "Atom feed", and
`e2e/media.spec.ts` has always asserted the latter. The test was the stale
party.

### 3. The export-delete E2E deleted a row the worker still owned (`d624c15`)

The test waited for `.state` to be *visible*, which is true the instant the
export is queued, then asked the server to remove a row the worker still
owned. It read the refusal as "the delete did not work" rather than as the
race it was. Now it waits for `Ready` — the contract the EPUB download test
already asserts. Passes in 2.9s where it previously burned its full 15s
timeout and failed.

## Release status

No `v1.0.0` tag exists, locally or on the remote, and none should be cut
until Alvaro decides the site is at 1.0. The plan says so explicitly.

## Next concrete steps

1. `cd ~/code-local/rust/lorehaven/frontend && npx playwright test` — confirm
   73/73 still holds after the reset.
2. Rebuild release and deploy to thinkcentre once green.
3. M52-08: implement the per-user recommendation engine preference (spec
   §16.1.5), from a green tree this time. Start with the `user_settings`
   migration, then the resolver, then the Settings UI.
4. The M45 and M47 planned rows are untouched and remain the bulk of the gap.

## Files changed this session

- `crates/app/src/config.rs` — `InstanceMode` enum, `InstanceConfig.mode`,
  `[instance] mode` file parsing, 8 tests.
- `crates/app/src/routes/works.rs` — reading decision uses the configured
  policy.
- `crates/app/src/routes/media.rs` — direct-door eligibility uses it.
- `crates/app/src/routes/meta.rs` — reports the mode; summary uses it.
- `crates/app/tests/instance_access_mode.rs` — **new**, 5 acceptance tests.
- `docs/spec.md` — §0.4.7.
- `lorehaven.toml.example` — `[instance]` section.
- `docs/requirements.csv` — M52-08 and M56-01 rows.
- `docs/plans/remaining-work.md` — M52-08 write-up, M56-01 status, no-1.0 note.

## Things not to trust

- Any claim about E2E after `921a59d` that was not re-measured on this tree.
- The `pub mod shadow;` line, if it reappears: there is no `shadow.rs`.
- `rec_enabled_strategies` still exists and is still read. If a future change
  removes it, `rec_engine.rs` breaks.
- The patch tool's standalone syntax check on `.rs` files in this repo
  reports spurious `async fn is not permitted in Rust 2015` errors. `cargo`
  is the authority; the crate edition is 2021 and compiles.
