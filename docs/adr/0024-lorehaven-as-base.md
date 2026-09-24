# ADR 0024: Lorehaven as the base for the from-scratch spec — and what it adopts from FicNexus

- Status: accepted
- Date: 2026-09-24

## Context

Two codebases claim the same product territory. FicNexus
(`~/code/rust/ficnexus`, plus `fanfic-scrapers` and `fanfic-archivist-bot`)
was built July 25 – September 9, 2026: ~109k LOC Rust + ~89k LOC frontend,
89 migrations, ~150 write routes, a 107-site adapter crate, deployed behind
Cloudflare. Lorehaven was started September 10, 2026 as a rebuild to a written
spec: ~160k LOC Rust + ~31k LOC frontend, 488 commits in two weeks, dual
SQLite/PostgreSQL from day one.

A consolidated from-scratch specification was written (2026-09-24, session
copy in `/tmp/ficarchive-from-scratch-spec.md`) that keeps the Lorehaven
architecture and engineering discipline and folds in the FicNexus features
that the gaps analysis (`docs/spec-gaps-ficnexus.md`) identified. This ADR
records the decision that followed from it and the direction it gives the
remaining build.

## Decision

1. **Lorehaven is the base.** The from-scratch spec is adopted as the target
   state of *this* repository, not as a new codebase. Adapting Lorehaven to
   it is mostly completion; adapting FicNexus would be architectural surgery
   (single-dialect PostgreSQL + pgvector + tsvector everywhere, load-bearing
   Redis, XP/levels/ranks to remove, ~150 write routes with opt-in
   enforcement per its own `docs/design/REBUILD-NOTES.md`).

2. **Adapter count stays an outcome, not a claim** — but the FicNexus adapter
   work is a port source, not a rewrite. The `fanfic-scrapers` crate
   (107-site FanFicFare-parity, MIT-compatible portions) is the reference for
   porting adapters onto Lorehaven's safe-fetcher `SiteAdapter` trait, one
   adapter per milestone batch, each with frozen HTML fixtures and live
   verification where permitted. `FFF_PARITY.md` is the checklist.

3. **The recommendation platform gets a strategy registry.** FicNexus's
   design (`docs/design/brainstorm-03-scraper-rec-platforms.md` in that repo)
   — `RecStrategy` trait, RRF blend (k=60), per-strategy config, failing
   strategy skipped not fatal, `rec.mode = legacy | pluggable` rollout with a
   golden legacy-parity test — is adopted as the shape of spec §16.1's
   "baseline engines". The current `discovery::blend` is the legacy mode the
   golden test freezes.

4. **The companion bot is a port, not a rebuild.** `fanfic-archivist-bot`'s
   platform-neutral core (commands, pagination cache, token store, render
   matrix) is the reference for spec §23.2's bot-client framework; the bot
   stays a thin REST client over `/api/v1` and never touches the database.

5. **The remaining FicNexus gap items not yet absorbed into spec.md are
   adopted as spec amendments** (see below): they are already in the
   from-scratch spec and most exist in spec.md only partially or not at all.

## Spec amendments adopted with this ADR

- **§16.1a Strategy registry and blend** (new): `RecStrategy` trait +
  registry + RRF blend + golden legacy-parity test + `rec.mode` config.
- **§11.16 porting backlog** (new): adapter porting batches from
  `fanfic-scrapers`, fixture requirements, live-verification policy,
  `FFF_PARITY.md` as checklist.
- **§23.2 is amended**: reference adapters cite `fanfic-archivist-bot` as
  the port source; Discord/Telegram/Matrix first, IRC/Slack/Fediverse after.
- **M48–M52 plan rows** (see `docs/plans/remaining-work.md`): rec registry,
  adapter porting, bot port, OpenAPI publication, body search (already
  partly landed via M35-06 zero-result tracking — verify and complete).

## Consequences

- The build continues in this repo; no code moves from FicNexus except as
  ported adapter logic (rewritten against the safe fetcher) and the Elo math
  already ported for M45 (ADR 0023 precedent).
- `docs/spec-gaps-ficnexus.md` items are now tracked either as adopted
  amendments (above) or as explicit non-adoptions recorded in that file;
  nothing stays silently unaddressed.
- Estimated remaining effort to the from-scratch spec, at this repository's
  observed agent-driven pace (~11 requirements/day, ~35 commits/day):
  **6–9 weeks concentrated**, plus a 2–3 week backgroundable tail for the
  full 107-adapter port. At single-developer human pace, 3–5× that.
- FicNexus, `fanfic-scrapers`, and `fanfic-archivist-bot` remain read-only
  references. Do not run cargo/git there; `~/code/rust/*` is partly SSHFS
  (see memory: build only in `~/code-local`).
