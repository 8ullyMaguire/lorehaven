# Standing goal — build Lorehaven to completion

> This is the brief an implementing agent works from. It supersedes nothing:
> `docs/spec.md` says what the platform must do, `docs/requirements.csv` is the
> row-by-row authority for what is done, `docs/plans/remaining-work.md` is the
> forward build plan, and `docs/handoff.md` says where the build stands today.
> Where this brief and those disagree, those win and this file is corrected in
> the same commit.

## The goal

Act as a senior product-minded full-stack engineer and UI/UX designer. Continue
implementing Lorehaven until it is fully complete — **no stubs** — with full unit,
integration and E2E test coverage, and a polished, verified result that a real
person can use.

Then, in this order and as part of the same product:

1. **The bots** — the platform-neutral fanfic-archivist bot core (M54), speaking
   only to `/api/v1`, never to the database.
2. **The local software** — `~/code-local/rust/lorebook`, a Tauri desktop app that
   reads and writes Calibre libraries directly. Already real, at M1 of M14; see
   *Scope, and what this brief adds* below.
3. **The Chromium extension** — a Manifest V3 browser extension. Does not exist
   yet in any repo under `~/code` or `~/code-local`; it is new work, described in
   `docs/plans/bots-extension.md`.

Follow best practices. Commit often — one milestone or one coherent fix per
commit, never a sweeping "misc" commit. Tag milestones. **Deploy to ThinkCentre
on clean milestones.** On clean milestones, update `docs/handoff.md`, `README.md`
and the docs. Keep the code maintainable. Use your own best judgment for
remaining decisions, and record the ones that were genuinely open in the handoff
rather than leaving them implicit.

## What "complete" means

A requirement is complete when it has code that runs, tests that drive the real
path, and evidence recorded. Not:

- a field that exists on a struct but that no configuration can set;
- a migration that adds a column no query reads;
- a unit test on a function nothing calls;
- a test asserting a value the spec never promised.

Two requirements failed this bar in the last stretch and are the reason it is
written down. `rec.mode` was a documented operator control that `FileConfig` had
no section for, so a deployment could not set it — and 2901 tests passed, because
a value no configuration can set is indistinguishable from a correct one. An
expired claim stayed in the state its own expiry matched on. In both cases the
machinery was present and the wiring was absent, and both were invisible until a
gate that had never run was finally run.

## The gates — a green result is not a pass

A result is green only when the **exit code** is zero. `2901 passed, 0 failed`
with exit 101 is a *failed* run: a target failed to build. An aggregate that
counts `test result:` lines is blind to a target that never ran. Read the exit
code, then the tally.

Every change passes, before it is committed:

| Gate | Command | Runs |
|---|---|---|
| fmt | `cargo fmt --all --check` | always |
| clippy | `cargo clippy -p <touched> --all-targets` | always, no diagnostics |
| SQLite | `cargo test -p <touched> --no-fail-fast` | always, for schema and query changes |
| PostgreSQL | same, with `LOREHAVEN_TEST_PG_URL` set | **for anything touching SQL** |
| doctests | `cargo test -p <touched> --doc` | **standalone — never mixed with target flags** |
| E2E | Playwright, one suite at a time | for anything user-facing |

**Two single-backend blind spots have already cost real defects.** A PG-only INT4
into `i64` decode, and a doctest target that never compiled. Neither is visible
on SQLite. A change that can only fail on one backend must be gated on both, and
doctests run as their own command because cargo refuses to mix `--doc` with
target-selecting flags.

Full-workspace runs on both backends before a milestone is called done, and before
a deploy. If a gate is red, clear it — including issues that predate the change.
A red gate that predates the work is part of the work.

## The two blind spots, named

1. **PostgreSQL-only type breaks.** `SELECT 1` is INT4 on PG and was decoded into
   an `i64`. `CAST(1 AS BIGINT)` works on both. A name absent from both backends
   has no dialect divergence to notice, so verify the *dialect parity* test
   passes rather than assuming a new table is symmetric.
2. **Doctests are not part of `cargo test`.** They run under `cargo test --doc`
   or in a full-workspace run that includes doc targets. The SQLite gate never
   ran them, so a `Serialize` derive with no import compiled nowhere and failed
   only on the PG pass.

## Scope, and what this brief adds

`docs/requirements.csv` has 687 rows: **299 fully tested, 117 verified E2E, 227
built but not verified end to end, 39 planned, 4 deliberately unsupported, 1
evaluated and rejected.**

