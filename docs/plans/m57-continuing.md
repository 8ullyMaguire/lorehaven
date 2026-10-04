# M57 — Continuing, and the honest shape of "coming back" (spec §57)

**Status:** spec written (`docs/spec.md` §57, commit `fd4becc`). Not yet implemented.
Executable by an LLM with no other context.

## What this row is

`docs/plans/100-ideas-scope.md` cluster one: the reader-retention features that
read data this instance already has.

## The finding that shapes the work

Nearly all of §57 is **rendering existing data**. Before writing a line, check
what is already there — this plan's job is to stop the implementer adding a table
that already exists:

| §57 clause | already exists |
|---|---|
| 57.1 Continue Reading | `reading_progress` (`migrations/sqlite/0004_reading.sql:38`): `account_id`, `pseud_id`, `subject_type`, `subject_id`, `chapter_id`, `content_revision`, `paragraph_anchor`, **`position_permille`**, `device_id`, `version` |
| 57.2 length | `chapter_revisions.word_count` (`migrations/sqlite/0003_works.sql:111`) |
| 57.3 completion badge | `works.completion` (§8.2): `in_progress \| complete \| hiatus \| abandoned` |
| 57.5 reason | §16.1 `reason` field — stored, never rendered |
| 57.6 Surprise Me | §16.10 surprise-me mode — specified, never implemented |
| 57.7 new in fandoms | an existing join plus a time window |
| 57.4 DNF | **the only new state** |

The line numbers above were read out of the schema for this plan, not remembered.
Two of them are worth stating because they contradict the spec's prose:

- **The column is `position_permille`, not `position_fraction`.** §9.3's prose says
  `position_fraction`; the schema says `position_permille INTEGER NOT NULL
  DEFAULT 0`. Use the schema. An implementer who trusted the prose would write
  `position_fraction / 100.0` against a column that does not exist, and on
  PostgreSQL that is `42703` while SQLite reports something less legible.
- **`works.completion` carries no CHECK constraint** (line 48 is
  `completion TEXT NOT NULL DEFAULT 'in_progress'`), so the four values of §8.2 are
  a convention the application enforces, not the database. §57.3's badge therefore
  needs a total `match` with a fallback arm; an unknown value must render as
  "unknown", never as one of the four.

`words_per_minute` does **not** exist in `crates/app/src/config.rs` yet. §38.1
requires it (`If a value affects behavior ... it lives in Config`), so adding it is
part of this work, not a lookup.

## Step 1 — migration 0116, both dialects

One new table. DNF is a per-reader status, so it is a row keyed on
(reader, work) like the rating beside it, and **it carries no public counter.**

```sql
-- reader_work_status: the reader's private disposition toward a work.
-- DNF lives here rather than in a new table because it is the same SHAPE as the
-- rating: one row per (reader, work), set by that reader, never public.
CREATE TABLE reader_work_status (
    id              TEXT/UUUID PRIMARY KEY,
    pseud_id        TEXT/UUUID NOT NULL REFERENCES pseuds(id) ON DELETE CASCADE,
    work_id         TEXT/UUUID NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    -- 'dnf' is the only value today. Not an enum column, not a boolean: the
    -- obvious next value is 'dnf_again' (read it, gave up twice) and a schema
    -- that cannot hold it forces a migration.
    status          TEXT NOT NULL,
    -- PRIVATE. Never aggregated, never in a public API response, never shown to
    -- the author. The test that enforces this is the reason the column exists in
    -- this form.
    private_reason  TEXT,
    created_at      TEXT/TIMESTAMPTZ NOT NULL,
    UNIQUE (pseud_id, work_id)
);
CREATE INDEX reader_work_status_work ON reader_work_status (work_id);
```

**There is deliberately no `dnf_count` column and no public aggregate.** §57.4 is
explicit that a public DNF count is a quality judgement about an author published
by readers, and §19.15's feedback-drought rules exist because this archive decided
not to do that to writers. The absence is the design; a later implementer will be
tempted to add the counter, so the comment says why not.

`reader_work_status` is the right home rather than a `dnf` table because a private
reading note per chapter (list item 20) wants the same privacy shape, and one table
with a documented private column is easier to keep private than two tables.

## Step 2 — domain types

