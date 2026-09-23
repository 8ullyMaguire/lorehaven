# M45 Roadmap Consensus — Handoff

**Date:** 2026-09-23
**Branch:** `main` (uncommitted changes)
**Status:** Domain + DB + routes compile; 3/3 domain tests pass; 2/2 integration tests pass.

## What's Done

1. **Domain layer** (`crates/domain/src/consensus.rs`):
   - `expected_score`, `elo_update`, `maxdiff_elo_updates` (pure functions)
   - `MaxDiffOutcome` struct: `{best_delta, worst_delta, unchosen_deltas}`
   - `STAGES` constant, `stage_arena_eligible`
   - 3 tests: symmetric expected score, equal-ratings symmetry, zero-sum ballot

2. **DB layer** (`crates/db/src/roadmap.rs`):
   - `upsert_card`, `find_card_by_title_normalized`, `list_cards`, `arena_candidates`
   - `create_ballot`, `fetch_ballot`, `mark_voted` (atomic one-vote), `apply_elo_and_counters`
   - `record_move`, `list_moves`, `insert_suggestion`, `update_card_stage`
   - Dual-backend (`match db.backend()`)

3. **Routes** (`crates/app/src/routes/roadmap.rs`):
   - `GET /api/v1/roadmap` — public board
   - `GET /api/v1/roadmap/arena` — get ballot (TL>=1)
   - `POST /api/v1/roadmap/arena` — submit vote (TL>=1, one-vote-per-ballot)
   - `POST /api/v1/roadmap/suggest` — suggest feature (TL>=1)
   - `POST /api/v1/admin/roadmap/move` — move card (operator/trust>=5)
   - `GET /api/v1/roadmap/changelog` — public move feed

4. **Migrations**: `0067_roadmap_consensus.sql` (both dialects, schema-consistent)

5. **Seed script**: `scripts/seed_roadmap.py` — dry-run verified (218 rows from requirements.csv)

## What's Left

- **Frontend**: SvelteKit pages for the board UI (no work started)
- **Integration tests**: Full vote flow needs auth + seeded cards (partial tests exist)
- **Deploy**: `ssh thinkcentre` → `cargo build` → `cargo run -- migrate` → `python3 scripts/seed_roadmap.py`

## How to Continue

```bash
cd ~/code-local/rust/lorehaven
# Full workspace builds:
cargo build
# Domain tests:
cargo test -p lorehaven-domain --lib -- consensus
# Integration tests:
cargo test -p lorehaven-app --test milestone_45
# Seed local DB:
python3 scripts/seed_roadmap.py
```

## Files Changed (uncommitted)

- `crates/domain/src/consensus.rs` (new)
- `crates/domain/src/lib.rs` (added `pub mod consensus`)
- `crates/db/src/roadmap.rs` (new)
- `crates/db/src/lib.rs` (added `pub mod roadmap`)
- `crates/db/src/governance.rs` (added `is_operator`)
- `crates/app/src/routes/roadmap.rs` (new)
- `crates/app/src/routes/mod.rs` (added `pub mod roadmap`)
- `crates/app/src/server.rs` (wired roadmap routers)
- `migrations/sqlite/0067_roadmap_consensus.sql` (new)
- `migrations/postgres/0067_roadmap_consensus.sql` (new)
- `crates/app/tests/milestone_45.rs` (new)

## Pre-existing Test Failure (NOT from M45)

`migrate::tests::the_two_dialects_declare_the_same_columns_and_indexes` fails on
migration 0043 (sqlite missing `idx_instance_fingerprints_valid`). Unrelated to M45.
