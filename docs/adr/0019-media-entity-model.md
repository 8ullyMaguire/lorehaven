# ADR 0019 — Media entity model and the media query API

Status: accepted (Milestone 22 skeleton)
Date: 2026-09-16
Supersedes: nothing; extends ADR 0002 (content model) and ADR 0004
(timestamp and identifier storage)

## Problem

Spec §32 (new) generalizes Lorehaven from fanfiction to written and
recorded media of any kind. Three questions needed decisions before
skeleton work could land:

1. Where do creators, distributors, collections, editions, rights and
   quality signals live in the schema, and which identifier family do the
   new tables use?
2. How do media collections relate to the existing M13 event collections?
3. What is the public API contract for "all media for an author, creator,
   distributor, collection, whatever, filtered by quality, date, and
   everything else"?

## Decision

1. **New tables join the content identifier family** (ADR 0004): UUID in
   PostgreSQL, TEXT in SQLite; TEXT timestamps; BIGINT/INTEGER counters;
   no JSON columns. Tables: `creators`, `media_creators`,
   `distributors`, `distributorships`, `media_collections`,
   `media_collection_items`, `media_editions`, `media_rights`,
   `quality_signals`; `works` gains `format` (default `prose`). The
   vocabularies (kinds, roles) are domain enums in
   `lorehaven-domain::media` and are refused at the edge when unknown.

2. **M13 event collections stay.** `media_collections` generalizes the
   *media* side (series, anthologies, reading lists, archive collections,
   challenge anthologies, preserved batches). A challenge anthology links
   across via `collection_kind = 'challenge_anthology'`; the two models
   are never merged, because an event's lifecycle (§18) is not a
   collection's lifecycle.

3. **One query engine, many doors.** `GET /api/v1/media` and the scoped
   doors (`/creators/{id}/media`, `/distributors/{id}/media`,
   `/media-collections/{id}/media`, `/canons/{id}/media`,
   `/spaces/{id}/media`) are the same engine with a pre-applied scope,
   all returning one `MediaRecord` shape. Quality is a filter dimension
   computed from `quality_signals` with instance-configured weights.

4. **Filters are the product; scores are not.** The composite quality
   score is never shown as a public leaderboard by default and can never
   be moved by credits, payments or trust (spec §0.3). This is the
   ranking philosophy for the generalized corpus.

5. **Interop is standards-based, not emulation.** OPDS acquisition feeds,
   Dublin Core (`?format=dc`), JSON-LD (`CreativeWork` family), Atom/RSS
   per query, grant-gated bulk export. Other sites' private API shapes are
   not emulated; other sites' content arrives through adapters
   (ingestion only).

## Consequences

- Every existing work is `format = 'prose'` after 0024; no reading path
  changes.
- The M21 `ai_training` column stays on `works`; licensing and lending
  live in `media_rights`. Two rights-ish places, two different questions —
  the author's AI statement vs the object's license.
- External creators are records, not accounts: attribution survives
  without a login, and verification is a quorum act (§19).
- The query engine must apply content eligibility (§7.6) inside every
  door, including collection and canon scopes — a filter can never leak a
  restricted work through an aggregation.

## References

- Spec §32 (M22–M26), review note
  `secondbrain: 10-projects-lorehaven-platform-redesign-spec.md`,
  migration 0024 (both dialects), `crates/app/tests/milestone_22.rs`.
