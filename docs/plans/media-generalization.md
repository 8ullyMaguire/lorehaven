# Media generalization — implementation plan (M22–M26, spec §32)

Authority chain: `docs/spec.md` §32 > ADR 0019 > this plan > the session
handoff (`~/.hermes/plans/2026-09-16-lorehaven-media-redesign-handoff.md`).
Design rationale: `secondbrain: 10-projects-lorehaven-platform-redesign-spec.md`.

## Phase status

| Phase | Milestone | Content | Status |
|---|---|---|---|
| R1 | M22 | Migration 0024 (creators, media_creators, distributors, distributorships, media_collections, media_collection_items, media_editions, media_rights, quality_signals, works.format) + domain `media` module + 501 contract routes + milestone_22 contract tests | **skeleton landed** (this session) |
| R2 | M23 | Media query engine + `/api/v1` doors with filters (quality, dates, everything), cursors, ETag/304, Atom/RSS, OPDS, webhooks, grant-gated bulk export, JSON-LD/Dublin Core | owed |
| R3 | M24 | Anchored comments (paragraph/timestamp), orphaning, creator dashboard, half-star setting, shelf/works import adapters (Goodreads/StoryGraph CSV, Wattpad, AO3), bulk manuscript import, per-format goals | owed |
| R4 | M25 | Derivative pipeline (EPUB/PDF/text/OCR/transcode), full-text over transcripts+OCR, public-domain collections, optional lending (default off), vanished-source marking for derivatives | owed |
| R5 | M26 | Adult taxonomy behind §7.3/§7.6 gates, TTS narration editions, gallery mechanics for illustrated works | owed |
| R6 | — | Route aliases for any renamed paths, requirements.csv + verification.md + session docs per phase, ADR updates | continuous |

## Gate discipline (every phase, both dialects)

```bash
# SQLite (harness reads the env var at runtime — unset it)
unset LOREHAVEN_TEST_PG_URL && CARGO_TARGET_DIR=~/.cargo-target/lorehaven \
  cargo test --workspace --no-fail-fast

# PostgreSQL (scratch container lh-review-pg on 55432)
. ~/.hermes/plans/lhpg-env.sh && \
  cargo test -p lorehaven-app --no-fail-fast -- --test-threads=4

cargo fmt --all -- --check
CARGO_TARGET_DIR=~/.cargo-target/lorehaven cargo clippy --workspace --all-targets 2>&1 | grep -c warning  # 0
```

Full PG runs need `lhpg_drop_stale` first (orphaned `lh_test_*` databases
accumulate when a test panics past cleanup).

## Implementation notes per phase

- **R2:** the filter compiler sits beside the §15 query code
  (`crates/domain/src/query.rs`, `query_sql.rs`); reuse `works_index_terms`
  rather than a new index until EXPLAIN says otherwise. New db module:
  `crates/db/src/media.rs` (registered in `crates/db/src/lib.rs`). Replace
  the 501 stubs in `crates/app/src/routes/media.rs` one door at a time;
  delete the matching 501 assertion in `milestone_22.rs` as each body
  lands, replacing it with a behavior test.
- **R3:** adapters follow §11.1's source schema — a new source is
  `source_key` + adapter + credential kind, configuration not code.
- **R4:** lending mode is a config flag, default off, refused with the
  policy named; derivatives ride the existing job queue (§10.4) and
  content-addressed storage.
- **R5:** adult taxonomy is data behind existing gates; the TTS narration
  pipeline is an AI provider job (§23.7) with the budget guardrails
  (§22.11).

## Non-negotiables (from spec §0 and ADR 0019)

- Positivity-first feedback on every new surface, including anchored and
  timestamped comments.
- Trust levels are the only authority axis; quorum verifies external
  creators and decides lending policy.
- No purchased ranking; quality scores are filters, not leaderboards.
- Scraping exists only in the ingestion direction. Every door applies §7.6
  eligibility, including aggregations.
- Dual-dialect parity: mirror SQLite shapes exactly in the PG twin (TEXT
  timestamps, BIGINT counters, UUID ids in the content family), no JSON
  columns, `?` placeholders in `db.sql()` PG strings, honest multi-line
  SQL literals (never backslash continuations).
