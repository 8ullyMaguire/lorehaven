# Plan — roadmap card bodies (spec §44 amendment)

Implements `docs/spec-amendments/roadmap-card-bodies.md`.

**Status: not started. This plan has never been executed.** Every symbol it
references was verified against the tree on 2026-09-28; every command's
expected output is written down. Where a step needs a symbol that does not
exist yet, it is marked **[NEW]** and the step says so.

Read `docs/plans/README.md` §2 for the house rules this inherits. The ones
that bite here:

- Every migration exists twice with identical ids
  (`the_two_dialects_define_the_same_migration_ids`).
- Every statement is written twice, once per dialect, via `match db.backend()`.
- `db.sql(...)` borrows; use `sql_owned` for assembled statements.
- Bind only `String`/`i64`/`Option<String>`; UUIDs read as `id::text AS id`.
- Svelte 5 runes. Never `export let`.
- Run the frontend with `bash frontend/scripts/fe.sh <cmd>`.

## 1. The order, and why

```text
1  migration (both dialects)      0090_roadmap_card_body.sql
2  domain + repository            Card.body, six queries, find_card_by_id [NEW]
3  routes                         board body, detail route, suggest accepts body
4  frontend                       RoadmapCard.svelte + the board link
5  the journey, by hand
6  tests that pin what the journey proved
7  requirements.csv               the body column, and 667 bodies
8  the brainstorm seed            162 new cards
```

The migration is first because every other step reads the column. The CSV
bodies are step 7, deliberately *after* the code: a 667-row writing task
should not block the schema from landing, and the code is verifiable on its
own while the prose is being written. The brainstorm seed is last because it
depends on the CSV having the column at all, and because it is the step that
depends on an answer nobody has given yet (§5).

## 2. Step 1 — the migration, both dialects

### 2.1 `migrations/sqlite/0090_roadmap_card_body.sql`

```sql
-- Roadmap card bodies (spec §44.1, amendment
-- docs/spec-amendments/roadmap-card-bodies.md).
--
-- Dialect: SQLite.
--
-- WHAT THIS IS. One column on `roadmap_cards`. §44.1 defined a card as "one
-- idea, stated in one sentence", and the board rendered the title alone. The
-- preservation brainstorm gave the arena 162 ideas whose entire argument is
-- in a paragraph that had nowhere to go, so a card gained a body: a page of
-- prose saying what the feature is and why it exists. The board still lists
-- the title; the detail view shows both.
--
-- NOT NULL DEFAULT '' is deliberate and is the whole compatibility story:
-- 667 cards exist, none of them has a body, and none of them may break. A
-- NULL body would force every reader to branch on Option<String> and every
-- writer to supply a value, in exchange for a distinction (absent versus
-- empty) that no surface renders differently. Empty is a real state, rendered
-- as a placeholder.
--
-- NOT a separate table. A card's body is edited with the card, travels with
-- it through a stage move, and is read on every detail view. A side table
-- makes every read a join and puts the question of a card with a row in one
-- and not the other permanently on the table.
--
-- RETENTION. No cascade, nothing references this column, nothing is deleted
-- by it. Cards are never deleted (§44.1); a body dies when its card does.
ALTER TABLE roadmap_cards ADD COLUMN body TEXT NOT NULL DEFAULT '';
```

### 2.2 `migrations/postgres/0090_roadmap_card_body.sql`

```sql
-- Roadmap card bodies (spec §44.1, amendment
-- docs/spec-amendments/roadmap-card-bodies.md).
--
-- Dialect: PostgreSQL.
--
-- The counterpart of migrations/sqlite/0090_roadmap_card_body.sql. Read that
-- file first: it carries the reasoning.
--
-- PARITY. `the_two_dialects_declare_the_same_columns_and_indexes` in
-- crates/db/src/migrate.rs parses CREATE TABLE bodies and inline REFERENCES.
-- It does NOT parse `ALTER TABLE ... ADD COLUMN` -- there is no "ADD COLUMN"
-- in the runner's parser, and the two dialects declare this column
-- identically, so this migration is invisible to that test in both files.
-- That is a known blind spot in the check, not a licence to skip parity:
-- the column name, order and default are identical above by inspection.
--
-- TEXT NOT NULL DEFAULT '' matches every other prose column in this schema
-- (ADR 0004, and the 0085 files' note that timestamps are TEXT here too).
-- The migration is a single ALTER, so it takes a table lock briefly; at 667
-- rows that is microseconds.
--
-- RETENTION. As the SQLite file: no cascade, nothing deleted by it.
ALTER TABLE roadmap_cards ADD COLUMN body TEXT NOT NULL DEFAULT '';
```