*(Re-derived 2026-09-30. The previous figures here — 640 rows, 53 planned, and a
list naming M53/M54/M57 as open — were all stale: M57, M54, M53-03/-04 and the
four `M38.doctor_*` rows have since been built, and `docs/requirements.csv` is
the authority. Re-derive rather than believe:*

```sh
python3 -c "import csv,collections; \
  r=list(csv.DictReader(open('docs/requirements.csv'))); \
  print(collections.Counter(x['status'] for x in r))"
```*

**Every remaining planned row is now M45.** That is a single dominant term, not
a spread across five milestones, and it changes the shape of the remaining work:
there is no ordering decision left inside the platform. M11-17 (`a33cf4b`), M54,
M53 and the doctor rows are all done.

**M45 is a gaps-review block, not spec §45.** The ID namespace and the spec
section share a number by coincidence — spec §45 is *Directory Category
Governance*, and all six of its `M45-45g*` rows plus `M45-45a` are already
`implemented-fully-tested`. What is outstanding is `M45-10` … `M45-55`, the
"gaps review" items: Scout-value curation credit, exposure floors, MMR variety
re-ranking, operator taste-leakage, reason-tagged kudos, fic-finder bounties,
keystone authors, cross-language supply, sister-instance federation, the Discord
bot, and the GDPR/DSA compliance tooling. Conflating the two has already cost one
wrong line in this file; the CSV is the authority.

**The 230 `implemented-locally-tested` rows are not free.** They are built, not
verified end to end, and they need a verification pass, not new code. They
inflate each remaining line item rather than forming a phase of their own.

**The 4 `unsupported` rows are never counted as remaining.** They are a standing
exclusion — work that was consciously declined, not work deferred.

**No decisions are open. Both are closed, and neither closed by an assumption.**

- *Is all of the spec in scope?* **Yes — the owner's call, 2026-09-30.** All 45
  M45 rows are in scope, so the build runs to the whole spec rather than a
  chosen subset. Five of them (M45-10, -11, -13, -15, -49) have since moved to
  `implemented-locally-tested` under spec §47, so 40 remain `planned`.
- *M11-17 first, or M45?* Settled by the work itself: M11-17 is built and
  verified, so M45 is what remains.

The four `unsupported` rows stay excluded — that is a standing decision about work
consciously declined, not a question about scope.

The tail does not move at the platform core's rate: the remaining rows are
first-of-their-kind integrations (a bot port, a DSAR export, a credential vault),
where the rate is one to two orders of magnitude below the repetitive
schema-and-route work the earlier throughput figure was measured on. Averaging the
two produces a confident, wrong number.

### Beyond the site: bots, local software, extension

The bots (M54) are **in the spec already** — §23.2, amended by ADR 0024. They are
port, not invention, so build them against `/api/v1` and let the spec decide the
rest. The bot never touches the database and never receives a password; the link
flow is a short-lived challenge, a Lorehaven-side pseud+scope confirmation, and
revocable tokens.

**The local software already exists and is not in this repo:**
`~/code-local/rust/lorebook` — a Tauri desktop app, Rust core
(`lorebook-core`, `lorebook-calibre`, `lorebook-interop-check`) plus a SvelteKit
UI, which reads and writes **Calibre's own `metadata.db` directly** so there is
no import step and no second copy of the books. It is at **M1 of M14**, built and
verified at the library level, with one unverified hop: the Tauri window does not
paint in this environment, so "webview loads the UI, IPC round trip" is untested.
`docs/PLAN.md` in that repo covers M2–M14.

It is a **separate project, not a Lorehaven crate**. Do not add it to this
workspace. Do not merge it in. Do not add a Lorehaven dependency from it. Do not
add a Lorehaven dependency *to* it. Do not make Lorehaven require it.

**The Chromium extension does not exist.** No `manifest.json` in any repo under
`~/code` or `~/code-local` (excluding `node_modules`) at depth ≤ 3. The nearest
prior art is `~/code/js/userscripts` — 14 Tampermonkey scripts, notably
`ao3_download_buttons.user.js` and `ao3_kudosed_and_seen_history.user.js` — which
show the interaction patterns the extension formalizes, not the architecture.

Design it as its own repo with its own spec, and give it a spec section before
building it. Build order: Lorehaven `/api/v1` surface → bot core (M54) → extension
against the same API. Build the extension last, it depends on the others.

## Order of work

State the order and *why* it is that order — dependency order, not preference. Give
two or three concrete examples of what breaking it costs.

1. **Finish the M11-17 block** (M57). The wire types exist; the endpoint does not.
2. **The bot core** (M54-01, -02), against `/api/v1` only.
3. **The Chromium extension**, after its own spec section, against the same API.
4. **M53-03/-04** — the credential-vault-gated adapters.
5. **M45** — the discovery-and-community growth block, the dominant term. Spec
   §47 and `docs/plans/m45-ranking-substrate.md` cover its first five rows
   (M45-10, -11, -13, -15, -49), and **all seven steps of that plan are now
   committed** — including step 7, which wires `rank_works` into
   `GET /api/v1/discovery` and gives `log_impression` its first caller. Three of
   the five rows are `implemented-fully-tested`; M45-10 (`scout_value`) and
   M45-15 (exposure floor) deliberately stay `implemented-locally-tested` because
   the route does not call them, and the CSV says so per row. (This
   line previously read "the metadata exchange block", which is M57's job
   description copied forward by mistake. M45 is the gaps-review block: Scout
   credit, exposure floors, MMR re-ranking, taste leakage, kudos, bounties,
   keystone authors, the Discord bot, GDPR/DSA compliance tooling.)

*(Updated 2026-10-01: M57's rows are all `implemented-fully-tested`, M54-01/-02
are `implemented-verified-e2e`, M53-03/-04 are `implemented-fully-tested`, and
the M45 ranking substrate is wired end to end through §47 and §48. What is left
is the remaining 40 planned M45 rows and then the browser extension body.*

**37 of those 39 rows still have no spec section at all** — `docs/spec.md` had zero
hits for `concierge`, `keystone`, `Fic Finder`, `view-as-persona`, `DSAR`, `DSA`
and a dozen more. They existed as CSV rows whose entire specification was
`Gaps review B1`, from `docs/spec-gaps-design-review-2026-09-22.md`, which is
marked *review draft — proposed, not adopted*. So the next step is
**specification, not code**, and `docs/plans/m45-gaps-adoption.md` is that plan:
eight phases in dependency order, one spec section each, with the reasoning for
why the groups are ordered that way. §49 (taste signal: M45-14, -16, -19, -20,
-35) is committed, and **§49.2 is built**: migration 0099 plus
`crates/db/src/tag_confirmation.rs`, with M45-16 promoted to
`implemented-fully-tested` on a test that fails for exactly one reason — remove the
confirmation filter from the route and that test goes red while every other in the
file stays green. Phase 1's remaining steps are M45-19 (tasting menu) and M45-20
(history import); their tables are already in 0099.**

Each step's exit condition is in `docs/plans/remaining-work.md` and its row in
`docs/requirements.csv`.

## Standing rules for every implementer

- **No stubs.** Every branch returns a real value or refuses. A stub that returns
  a plausible constant is worse than an unimplemented function, because it is
  indistinguishable from a finished one at the call site.
- **A requirement is done when it has evidence, not when a status flips.** A row
  moves to `implemented-fully-tested` with an `evidence` entry naming the test.
  A row whose machinery is unreachable — nothing writes the table, nothing
  enqueues the job — is not tested by unit tests on the dead functions.
- **Record every passed test in `docs/requirements.csv`**, in the same commit as
  the code it evidences.
- **Corrections land in place, in the same commit as the code.** A doc nobody
  corrects is worse than no doc, because the next reader trusts it.
- **Never fabricate a result.** A blocker reported honestly beats invented output
  every time.
- **One E2E suite at a time.** Kill a competing Playwright process before
  starting a new one.
- **Clean pre-existing diagnostics in the tree you touched.** Not just the ones
  you introduced.
- **Release tags (v1.0, v1.x) are the user's decision alone.** Never create, move
  or delete a 1.0 tag unprompted. Milestone tags are fine.
- **The full suite runs once at the end of a task**, not after every edit. Use
  the narrowest command that answers the question first.

## Where things are

| Path | What |
|---|---|
| `/home/alvaro/code-local/rust/lorehaven` | this repo — the site |
| `/home/alvaro/code-local/rust/lorebook` | Tauri desktop app, separate project |
| `crates/lore-metadata` | shared wire types (M11-17d) |
| `migrations/sqlite/`, `migrations/postgres/` | both dialects, written together |
| `frontend/e2e/` | Playwright suites |
| `docs/spec.md` | what the platform must do |
| `docs/requirements.csv` | the row-by-row authority |
| `docs/plans/remaining-work.md` | the forward build plan |
| `docs/handoff.md` | where the build stands |
| `docs/known-gaps.md` | 12 open decisions |
| `docs/verification.md` | the evidence log |

Deployment: SSH to the ThinkCentre production box. Check the local system for
deployment status and you will conclude the wrong thing.

## The correction rule

A statement here found to be wrong is fixed in place, in the same commit as the
code that made it wrong. A genuinely ambiguous spec gets an ADR under
`docs/adr/`. A plan nobody corrects is worse than no plan, because the next
reader trusts it.
