# Lorehaven

A self-hosted home for fanfiction: read without an account, write without a
publishing queue, import the library you already have, and take it offline when
you want it.

Lorehaven is built from a written plan rather than by accretion. The plan, the
architecture decisions, and an honest log of what has actually been *executed*
all live in this repository.

## Status

**Milestone 0 (running application) and Milestone 1 (design system and
navigation) are complete. Milestones 2–18 are not implemented.**

That is the honest summary; `docs/requirements.csv` lists every requirement with
a status, and `docs/verification.md` records the evidence behind each claim.

| Milestone | Area | State |
|---|---|---|
| 0 | Repository, tooling, running application | Done, tested |
| 1 | Design system and navigation | Done, tested |
| 2 | Accounts, pseuds, privacy, age policy | Schema and policy exist; no endpoints |
| 3–18 | Writing, reading, jobs, imports, offline, library, search, discovery, community, events, governance, economy, extensions, integrations, operations | Not implemented |

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