### 2.3 Verify

```bash
cargo test -p lorehaven-db --lib migrate:: 2>&1 | tail -20
```

Expected: all `migrate::` tests pass, including
`the_two_dialects_define_the_same_migration_ids` and
`the_two_dialects_declare_the_same_columns_and_indexes`. The new migration
appears in the catalogue:

```bash
cargo run -q -- migrate --status 2>&1 | grep -c 0090
```

Expected: `0` (not yet applied to a fresh dev DB — `--status` lists *pending*;
if it prints `1`, the migration was already applied and the dev DB is stale,
which is fine, note it and move on).

```bash
ls migrations/sqlite/0090_roadmap_card_body.sql migrations/postgres/0090_roadmap_card_body.sql
```

Expected: both paths printed.

**The number is 0090, and not a smaller one.** Verified 2026-09-28 against
the tree:

- **0084** — reserved by 0083 for `bot_registrations.token_id` (gap D5); see
  the NUMBERING note in `migrations/sqlite/0085_story_identity.sql`.
- **0086** — **already taken.** `migrations/{sqlite,postgres}/0086_body_audience.sql`
  exist untracked: the in-flight M59 §7.7 work. This was found by running
  `git status` while committing, which is the only reason it was found at all.
  `the_two_dialects_define_the_same_migration_ids` would NOT have caught a
  same-number collision in a *new* file — it compares the two dialects against
  each other, so two 0086 files pass it happily and the failure arrives later,
  as a checksum collision in the migration ledger.
- **0087, 0088** — claimed by the M59 plan's later phases
  (`docs/plans/crawling-retention-preservation.md`, the numbering note at its
  line 14).

So: check `ls migrations/*/` for the next free number *at the moment you
start*, and do not trust this document's 0090 if time has passed.

## 3. Step 2 — domain and repository

### 3.1 `Card` gains the field

`crates/db/src/roadmap.rs`, `pub struct Card` (line ~11). Add after `title`:

```rust
    /// §44.1: what the feature is and why it exists. The board lists the title
    /// alone; the detail view shows this. Empty is valid — a card created by
    /// the suggest endpoint is title-only, and 667 seeded cards have no body
    /// until the CSV is filled. Never NULL, so no reader branches on Option.
    pub body: String,
```

### 3.2 The six queries

All six are hand-written column lists — no `SELECT *` anywhere in this file
(verified). Each needs `body` in the list and `.get(2)`-style index shifts
fixed. The exact set, verified by grep:

| Function | Line | Query |
|---|---|---|
| `upsert_card` | 27 | INSERT + ON CONFLICT (both branches) |
| `list_cards` | 119 | 4 variants (sqlite/pg × stage-filtered/unfiltered) |
| `arena_candidates` | 154 | 2 variants |
| `record_move`/moves join | 378 | the changelog's card_title join |
| row→Card mappers | 503, 518 | `row.get::<String,_>(n)` |

Two mappers exist because there are two SQL dialects. Both need the shift.

**The index-shift trap.** Inserting `body` after `title` shifts every
positional `row.get` below it. `list_cards` currently reads
`id, title, category, stage, elo, matches, best, worst, created, updated` and
maps 0..9; adding `body` at position 2 makes it `id, title, body, category,
…` — and a mapper that was not updated still compiles, still passes its type
check, and silently serves one column off by one. `category` is the column
that eats the error, because both are `String`. Two `String` columns swapped
is invisible to the compiler and to a test that asserts a card's category.

Write the new column lists first, then the mappers, and read the diff for
every `.get(` line.

### 3.3 `find_card_by_id` **[NEW]**

No such function exists — verified: the module's public functions are
`upsert_card`, `find_card_by_title_normalized`, `list_cards`,
`arena_candidates`, `create_ballot`, `fetch_ballot`, `mark_voted`,
`apply_elo_and_counters`, `record_move`, `list_moves`, `insert_suggestion`,
`update_card_stage`. The detail route needs a by-id read, and it must not be
implemented as "list every card and filter in Rust" — that is
`find_card_by_title_normalized`'s mistake, which loads the whole table to
answer one question, and on a 667-card board with bodies is a ~600 KB read to
find one card.

