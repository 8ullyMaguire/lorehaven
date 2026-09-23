# 2026-09-23T190000+0200-work-aggregates-feature.md

**M46 — Work Card Aggregate Metrics**
**Status: SPECIFIED, NOT STARTED**

## What changed before this handoff

- M45 (Roadmap Consensus) fully implemented end-to-end: board, MaxDiff arena,
  TL>=1 gates, operator moves, seed script, one-vote-per-ballot, dedup.
  Deployed to thinkcentre. Migrations 0043/0044/0065 dialect drift fixed
  (`411d3d5`), 99-file cargo fmt sweep applied (`e6f1c55`).
- HEAD: `e6f1c55` (clean, formatted, all tests green).

## Task

Add public aggregate counts to every work card — **not averages, counts**. Per-work
totals visible to all readers, for author feedback:

| Metric | Source table | Exists? |
|---|---|---|
| Hits/views | new `work_view_log` or counter | No |
| Complete reads | `reading_progress` WHERE permille=1000 or `reading_status='finished'` | Yes (reuse) |
| Reactions | `work_reactions` | Yes |
| Kudos | new `work_kudos` (1 per reader per work) | No |
| Bookmarks | `bookmarks` WHERE subject_type='work' | Yes |
| Collection adds | `collection_items` JOIN collections | Yes |
| Review count | `review` WHERE is_public=1 | Yes |

**Decisions to confirm:**

1. **Hits vs views** — spec §10 says "Repeated refreshes do not inflate views without
   bound" and "Completion rate calculation excludes automated traffic." So views
   must be de-duplicated per reader per time window and filtered for bots. A
   `work_view_log` table with `(work_id, account_id NULL, ip_hash, ua_hash,
   is_automated, viewed_at)` and an aggregate `work_metric_aggregates` counter table
   maintained on insert (or via cron).
2. **Kudos** — not yet implemented. 1 per account per work, anonymous kudos not
   allowed (spec: "Count once per reader per target"). Table: `work_kudos(work_id,
   account_id PK)`.
3. **Aggregation strategy** — spec lists `work_metric_aggregates` as a table. Materialized
   counters updated by triggers or application-level on each event (rating added,
   bookmark added, etc.). A simple `INSERT ... ON CONFLICT DO UPDATE` counter table
   maintained in the application layer avoids trigger dialect drift.
4. **Visibility** — aggregate counts are public and shown on every `WorkCard`
   variant (full at minimum; row/compact show a subset like ★hits · ♡kudos).
5. **Author preference** — spec §9.5: "Work owners may disable display of public
   rating aggregates." Same flag should gate the entire metric bar.

**New tables (both dialects):**
- `work_view_log` — raw deduplicated view events (partitioned by day, capped).
- `work_kudos` — one row per (work, account).
- `work_metric_aggregates` — `(work_id PK, hits, complete_reads, reactions, kudos,
  bookmarks, collection_adds, reviews)` maintained incrementally.

**Endpoints:**
- `POST /api/v1/works/:id/view` — record a view (dedup + bot check).
- `POST /api/v1/works/:id/kudos` / `DELETE` — toggle kudos.
- `GET /api/v1/works/:id/metrics` — read aggregate counts (public).
- The existing work detail endpoint should embed `metrics` in its response.

**Frontend:**
- `WorkCard.svelte` gets a new `metrics` prop on `WorkSummary`.
- New chip row: `👁 1.2k · ★ 47 · ♡ 12 · ↩ 3 · 📚 5 · ✍ 8`.
- Full variant: all metrics. Row/compact: top 2-3 (hits, kudos, reviews).
- `WorkDetail.svelte` page: same metric bar near the title.

**Tests:**
- Backend: view dedup (same reader twice in window = 1), bot exclusion, kudos
  toggle, aggregate counter increments, author opt-out hides metrics, dialect
  parity migration test.
- Frontend: WorkCard renders chips, author hides them, kudos button toggles.

**Files:**
- `migrations/sqlite/00NN_work_metrics.sql`, `migrations/postgres/00NN_work_metrics.sql`
- `crates/db/src/work_metrics.rs` — view/kudos/aggregate CRUD.
- `crates/app/src/routes/works.rs` — `record_view`, `toggle_kudos`, embed metrics.
- `crates/app/src/routes/metrics.rs` (or fold into works).
- `crates/app/tests/milestone_46.rs` (or whatever milestone number maps to this).
- `frontend/src/lib/components/WorkCard.svelte` — prop + chip row.
- `frontend/src/routes/WorkDetail.svelte` — metric bar.
- `frontend/src/lib/components/WorkCard.test.ts` — chip visibility tests.
- Update `docs/requirements.csv` once implemented.

**Seed script:** `scripts/seed_work_metrics.py` backfills counts from existing
`bookmarks`, `collection_items`, `work_reactions`, `review` tables so production
shows correct numbers on first deploy.

## Environment notes

- Build/test locally in `~/code-local/rust/lorehaven`.
- Deploy: `ssh thinkcentre`, pull, `cargo build --release -p lorehaven-app`, run
  `./target/release/lorehaven migrate`, seed, restart service.
- `cargo check --workspace` is clean; `cargo test -p lorehaven-db` is 40/40.
- Frontend `npx vitest run` is 285/285.
- thinkcentre Postgres has migrations 0043/0044/0065 applied; the dialect parity
  test enforces identical index names and columns between sqlite and postgres.

## Out of scope

- Year-in-review compilation (private analytics).
- Time-decayed popularity (the `Popularity` strategy in meta_ranker already exists;
  it consumes these aggregates but its weighting is a separate concern).