**File:** `crates/domain/src/continuing.rs` (new)

```rust
pub enum WorkCompletion { InProgress, Complete, Hiatus, Abandoned }

/// §57.1. `progress` is a fraction of the WORK, not of the chapter.
pub struct ContinueReading {
    pub work_id: String,
    pub work_title: String,
    pub chapter_id: String,
    pub chapter_title: String,
    /// 0.0..=1.0 across the whole work.
    pub progress: f64,
    pub chapters_read: i64,
    pub chapters_total: i64,
    /// §9.3: when devices disagree, present a choice rather than picking silently.
    pub conflicted: bool,
    pub updated_at: String,
}

pub struct WorkLength {
    pub word_count: i64,
    /// word_count / Config.words_per_minute, labelled an ESTIMATE.
    pub reading_minutes: i64,
    /// False when a rating ceiling suppressed the estimate (§57.2).
    pub estimate_available: bool,
}
```

`ContinueReading.progress` is a work-level fraction and `WorkLength` has an explicit
`estimate_available` rather than `reading_minutes: Option<i64>` — because the failure
mode is a card that renders "about 0 min" when it should render nothing, and a
zero-minute reading time is the number a naive `unwrap_or(0)` produces.

## Step 3 — the store

**File:** `crates/db/src/continuing.rs` (new)

| function | the rule it enforces |
|---|---|
| `continue_reading(db, pseud)` | most recent `reading_progress` row; §57.1 |
| `work_length(db, work_id, reader)` | §57.2 — returns `estimate_available: false` under a ceiling |
| `set_dnf(db, pseud, work_id, reason)` | §57.4 — reason stored, never returned to another reader |
| `clear_dnf(db, pseud, work_id)` | §57.4 — reversible, restores eligibility |
| `dnf_excludes(db, work_ids)` | §57.4 — the read path for recommendation suppression |

### The SQL hazards this repository has already paid for

Three, all recorded in `docs/plans/REMAINING-2026-10-03.md`:

- **`created_at` on `reader_work_status` is TEXT on SQLite and TIMESTAMPTZ on
  PostgreSQL.** Read the neighbouring migration before writing the bind. A bare
  `$1` against a TIMESTAMPTZ column is `42883` (three occurrences here).
- **An unaliased subquery in `FROM` is `42601` on PostgreSQL and fine on SQLite**
  (three occurrences: `hit_rate.rs`, `analytics.rs`, `media_resilience.rs`).
  `continue_reading()` will want a correlated subquery for `chapters_total`;
  alias it. `scripts/check-pg-subquery-alias.py` catches it.
- **`accounts.id` / `pseuds.id` decode as `String` on SQLite and `Uuid` on
  PostgreSQL.** Any test helper that reads an id needs per-engine decode, the way
  `crates/app/tests/flow_dashboard.rs` already does it. This cost four 500s during
  the M45-23 pass because the masked error named no column.

## Step 4 — routes

```
GET  /me/continue            57.1, 200 with {continue: null} when nothing
PUT  /me/continue/dismissed  57.1, stays dismissed
GET  /works/:id/length       57.2
PUT  /works/:id/dnf          57.4
DELETE /works/:id/dnf        57.4
```

All five are `RouteClass::Read` or `Write` under `flow_dashboard`'s constraints.
`GET /me/continue` returns **200 with a null body**, never 404: "you have nothing to
continue" is an answer, and 404 would make an empty library look like a broken route.

**`PUT /works/:id/dnf` must not return the reason in any response.** §57.4's test is
the enforcement; the reason goes in, and the response carries only the status.

Register `GET /me/continue` in `crates/app/tests/route_inventory.rs` so it is
declared as well as mounted.

## Step 5 — frontend

**`src/routes/Home.svelte`** — the banner, first, above the feed:

```
Continue Chapter 4 of 19 · Stars Fall Softly · 21%     [dismiss]
```

- Progress rendered as a **work-level** fraction. The test asserts finishing a
  chapter does not reset it to zero.
- Absent entirely when there is nothing to continue.
- Never rendered for a signed-out visitor.

**`src/lib/components/WorkCard.svelte`** (new, or fold into the existing card):
`{word_count} words · about {n} min` plus the completion badge.