```rust
/// Find a card by id. Returns None if no match.
///
/// [NEW] The detail route needs this, and the alternative — reusing
/// `find_card_by_title_normalized`'s load-everything approach — turns a
/// one-card read on a 667-card board into a 600 KB table scan.
pub async fn find_card_by_id(
    db: &Database,
    card_id: &str,
) -> Result<Option<Card>, sqlx::Error> {
    match db.backend() {
        Backend::Sqlite => {
            let row = sqlx::query(
                "SELECT id, title, body, category, stage, elo_rating, matches_played, times_best, times_worst, created_at, updated_at FROM roadmap_cards WHERE id = ?",
            )
            .bind(card_id)
            .fetch_optional(db.sqlite_pool().expect("sqlite"))
            .await?;
            Ok(row.map(map_card))
        }
        Backend::Postgres => {
            let row = sqlx::query(
                "SELECT id, title, body, category, stage, elo_rating, CAST(matches_played AS BIGINT), CAST(times_best AS BIGINT), CAST(times_worst AS BIGINT), created_at::text, updated_at::text FROM roadmap_cards WHERE id = $1",
            )
            .bind(card_id)
            .fetch_optional(db.postgres_pool().expect("postgres"))
            .await?;
            Ok(row.map(map_card))
        }
    }
}
```

This names `map_card` **[NEW]** — today the mapping is inline in `list_cards`
(two copies, at lines 503 and 518). Extract it once and have all callers use
it, so the index shift is fixed in one place instead of three. The extraction
is part of this step, not a cleanup: three copies of a positional mapper is
three places to get the same shift wrong.

### 3.4 `upsert_card` writes the body

Both branches. Note the existing `WHERE stage NOT IN ('shipped','rejected')`
guard: a `shipped` card's body still updates, because the guard is on `stage`
in the `DO UPDATE SET` clause and the SET list does not include `body` today.
After this change the SET list includes `body`, and the guard still only
protects the *stage* — which is §44.6's rule, exactly. A shipped card's body
should be correctable.

### 3.5 Verify

```bash
cargo test -p lorehaven-db --lib roadmap 2>&1 | tail -15
```

Expected: existing roadmap tests pass. Any failure naming a column is the
index shift; any failure naming `expected String, found` is a mapper.

```bash
cargo clippy --workspace --all-targets -- -D warnings 2>&1 | tail -15
```

Expected: no warnings.

## 4. Step 3 — routes

`crates/app/src/routes/roadmap.rs`.

### 4.1 The board includes the body

`group_by_stage` (line 50) gains one line in its `json!`:

```rust
                "body": card.body,
```

### 4.2 The detail route **[NEW]**

Add to `read_router` (public, no session — §44.5's board is public and this is
the same data about one card):

```rust
        .route("/roadmap/cards/:id", get(get_card))
```

```rust
/// `GET /api/v1/roadmap/cards/:id` — one card, full body. Public (§44.5).
pub async fn get_card(
    _maybe: MaybeSession,
    State(state): State<AppState>,
    Path(card_id): Path<String>,
) -> ApiResult<Json<Value>> {
    let card = roadmap::find_card_by_id(state.db(), &card_id)
        .await
        .map_err(|e| ApiError(AppError::Internal(e.into())))?
        .ok_or_else(|| ApiError(AppError::NotFound { resource: "card" }))?;
    Ok(Json(json!({ "card": {
        "id": card.id,
        "title": card.title,
        "body": card.body,
        "category": card.category,
        "stage": card.stage,
        "elo_rating": card.elo_rating,
        "matches_played": card.matches_played,
        "times_best": card.times_best,
        "times_worst": card.times_worst,
    }})))
}
```

`Path` must be added to the `axum` import at the top of the file — verified
absent.

`NotFound { resource: "card" }` — §2.2: a coarse noun, never an identifier
(README §2.2), and §3.3: 404 not 403 where saying "forbidden" would confirm
existence. The board is public so there is no forbidden case, but the
convention still holds.

### 4.3 The ballot carries the body

`get_arena` (line 109) gains `"body": c.body` in its `json!`.

### 4.4 Suggest accepts a body

`SuggestBody` (line 237) gains `pub body: Option<String>`, and
`post_suggest` passes it through. **It does not edit an existing card's body**
— §44.1: stage is the operator's. A suggestion that matches an existing card
attaches to it and its body is not written (that is `roadmap_suggestions`'s
job, and it stays a triage queue).

