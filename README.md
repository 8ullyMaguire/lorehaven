# Lorehaven

A self-hosted home for fanfiction: read without an account, write without a
publishing queue, import the library you already have, and take it offline when
you want it.

Lorehaven is built from a written plan rather than by accretion. The plan, the
architecture decisions, and an honest log of what has actually been *executed*
all live in this repository.

## Status

**2026-09-25 (ADR 0024):** this repository is the base for the consolidated
from-scratch specification. The honest one-line status, re-derived from
`docs/requirements.csv`: **194 of 255 requirement rows implemented**
(174 locally-tested, 20 fully-tested), **57 planned**, **4 deliberately
unsupported**, tags through `v0.51.0`. What remains is planned in
`docs/plans/remaining-work.md`: the recommendation strategy registry (M52),
adapter porting batches (M53+), the companion bot (M54), OpenAPI publication
(M55), the metadata exchange (M57, new — see below), and the M45/M47 planned
rows (M56).

**The metadata exchange** (spec §11.17, §15.17, §19.14, §2.3.1) was adopted
2026-09-25 after triaging an external proposal. Readers may offer metadata
signals about works they hold and receive community-curated canonical metadata
back. Its first rule is the one that matters: **a signal is a fact about a work,
never a fact about a reader** — reading history, ratings, notes, private library
membership and credentials are not fields the signal type has, so no
configuration can widen what leaves the instance. It is opt-in per instance and
independent of the analytics tier. The proposal as submitted could not be applied
— roughly a third of its section citations were wrong — so the reasoning,
corrections and rejections are in `docs/plans/metadata-exchange-triage.md`.

The build is organized as worked milestones that do not correspond one-to-one
to spec §-numbers — `docs/plans/junior-implementation-plan.md` §0 maps them.
`docs/requirements.csv` lists every requirement with a status (the row-by-row
authority, and the seed for the roadmap board), and `docs/verification.md`
records the evidence behind each claim.

| Milestone | Area | State |
|---|---|---|
| M0–M15 | Platform core: tooling, design system, accounts/pseuds, publishing, reader, jobs, imports, exports, library, taxonomy/search, discovery, positivity, community, governance, economy | Built and locally tested (see requirements.csv for per-row status) |
| M16–M26 | Marketplace, translation, public API surface, admin/ops, monetization, media skeleton, comments/CSV, archive mode, TTS | Built (contract + bodies; per-row status in requirements.csv) |
| M31–M35 | Forum series: categories, read state, thread modes, spoilers, forks, sparklines | Built and locally tested |
| M39–M47 | Directory, fork provenance, half-life, export CTAs, browse ordering, roadmap consensus (Elo board), work aggregates, user settings | Built (M45/M47 have planned residual rows) |
| M48–M51 | Media resilience: import rescue, health dashboards, mirrors, reverse search | Built — tag `v0.51.0` |
| M52–M55 | Rec strategy registry, adapter porting, companion bot, OpenAPI publication | Planned — `docs/plans/remaining-work.md` |
| M57 | Metadata exchange: signals out, canonical metadata in, auto-created taxonomy entities | Specified (spec §11.17, §15.17, §19.14) — not built |

No screen in this repository displays mock data. Pages that exist show real
values from the server; routes that are linked but unbuilt say so plainly.

## Requirements

- Rust 1.82 or newer (developed on 1.98)
- Node 20 or newer (developed on 26) — only to build the frontend
- SQLite (default) or PostgreSQL

## Quick start

```bash
cargo build                       # builds without the frontend (uses a placeholder page)
cd frontend && npm ci && npm run build && cd ..
cargo build                       # now embeds the real bundle

cargo run -- migrate              # apply migrations
cargo run -- seed --development   # a development account, pseuds and defaults
cargo run -- doctor               # is this instance healthy?
cargo run -- serve                # http://127.0.0.1:8080
```

Then look at:

```
/health/live      process liveness
/health/ready     database, schema and storage checks
/api/v1/meta      instance metadata, including the active content policy
```

## Command line

```
lorehaven serve               start the HTTP server
lorehaven migrate [--status]  apply or inspect migrations
lorehaven seed --development  write development fixtures (refuses production)
lorehaven doctor [--strict]   check configuration, database, storage, converters
```

Configuration precedence is **argument → environment → file → default**. Copy
`lorehaven.toml.example` to `lorehaven.toml`, or use `LOREHAVEN_*` environment
variables (see `.env.example`).

Production is strict by design: `serve` refuses to start if cookies would not be
`Secure`, if CSRF protection is off, if the development seeder is enabled, if the
public URL is not HTTPS, or if storage is still the relative development path.
Each refusal names the setting to change.

## Repository layout

```
crates/
  domain/     identifiers, error taxonomy, resource policies — no I/O, no transport
  db/         SQLite and PostgreSQL access, migrations, repositories
  app/        CLI, configuration, HTTP surface, embedded frontend, worker entry point
frontend/     Svelte 5 + TypeScript, built to static assets and embedded in the binary
migrations/
  sqlite/     one set per dialect, kept identical by a test
  postgres/
docs/
  adr/                  architecture decision records
  design/THEME.md       "The Reading Room" — the visual specification
  spec.md               the 18-milestone implementation plan every decision is measured against
  spec-gaps-ficnexus.md features the older FicNexus build has that this plan does not; review draft, not adopted
  tutorial/             build it yourself, milestone by milestone
  requirements.csv      every requirement and its status
  verification.md       what has actually been run, and what has not
  plans/
    remaining-work.md           the forward plan: what is left, in what order
    metadata-exchange-triage.md an external proposal reviewed, corrected and partly adopted
    junior-implementation-plan.md  the historical milestone map
```

## Design

The interface is **"The Reading Room"**: warm paper, deep evergreen, muted
copper, bookish headings over clean interface text, and almost no decoration in
the reader. `docs/design/THEME.md` is the specification;
`frontend/src/styles/tokens.css` is its implementation. Three presets ship —
Reading Room, After Hours, Clear Day — and reader appearance is deliberately
independent of the site theme.

## Contributing to your own instance

This is meant to be self-hosted software. Every limit and toggle is a
configuration value with a documented default rather than a number in the code,
and `lorehaven doctor` reports the state of the machine it is running on.

## Running the gates

```sh
cargo fmt --all -- --check
cargo clippy --workspace --all-targets > /tmp/clippy.log 2>&1   # stderr matters
cargo test  --workspace --no-fail-fast  > /tmp/test.log   2>&1
```

Redirect with `2>&1` and read the file. `rustc`, `clippy` and `cargo test`
report warnings and errors on **stderr**, so a check piped through
`grep -c warning` with `2>/dev/null` in front of it returns a confident zero
from an empty stdout. That mistake sat in this repo's gate for several
sessions, reporting "0 warnings" while the tree held 30 — three of them real
defects, including a test that had never run. If a gate has never once failed,
suspect the gate before you trust it. Background: `docs/verification.md`.

A green E2E deserves the same suspicion. `frontend/e2e/coverage.spec.ts` asserted
that the export list was empty after deleting one export, on an account shared
with the test above it — so it could never pass, and the failure read as a
product bug. Assert the specific thing you changed, using a stable per-row
handle, and count rows before and after.

## Licence

AGPL-3.0-or-later.
