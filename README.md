# Lorehaven

A self-hosted home for fanfiction: read without an account, write without a
publishing queue, import the library you already have, and take it offline when
you want it.

Lorehaven is built from a written plan rather than by accretion. The plan, the
architecture decisions, and an honest log of what has actually been *executed*
all live in this repository.

## Status

**Milestones 0 through 5 are complete**: the running application, the design
system, accounts and pseuds, writing and publishing, the reader with ratings,
reviews, notes and history, and the job queue with its content-addressed blob
store, encrypted secret storage and worker. Milestone 6 is **partly built**: the
import framework, the safe fetcher that stands between a pasted URL and the
network, the first source adapter, the chapter sanitiser and the source-credential
surface are implemented and tested, and the pages that would let a reader use
them are not. Milestones 7–18 are not implemented.

That is the honest summary; `docs/requirements.csv` lists every requirement with
a status, and `docs/verification.md` records the evidence behind each claim.

| Milestone | Area | State |
|---|---|---|
| 0 | Repository, tooling, running application | Done, tested |
| 1 | Design system and navigation | Done, tested, with two gaps listed in the verification log |
| 2 | Accounts, pseuds, privacy, age policy | Done: API and pages, tested, and driven in a browser |
| 3 | Drafts, chapters, publishing, revisions | Done, tested, driven in a browser — tag `v0.04-publishing` |
| 4 | Reader, ratings, reviews, notes, history | Done, tested, driven in a browser — tag `v0.05-reader` |
| 5 | Jobs, storage, secret encryption, outbox delivery | Done, tested, driven in a browser — tag `v0.06-jobs` |
| 6 | Imports, source credentials, batches, preservation | **Partly built**: API, importer and one source adapter tested; pages not built and nine of the ten planned sources not ported |
| 7–18 | Offline, library, search, discovery, community, events, governance, economy, extensions, integrations, operations | Not implemented |

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

## Licence

AGPL-3.0-or-later.