Add a length bound. There is no other bound on this route and the suggest
input is a single `<input maxlength="200">` for the title; a body is a page.
The page-long target is ~3,000 characters, so:

```rust
    if let Some(ref body) = body.body {
        if body.len() > 8_000 {
            return Err(ApiError(AppError::Validation {
                message: "card body must be 8000 characters or fewer".into(),
                field_errors: BTreeMap::new(),
            }));
        }
    }
```

### 4.5 Verify

```bash
cargo test -p lorehaven-app --test milestone_45_roadmap 2>&1 | tail -20
```

Expected: all 60+ existing roadmap tests pass. The board's added `body` field
is additive, so no existing assertion should break.

## 5. Step 4 — frontend

### 5.1 `frontend/src/lib/api.ts`

`RoadmapCard` (line 3696) gains `body: string;` after `title`. Add:

```ts
export interface RoadmapCardDetail {
  card: RoadmapCard;
}

export function fetchRoadmapCard(cardId: string, signal?: AbortSignal): Promise<RoadmapCardDetail> {
  return apiFetch<RoadmapCardDetail>(`/roadmap/cards/${encodeURIComponent(cardId)}`, { signal });
}
```

`encodeURIComponent` is not optional. Card ids are UUIDs today, but the
suggest endpoint generates them and nothing constrains a future id to that
shape; a raw interpolation is an injection surface the moment it stops being
a UUID.

### 5.2 `frontend/src/lib/router.ts`

Add `'/roadmap/:id'` with a param. Verified: `router.ts` line 112 maps
`'/roadmap': 'roadmap'` and line 51 lists `'roadmap'` in the path union. The
`:id` form is the pattern to copy — check how an existing parameterised route
is declared (e.g. the work detail route) and match it, because the resolved
type is what `handleLinkClick` and `RoadmapCard.svelte` both depend on.

### 5.3 `frontend/src/routes/RoadmapCard.svelte` **[NEW]**

Svelte 5 runes. `$state` for the card, `$effect` to load, `Skeleton` while
loading, `ErrorSummary` on failure, `handleLinkClick` on the back link.

```svelte
<script lang="ts">
  import { fetchRoadmapCard, type RoadmapCard } from '../lib/api';
  import { handleLinkClick } from '../lib/router.ts';
  import { page } from '../lib/router.ts';
  import ErrorSummary from '../lib/components/ErrorSummary.svelte';
  import Skeleton from '../lib/components/Skeleton.svelte';

  let card = $state<RoadmapCard | null>(null);
  let loading = $state(true);
  let error = $state<unknown>(null);

  $effect(() => {
    const id = page.params.id;
    if (!id) return;
    loading = true;
    error = null;
    fetchRoadmapCard(id)
      .then((r) => (card = r.card))
      .catch((e) => (error = e))
      .finally(() => (loading = false));
  });
</script>

<svelte:head><title>{card?.title ?? 'Card'} · Lorehaven</title></svelte:head>

<article class="card-page">
  <p><a href="/roadmap" onclick={(e) => handleLinkClick(e, '/roadmap')}>← Roadmap</a></p>
  {#if loading}
    <Skeleton lines={8} label="Loading card" />
  {:else if error}
    <ErrorSummary {error} />
  {:else if card}
    <h1>{card.title}</h1>
    <div class="card-meta">
      <span title="Elo rating">★ {Math.round(card.elo_rating)}</span>
      <span>{card.stage}</span>
      {#if card.category !== 'general'}<span>{card.category}</span>{/if}
    </div>
    {#if card.body.trim()}
      <div class="card-body">{card.body}</div>
    {:else}
      <p class="no-body">This card has no description yet.</p>
    {/if}
  {/if}
</article>
```

**Plain text, not `{@html}`.** The body is rendered as text, never as HTML
(README §2.5). Bodies come from the operator's CSV, not from members, so
there is no XSS surface today — and the next person to make bodies
member-authored inherits the sanitisation question, which the amendment
answers once.

### 5.4 The board card becomes a link

In `Roadmap.svelte`, `<article class="card">` (line 185) becomes a link to
`/roadmap/{card.id}`. Use `handleLinkClick` for SPA navigation like every
other link in this codebase. The `<h3>` title stays the visible text.

### 5.5 Verify

