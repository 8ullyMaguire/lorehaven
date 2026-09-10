# ADR 0001 — Stack

Status: accepted (Milestone 0)
Date: 2026-09-10

## Problem

Lorehaven must run comfortably on a small self-hosted machine — a 1 GB VPS or a
Raspberry Pi 4 — while serving an archive, a writing application, an import
pipeline and a community. It must be deployable by one person, backed up by one
person, and understood on a bad day by that same person.

The plan fixes the broad shape (spec §2.1): Rust + Axum on the server, Svelte +
TypeScript in the browser, SQLx for persistence, a modular monolith rather than
services. This record states why, and what we gave up.

## Decision

- **Rust with Axum + Tokio** for the server.
- **Svelte 5 + TypeScript + Vite** compiled to static assets, embedded in the
  server binary.
- **SQLx** for database access, with SQL written by hand.
- **A modular monolith**: one executable that also runs workers.
- **A domain crate with no I/O and no transport**, which every other crate may
  depend on and which depends on none of them.

## Alternatives considered

**Node/TypeScript with a framework such as Fastify.** The fastest path to a
working prototype, and the frontend and backend would share a language. Rejected
because the target is a 1 GB machine: a Node runtime plus a bundler plus a
database client is a materially larger idle footprint than a native binary, and
the deployment story becomes "install a runtime, then the app".

**Go.** Excellent for this shape — small binaries, easy concurrency, trivial
cross-compilation. Rejected because the type system is a poor fit for the parts
of this project that are *rules*: content eligibility, age policy, trust levels,
extension capabilities. Those are sets of states and transitions, and encoding
them as enums with exhaustive matching is how they stay correct under change. Go
would push them into runtime checks and comments.

**Python (FastAPI).** Would make the scraper and importer work easy, and the
project already lives near that ecosystem. Rejected on footprint and on the
same type-safety grounds.

**Separate frontend and backend processes (two deployments).** Rejected: it
doubles the operational surface — two systemd units, two health checks, two
upgrade steps — for no benefit at this scale. Embedding the built assets in the
binary means a deploy is "replace one file" (spec §22).

## Consequences

- **Good:** one artifact to deploy, back up and version. Startup is fast and
  idle memory is small. Policy is compiled, not interpreted. The domain crate
  is unit-testable without a database or an HTTP client, which is why the
  eligibility and age rules have tests today.
- **Cost:** the frontend is plain Svelte + Vite rather than SvelteKit, because
  there is no Node server to run. We therefore implement routing, data loading
  and forms ourselves (a small router exists in `frontend/src/lib/router.ts`).
- **Cost:** dual-database support costs real duplication. See ADR 0004.
- **Cost:** Rust build times are the price of the borrow checker. Mitigated with
  a shared `CARGO_TARGET_DIR` and `lld`.

## Conditions that would justify revisiting

- The domain's rules stop fitting in Rust's type system — for example if the
  extension host needs dynamic typing that code generation cannot bridge.
- The single-process model cannot keep a slow import from affecting reader
  latency, *and* running the worker mode as a separate process does not fix it.
- Measured idle memory exceeds the 1 GB profile even after the caching budgets
  in spec §23 are reduced.