The badge maps §8.2's four values to four *visually distinct* treatments. `hiatus` and
`abandoned` must not look alike: one is a pause the author announced, the other is
the absence of an announcement, and §57.3 requires the reader to be able to tell.

**`src/routes/Discover.svelte`** — the reason line (§57.5) and the Surprise Me
control (§57.6).

## Step 6 — tests

**Files:** `crates/app/tests/continuing.rs`, `crates/app/tests/dnf.rs`,
`frontend/src/routes/ContinueBanner.test.ts`

Each row is paired with the implementation it rules out. Rows 1, 4, 6 and 9 are the
ones that matter.

| # | case | rules out |
|---|---|---|
| 1 | progress is a fraction of the **work**; completing a chapter does not reset it | per-chapter percentage |
| 2 | banner shows work, chapter and progress; absent when nothing to continue | an always-present empty card |
| 3 | dismissing persists across a reload | a dismissal that is session-only |
| 4 | a rating ceiling makes `estimate_available` false, and the card renders **nothing** where the estimate was | CSS-hidden estimate |
| 5 | the estimate is labelled "about", and changing `Config.words_per_minute` changes the rendered number | a hard-coded 250 |
| 6 | `hiatus` and `abandoned` render different badges | a generic "incomplete" badge |
| 7 | a `dormant` derived status (§8.8) does **not** produce an `abandoned` badge | conflating derived with real |
| 8 | TOC carries per-chapter length | nothing |
| 9 | a DNF reason is absent from every API response another reader can see | a leaked private note |
| 10 | DNF suppresses recommendations and **survives a taste recompute** | suppression by rank, not by fact |
| 11 | clearing DNF restores eligibility | a one-way door |
| 12 | every rendered recommendation has a reason, and the reason comes from the signal | a template chosen after ranking |
| 13 | Surprise Me returns nothing below the quality floor and still honours §16.4 | surprise as "no rules" |
| 14 | "New in your fandoms" is empty rather than absent with no fandoms | an ambiguous missing section |

Case 9 is the one to write first: it is the clause most likely to be quietly broken
by a later "helpful" change, and it is the reason the column is named `private_reason`.

Case 7 is the one that needs the derivation to actually run in the test — drive
`updated_at` far enough into the past for §8.8 to mark the work dormant, then assert
the badge is absent.

## The gates

```
python3 scripts/check-pg-subquery-alias.py crates
python3 scripts/check-pg-subquery-alias.py --self-test
python3 scripts/check-uncast-pg-placeholders.py crates
python3 scripts/check-pg-uuid-casts.py
python3 scripts/check-page-headings.py
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
```

## Verification

Both engines, per-suite:

```
# SQLite (default — LOREHAVEN_TEST_PG_URL unset)
cargo test -p lorehaven-app --test continuing --test dnf
cargo test -p lorehaven-db --test migration_catalogue

# PostgreSQL
sudo docker start lh-pg-test
LOREHAVEN_TEST_PG_URL='postgres://lorehaven:lorehaven@127.0.0.1:55433/postgres' \
  cargo test -p lorehaven-app --test continuing --test dnf
unset LOREHAVEN_TEST_PG_URL

# frontend, then the journeys -- through fe.sh, never playwright directly
cd frontend
node ./node_modules/vitest/vitest.mjs run
./scripts/fe.sh build
cd .. && CARGO_TARGET_DIR="$HOME/.cargo-target/lorehaven" cargo build --release --bin lorehaven
cd frontend && ./scripts/fe.sh e2e
```

**Always `./scripts/fe.sh e2e`, never `playwright test` directly.** The binary embeds
`frontend/dist` at compile time, so a `vite build` alone changes nothing that is
served. Running Playwright by hand tests a binary that predates the edit — and
`fe.sh`'s staleness guard exists to stop exactly that. This cost two mutation runs
in the last pass that appeared to prove nothing.

Then mutation-prove cases 1, 4, 7 and 9 — and **confirm the mutated build actually
ran**. A mutation that does not compile proves nothing, and three of four mutation
attempts in an earlier pass failed to compile.

## What this deliberately does not do

No streak counter, no reading-speed calibration, no email digest, no inline
content-warning filtering. §57.8 gives the reasoning for each; they are decisions,
not omissions.