```bash
bash frontend/scripts/fe.sh check 2>&1 | tail -20
bash frontend/scripts/fe.sh test 2>&1 | tail -20
bash frontend/scripts/fe.sh build 2>&1 | tail -10
```

Expected: `svelte-check` reports 0 errors; the existing `Roadmap.test.ts`
(190 lines) passes; the build emits `frontend/dist`.

## 6. Step 5 — the journey, by hand

Automated tests are necessary, not sufficient (README §2.6). Drive it:

```bash
just reset-dev
just serve-dev
```

1. `http://localhost:8080/roadmap` — the board renders, every card shows
   title + Elo, and **no body text**. The list is titles only. This is the
   requirement from the amendment: "The board still lists title only."
2. Click a card → `/roadmap/<id>` shows the title and a full body.
3. Reload `/roadmap/<id>` directly — the route resolves on a cold load, not
   only after client-side navigation from the board.
4. `/roadmap/<id>` for a card whose body is empty shows the placeholder, not
   a blank area and not an error.
5. The Arena tab serves a ballot; each of the 4 cards shows its body.
6. Vote; the board's Elo updates.

Record which of these passed, in `docs/verification.md`, with the status
vocabulary from that file.

## 7. Step 6 — tests

### 7.1 Rust, in `crates/app/tests/milestone_45_roadmap.rs`

Append. The harness (`Harness::new`, `.card()`, `.titles()`) is at lines
58–170 — read it before writing, and add a `body` parameter or a `.with_body()`
builder to `.card()` so existing 60 tests do not change.

| Test | Asserts |
|---|---|
| `a_cards_body_survives_a_roundtrip` | insert with body, read back equal |
| `a_card_without_a_body_reads_as_empty` | `''`, not an error |
| `the_board_serves_a_body_without_a_session` | anonymous GET includes `body` |
| `the_detail_route_serves_the_whole_body` | GET `/roadmap/cards/:id` → body verbatim |
| `an_unknown_card_id_is_404` | named error, not 500 |
| `a_duplicate_vote_on_one_ballot_is_refused` | already exists — keep |
| `a_suggest_with_a_body_creates_a_card_carrying_it` | suggest path stores it |
| `a_suggest_matching_an_existing_card_does_not_overwrite_its_body` | §44.1 |
| `a_body_over_eight_thousand_characters_is_refused` | the bound |
| `the_seed_script_is_idempotent` | run twice, second run 0 inserts |

### 7.2 Frontend, in `frontend/src/routes/RoadmapCard.test.ts` **[NEW]**

- renders the title and the body from a mocked `fetchRoadmapCard`
- an empty body renders the placeholder, not a blank
- a rejected fetch renders `ErrorSummary`

### 7.3 The seeder self-test

`scripts/seed_roadmap.py` gains, in `self_test()`:

```python
    empty = [r["id"] for r in rows if not (r.get("body") or "").strip()]
    if empty:
        print(
            f"error: {len(empty)} rows have no body: {', '.join(empty[:10])}",
            file=sys.stderr,
        )
        print("the arena shows a card's body; a title-only row is a row nobody can judge", file=sys.stderr)
        return 1
```

Same shape as the status-vocabulary check already in that function, for the
same reason: a vocabulary that grew without the tool noticing is how a
tracker stops being trustworthy. This is the gate that makes step 8's 667
bodies a command rather than an intention.

## 8. Step 7 — the CSV body column

### 8.1 Add the column

```
id,area,requirement,body,milestone,status,evidence,notes
```

`body` after `requirement`. For existing rows, `body` is empty in the file
until filled — the migration's `DEFAULT ''` makes that legal, and the
seeder's self-test (§7.3) is what stops it from staying that way.

Use `python3` with `csv.DictReader`/`DictWriter`, never hand-editing a
307 KB CSV. Preserve the existing column order and do not re-quote fields
that do not need it; a whole-file rewrite that changes quoting on 667 rows
produces a 300 KB diff that hides the real change.

### 8.2 Fill 667 bodies

Each body answers, in this order: **what it is → why it exists → what it is
not.** Three to five sentences. Plain text, no Markdown (the detail view
renders plain text — §5.3).

Where the requirement text already states the "what", do not restate it —
open with the "why". A body that repeats its own title in the first sentence
wastes the reader's first line.

Derive from `docs/spec.md` where the feature is specified there, not from the
requirement text alone. `evidence` names the test that proves it;
`docs/spec.md` says what it is.

### 8.3 Verify

