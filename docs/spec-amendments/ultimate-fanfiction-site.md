# Ultimate Fanfiction Site → Darkhold

**Status:** accepted · **Date:** 2026-10-19

This project was designated the base for the Ultimate Fanfiction Site, and the site is now
called **Darkhold** — *the dark side of fanfiction*. The designation stands, and this note
points at the two documents that now govern the work.

| | |
|---|---|
| **the specification** | `~/secondbrain/10-Projects/ultimate-fanfiction-site-spec.md` — Darkhold v3, 27 sections, the design and its reasons |
| **the implementation plan** | `~/secondbrain/10-Projects/darkhold-implementation-plan.md` — 255 task cards across M0–M32, the build order |
| **the split specification** | `docs/spec/` in this repository, with `docs/spec/00-index.md` as the index and the reserved skeleton |

**The filename `ultimate-fanfiction-site-spec.md` is retained deliberately.** This note is the
pointer, and the path is what it points at; the file keeps its old name so the pointer stays
valid. The *project* is Darkhold.

## What each source contributes

- **This repository (the base)** — the requirement ledger, the domain model, the governance
  design, the reading experience, the configuration contract, the 404-not-403 doctrine, the
  two-engine gate.
- **ficnexus** — the ingestion and search module: the 107-site FanFicFare-parity adapter
  architecture, the `_from_html` family, body full-text search, `main_char_attr`, the OPDS
  catalogue, the strategy registry and RRF fusion.
- **kindred** — the taste module: the memory discipline, the recommender and the arena's
  algorithms, the snapshot privacy model, the degradation-honesty rule.

Both are consumed as **services and references, not libraries**: kindred is a *reference
implementation* whose algorithms are reimplemented in Rust and checked against it, not a port
target.

## Which document wins

This repository's own specification (`docs/spec.md`) governs the parts it already covers —
reading, tags, warnings, community. **The Darkhold specification governs the cross-cutting
discovery and taste features**, and the implementation plan governs the order.

## The conversion is in place

The rename was done in M0 (ADR 0027), the history was kept, and the 703-row ledger is re-
triaged in place rather than restarted (ADR 0028).