```bash
python3 scripts/seed_roadmap.py --self-test
```

Expected: `status vocabulary ok: …`, `evidence paths ok: …`, and
`bodies ok: all 667 rows carry a body`.

```bash
python3 scripts/seed_roadmap.py --dry-run --db-url sqlite:///data/lorehaven.sqlite
```

Expected: per-row `inserted`/`unchanged`/`protected` lines, and
`"inserted": 0` on a second consecutive run. **Zero inserted on the second
run is the idempotence gate** — if the second run inserts anything, the
dedup key is wrong and the whole feature is unshippable.

## 9. Step 8 — the brainstorm seed

### 9.1 The source file

Copy `/tmp/brainstorm.md` into the repo as
`docs/ideas/preservation-brainstorm.md`. It is the source, not the CSV — the
CSV is generated from it, never hand-edited to match.

### 9.2 Generate a CSV from it

**[NEW] `scripts/import_brainstorm.py`**

Parses the markdown: `## ` headings → `area`; `- ✓ **Title** — body` and
`- **Title** — body` → rows. 162 items, 41 checked, 13 sections (verified
2026-09-28).

Dedup: normalized title, matching `seed_roadmap.py`'s
`normalize_title` (lowercase, punctuation stripped, whitespace collapsed) —
**the same function, imported, not re-implemented.** A second implementation
is a second answer to "what is the same title", and ADR 0023 records that
this is the key the whole dedup rests on.

Writes `docs/ideas/preservation-brainstorm.csv` with
`id,area,requirement,body,milestone,status,evidence,notes`:
- `id` — `PB-001` … `PB-162`
- `area` — the section, slugified
- `requirement` — the bolded title
- `body` — the paragraph after the em-dash, plus the section heading as
  context
- `status` — `planned` for checked, `idea`-source otherwise

Then the rows are merged into `requirements.csv` by id, and the operator
adjusts `status` in `requirements.csv` by hand.

### 9.3 The stage question

**This needs an answer before step 8 can finish.**

`seed_roadmap.py` maps `planned` → `idea` stage (its `STATUS_TO_STAGE`). A
checkmarked brainstorm item is not the same as a `planned` requirements.csv
row: "planned" in the CSV means *the project intends to build this*, and that
is the operator's call, made in the project's own inventory.

The plan's recommendation, and the reason:

1. Seed all 162 as `idea` (arena-eligible).
2. Treat the 41 checkmarks as a **recommendation to promote to `up_next` or
   `medium_term`**, applied by the operator with
   `POST /api/v1/admin/roadmap/move` and a reason — which is the existing,
   audited, operator-only path.
3. The 121 unchecked stay `idea` and enter the arena, which is what the arena
   is for.

Seeding checkmarks directly as `up_next` would put 41 cards into a stage
that is *frozen* (§44.2) and invisible to the community's vote, on the
strength of a character in a text file. §44.1: "A card's stage is the
operator's decision."

### 9.4 Verify

```bash
python3 scripts/import_brainstorm.py --dry-run
```

Expected: `162 rows parsed (41 checked, 13 sections)`, then a per-row
`would insert` / `already present` list, and
`"would_insert": 162` against an empty board.

```bash
python3 scripts/import_brainstorm.py \
  --db-url postgres://lorehaven:***@127.0.0.1/lorehaven
```

Expected: `inserted: 162, updated: 0, unchanged: 0, protected: 0`.

**Run it a second time immediately.** Expected:
`inserted: 0, updated: 0, unchanged: 162, protected: 0`. A non-zero insert on
the second run means the dedup key disagrees with the board's, and the 162
ideas are now duplicated on the public board.

Run it twice — this repo's memory records four separate fixture-collision and
wrong-runner-output incidents, and this is the fifth candidate.

## 10. What this plan does not decide

- Whether 41 checkmarks become `up_next` or stay `idea` (§9.3). Operator's
  call.
- Whether a card's body is ever member-editable. The amendment says no; the
  next person to disagree should change the amendment, not the code.
- Whether `GET /api/v1/roadmap` should paginate. The amendment accepts a
  ~600 KB board payload; at 3,000 cards it stops being acceptable and
  pagination becomes the answer. Not this milestone.
- Migration numbers below 0090. Out of scope: 0084 is reserved (D5), 0086 is
  the in-flight M59 `body_audience` work, and 0087/0088 are claimed by the M59
  plan's later phases. Check for the next free number when you start.
