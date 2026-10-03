# M45-22 — Personal concierge queue

Spec: `docs/spec.md` §54. Implementation plan. Tracker row: `docs/requirements.csv`
`M45-22`.

Every step below is copy-pasteable and carries its own verification command and
the exact output that means it passed. Do not start a step before the previous
step's verification prints what is quoted.

## What already exists, so this does not rebuild it

| Thing | Where | Note |
|---|---|---|
| Strategy registry + RRF blend | `crates/db/src/rec_strategy.rs`, `crates/app/src/rec_engine.rs` | §16.1a. `RecContext { account_id, seen, cap }` |
| Mood taxonomy | `crates/domain/src/taxonomy.rs` (`NodeKind::Mood`), `crates/domain/src/query.rs` (`QueryField::Mood`) | §15.8 / M10-06, `implemented-locally-tested` |
| Reading progress | `crates/db/src/reading.rs` | `save_progress`, `progress_for` |
| Notifications | `crates/db/src/notifications.rs` | `notify(db, account_id, kind, title, body, work_id)` |
| `Candidate` | `crates/domain/src/discovery.rs` | `work_id, score, reason, taste_signal, diversity_class` |
| §36.11 mood journal | — | **specced, NOT implemented.** No mood code exists. |

**§36.11 is not implemented and this plan does not implement it.** §54 reuses the
mood *vocabulary* (§15.8) and the idea of a private per-reader record, but a
concierge session is its own table. Implementing the journal is separate work;
where §54 needs reader-side mood data it uses §15.8's author-assigned moods.

## Two defects found while scoping, fixed in step 1

Both are in the path §54 depends on, and both are dead code today:

1. **`RecContext.seen` is never populated.** `rec_engine::generate_with_registry`
   sets `seen: vec![]`, so nothing excludes works the reader has already read.
2. **`generate_traced` computes `_seen_set` and never reads it.** The exclusion
   filter exists as a `HashSet` that is inserted into and dropped.

Without step 1 a concierge queue would offer a reader a work they finished last
week, and the queue's central promise ("works you have not read") would be false.

---

## Step 1 — Make `seen` real

**File:** `crates/app/src/rec_engine.rs`, `crates/db/src/rec_strategy.rs`

Add to `crates/db/src/reading.rs`:

```rust
/// Every work this account has any reading signal for.
///
/// Deliberately broad: a read, a started read and a rating all mean "this
/// reader has already been offered this and did not need it again". A narrow
/// definition would let the queue re-serve a work the reader DNF'd three weeks
/// ago.
///
/// **The shape is not three `work_id` columns.** `reading_history_entry` and
/// `reading_progress` are polymorphic — `subject_type` + `subject_id` — so a
/// work is `subject_type = 'work'`. Only `rating` has a real `work_id`. Querying
/// them as if they were all the same shape returns nothing for the two tables
/// that hold most of a reader's history.
pub async fn seen_work_ids(db: &Database, account_id: &str) -> Result<Vec<String>, sqlx::Error> {
    let sql = db.sql(
        "SELECT subject_id FROM reading_history_entry
          WHERE account_id = ? AND subject_type = 'work'
         UNION
         SELECT subject_id FROM reading_progress
          WHERE account_id = ? AND subject_type = 'work'
         UNION
         SELECT work_id FROM rating
          WHERE account_id = ? AND deleted_at IS NULL",
        "SELECT subject_id::text FROM reading_history_entry
          WHERE account_id = $1::uuid AND subject_type = 'work'
         UNION
         SELECT subject_id::text FROM reading_progress
          WHERE account_id = $1::uuid AND subject_type = 'work'
         UNION
         SELECT work_id::text FROM rating
          WHERE account_id = $1::uuid AND deleted_at IS NULL",
    );
    match db.backend() {
        Backend::Sqlite => Ok(sqlx::query_scalar(&sql)
            .bind(account_id).fetch_all(db.sqlite_pool().ok_or(pool_err())?).await?),
        Backend::Postgres => Ok(sqlx::query_scalar(&sql)
            .bind(account_id).fetch_all(db.postgres_pool().ok_or(pool_err())?).await?),
    }
}
```

> **Verified against the schema, not assumed.** `reading_history_entry` and
> `reading_progress` both carry `subject_type`/`subject_id`, not `work_id`; the
> only `work_id` is on `rating`. Both tables and all three columns come from
> `migrations/sqlite/0004_reading.sql`. `subject_id` is TEXT on SQLite and UUID on
> PostgreSQL, hence the `::text` in the PG branch — without it the driver cannot
> decode into `String`. `rating` carries `deleted_at`, so a withdrawn rating must
> not count as seen.

Then in `rec_engine.rs`, populate it:

```rust
pub async fn generate_with_registry(
    db: &Database,
    registry: &RecRegistry,
    account_id: &str,
    limit: usize,
) -> Result<Vec<String>> {
    let seen = lorehaven_db::reading::seen_work_ids(db, account_id).await.unwrap_or_default();
    let ctx = RecContext { account_id: account_id.to_string(), seen, cap: limit };
    registry.generate(db, ctx).await
}
```

`unwrap_or_default()` is correct here: a reader with no history has seen nothing,
and an unavailable query must not fail the feed.

In `rec_strategy.rs::generate_traced`, make the filter real:

```rust
let mut seen_set: HashSet<String> = ctx.seen.iter().cloned().collect();
// ...
for (rank, work_id) in ranked.iter().enumerate() {
    if seen_set.contains(work_id) {
        continue;   // already offered; §36.11's exclusion
    }
    let entry = scores.entry(work_id.clone()).or_insert(0.0);
    *entry += 1.0 / (self.k + (rank + 1) as f64);
}
```

Note the `continue` keeps the rank in the RRF denominator — a strategy's own
ranking is not renumbered because we skipped a work, so one strategy's
contribution stays comparable to another's.

**Verify**

```sh
cargo test -p lorehaven-app --lib rec_engine 2>&1 | tail -5
cargo test -p lorehaven-db --lib rec_strategy 2>&1 | grep 'test result'
```

Then prove the filter bites — add this test to `crates/db/src/rec_strategy.rs`:

```rust
#[tokio::test]
async fn a_work_in_the_seen_set_is_excluded_from_the_blend() {
    let db = test_support::TestDb::connect("rec-seen").await.db().clone();
    let work = seed_published_work(&db, "seen-work").await;
    let reg = RecRegistry::new(60.0);
    reg.register("always", Arc::new(|_db, _ctx| Box::pin(async { Ok(vec![work.clone()]) })));
    let ids = reg.generate(&db, RecContext {
        account_id: id("acc").to_string(),
        seen: vec![work],
        cap: 10,
    }).await.unwrap();
    assert!(ids.is_empty(), "a seen work must not be blended: {ids:?}");
}
```

Run it, then **break it**: delete the `if seen_set.contains(work_id) { continue; }`
lines and confirm it goes red. Restore, confirm green. A test that has not been
seen red proves nothing.

**Also run the existing suites that depend on this** — `seen` being populated for
the first time can change what `/discovery` returns:

```sh
cargo test -p lorehaven-app --test route_inventory 2>&1 | tail -3
cargo test -p lorehaven-app --lib discovery 2>&1 | tail -3
```

---

## Step 2 — Migration 0113: sessions and watches

**Files:** `migrations/sqlite/0113_concierge.sql`, `migrations/postgres/0113_concierge.sql`

SQLite:

```sql
-- M45-22: the personal concierge (§54).
--
-- One row per rendered queue. It records the reader's *stated intent* and what the
-- ranker returned. It is private to the reader (§54.3) and is never a ranking input
-- (§0.3): a session must not make one work outrank another in §16's blend.
--
-- `mood` is a taxonomy node key (§15.8), stored as the reader wrote it rather than as
-- an id, because the taxonomy is community-extensible and a canonical key survives the
-- id churn an extension causes. NULL means "no mood selector", which is the default
-- queue and not an error (§54.6).
--
-- `budget_minutes` is NULL when the reader stated no budget, and `truncated_at` is NULL
-- when the whole blended queue fit. A NULL truncated_at with a non-NULL budget means
-- "nothing was cut", which is a different fact from "cut at index 0" and has to be
-- representable.
CREATE TABLE IF NOT EXISTS concierge_sessions (
    id                TEXT PRIMARY KEY,
    account_id        TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    mood              TEXT,
    budget_minutes    INTEGER,
    -- What the queue returned, in order, as JSON. Work ids only - the queue is a
    -- rendering, and re-ranking it later would make it a feed, which §54.1 refuses.
    work_ids          TEXT NOT NULL DEFAULT '[]',
    estimated_minutes REAL,
    truncated_at      INTEGER,
    created_at        TEXT NOT NULL
);

-- §11.15's `cache | aggregate` split makes this `cache`: it is a record of intention,
-- dropped on the instance's own schedule. Nothing derives from it after the window.
CREATE INDEX IF NOT EXISTS concierge_sessions_account
    ON concierge_sessions (account_id, created_at DESC);

-- A WIP watch (§54.5). The UNIQUE is the guarantee that a reader cannot stack two
-- watches on one work and be notified twice; it is not a convenience constraint.
CREATE TABLE IF NOT EXISTS wip_watches (
    id                TEXT PRIMARY KEY,
    account_id        TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    work_id           TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    notified_at       TEXT,
    created_at        TEXT NOT NULL,
    UNIQUE (account_id, work_id)
);
CREATE INDEX IF NOT EXISTS wip_watches_pending ON wip_watches (work_id) WHERE notified_at IS NULL;
```

PostgreSQL twin: `id UUID PRIMARY KEY`, `account_id UUID NOT NULL`,
`work_id UUID NOT NULL`, `work_ids JSONB NOT NULL DEFAULT '[]'`, `created_at
TIMESTAMPTZ NOT NULL`, `estimated_minutes DOUBLE PRECISION`, and no `::timestamptz`
in the default (there are no seed rows here). Follow `0112`'s comments for what
each convention is for.

**Verify**

```sh
cargo test -p lorehaven-db --lib migration 2>&1 | grep -E '0113|test result'
cargo run --bin lorehaven -- migrate 2>&1 | tail -5
```

Expect **113/113** migrations applied, on SQLite and again on PostgreSQL. Then:

```sh
sqlite3 data/lorehaven.sqlite "select count(*) from concierge_sessions;"   # 0
PGPASSWORD=smoke_pw psql -h 127.0.0.1 -U postgres -d <db> \
  -c "select count(*) from concierge_sessions;"                            # 0
```

Both must answer, and the second only works if `created_at`'s type matches.

---

## Step 3 — Domain: the session selector and its vocabulary

**File:** `crates/domain/src/concierge.rs` (new), registered in `crates/domain/src/lib.rs`

```rust
//! §54 session selectors: what the reader stated, and what it is allowed to
//! constrain.

/// What the reader asked for. Both fields are optional and neither is required:
/// a request with neither is the plain §16 blend (§54.6).
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SessionSelector {
    /// A §15.8 mood key, or a §36.11 custom label. `None` means no mood filter.
    pub mood: Option<String>,
    /// Minutes the reader has. `None` means no budget, and the queue is not cut.
    pub budget_minutes: Option<u32>,
}

/// Why a queue item is in it.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum QueueReason {
    /// Selected by the mood the reader named.
    Mood { mood: String },
    /// Came back from the §16 blend with no selector matching.
    Blend,
    /// Included despite having no duration estimate (§54.4).
    DurationUnknown,
}

/// One entry in the queue.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct QueueItem {
    pub work_id: String,
    pub reason: QueueReason,
    /// Minutes estimated for this work. `None` when unknown, which §54.4 says is
    /// rendered rather than dropped.
    pub estimated_minutes: Option<f64>,
}

/// A rendered queue, and the facts a reader is entitled to about how it was made.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct ConciergeQueue {
    pub session_id: String,
    pub items: Vec<QueueItem>,
    /// Total estimated minutes of the returned items.
    pub estimated_minutes: f64,
    /// Index the budget bound at. `None` when nothing was cut.
    pub truncated_at: Option<usize>,
    /// Whether the estimate used the reader's observed reading speed or the
    /// §36.11 default rate. §54.4 requires this be stated, not assumed.
    pub rate_source: RateSource,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum RateSource {
    /// From §36.11 progress sync.
    Observed,
    /// The instance default, because the reader has no observation yet.
    Default,
}
```

Plus, in the same file, the cut:

```rust
/// Cut `ranked` to `budget_minutes`, in order (§54.4).
///
/// The cut is a prefix so two renders of one session agree. A work with no
/// estimate is *not* dropped: it is placed at the midpoint of the range and
/// marked, because dropping it would make the queue length depend on how
/// complete the archive's metadata is.
#[must_use]
pub fn apply_budget(
    ranked: &[(String, Option<f64>)],
    budget_minutes: Option<f64>,
) -> (Vec<QueueItem>, Option<usize>, f64) { /* ... */ }
```

**Verify**

```sh
cargo test -p lorehaven-domain --lib concierge 2>&1 | grep 'test result'
```

Required tests, each of which must be proven red by breaking it:

| Test | Asserts |
|---|---|
| `an_unknown_mood_is_refused_by_name` | `SessionSelector::validate` errors and names the moods that exist |
| `no_budget_cuts_nothing` | `truncated_at` is `None` |
| `the_cut_is_a_prefix` | returned ids are the first *n* of `ranked` |
| `two_renders_agree` | same input twice → identical output |
| `a_work_with_no_estimate_is_kept_and_marked` | present, `estimated_minutes: None` |
| `a_zero_budget_returns_an_empty_queue_with_a_reason` | empty, `truncated_at: Some(0)` — not a fallback |
| `the_boundary_work_is_included_when_it_fits_exactly` | `<=` not `<` at the boundary |

That last one is the off-by-one that decides whether "I have 20 minutes" returns
the work that ends at minute 20.

---

## Step 4 — Store

**File:** `crates/db/src/concierge_store.rs` (new), re-exported from `crates/db/src/lib.rs`

- `record_session(db, account_id, &ConciergeQueue) -> Result<String>`
- `sessions_for(db, account_id, limit) -> Result<Vec<ConciergeQueue>>` — the
  reader's own history, newest first, and **scoped in SQL by `account_id`**, not
  filtered in Rust. A filter after the fetch is a filter someone will forget.
- `moods_in_use(db) -> Result<Vec<String>>` — the §15.8 keys actually carried by
  at least one published work, for the "here are the moods we have" error.
- `add_watch(db, account_id, work_id) -> Result<String>` — `ON CONFLICT
  (account_id, work_id) DO NOTHING`, so a second watch returns the first's id
  rather than erroring.
- `pending_watches_for_work(db, work_id) -> Result<Vec<(String, String)>>` —
  `(account_id, watch_id)` where `notified_at IS NULL`.
- `mark_watched(db, watch_id)` — sets `notified_at`.
- `remove_watch(db, account_id, work_id) -> Result<bool>`.

Every function takes `account_id` and puts it in the `WHERE`. A function that
takes a work id and returns rows without one has no business existing (§54.6).

**Verify**

```sh
cargo test -p lorehaven-db --lib concierge_store 2>&1 | grep 'test result'
LOREHAVEN_TEST_PG_URL='postgres://postgres:smoke_pw@127.0.0.1:5432/postgres' \
  cargo test -p lorehaven-db --lib concierge_store 2>&1 | grep 'test result'
```

**Both must print the same count.** Then the two tests that matter for privacy:

- `another_readers_sessions_are_not_returned` — two accounts, each sees only its
  own. Break it by deleting `AND account_id = ?` and confirm red.
- `a_second_watch_on_the_same_work_returns_the_first` — `add_watch` twice, one
  row.

---

## Step 5 — Routes

**File:** `crates/app/src/routes/concierge.rs` (new), merged in
`crates/app/src/server.rs` under `classified(..., RouteClass::Default, ...)`

| Route | Class | Auth |
|---|---|---|
| `GET /api/v1/me/concierge` | Default | session |
| `GET /api/v1/me/concierge/sessions` | Default | session |
| `GET /api/v1/me/watches` | Default | session |
| `PUT /api/v1/me/watches/{work_id}` | Write | session |
| `DELETE /api/v1/me/watches/{work_id}` | Write | session |

`GET /api/v1/me/concierge` takes `?mood=` and `?minutes=`.

**Every new route must be added to `ROUTE_TABLE`** or
`registered_routes_are_tabled` fails — and note it stops at the *first*
unregistered route, so one omission hides the rest. Search the table for
`concierge.rs` directly rather than trusting the error to enumerate.

Order of work inside the handler, because §54.4 and §54.6 both constrain it:

1. Validate the selector. An unknown mood is `400` naming the moods that exist.
2. Build `RecContext` with `seen` populated (step 1) and a **cap well above the
   budget** — the cut is a tail operation, so the blend must be allowed to
   overrun and then be cut, not asked for exactly N.
3. Map ids → `Candidate` with `reason` from `QueueReason`, per §50.
4. `apply_budget`.
5. `record_session`.

Step 2's cap is the trap: asking the blend for `budget_minutes` worth of works
makes the budget decide *eligibility*, which §54.2 forbids.

**Verify**

```sh
cargo test -p lorehaven-app --test concierge_routes 2>&1 | grep 'test result'
LOREHAVEN_TEST_PG_URL='postgres://postgres:smoke_pw@127.0.0.1:5432/postgres' \
  cargo test -p lorehaven-app --test concierge_routes 2>&1 | grep 'test result'
cargo test -p lorehaven-app --test route_inventory 2>&1 | tail -3
```

Required, from §54.7:

- `the_queue_needs_a_session` — 401 unauthenticated, on **every** route.
- `no_selector_returns_the_same_works_as_discovery` — same ids, same order. This
  is the §54.6 invariant that the session layer made the default path worse.
- `an_unknown_mood_is_refused_naming_the_moods_we_have`.
- `twenty_minutes_returns_a_prefix_and_names_the_cut` — asserts
  `truncated_at` *and* `estimated_minutes`, not just the length.
- `two_renders_of_one_session_are_identical`.

---

## Step 6 — WIP completion notifications

**File:** `crates/app/src/wip_watch.rs` (new), called from the completion
transition path in `crates/app/src/routes/works.rs`

The column to watch is **`works.completion`, not `works.lifecycle`**, and they are
orthogonal. `crates/domain/src/policy.rs` defines
`Completion = InProgress | Complete | Hiatus | Abandoned`, serialised
`in_progress` / `complete` / `hiatus` / `abandoned`. Trigger on the transition
`in_progress -> complete`. A work can be `lifecycle = published` and still
incomplete, and that is exactly the WIP case this feature exists for.

Duration estimates use `word_count`, which lives on **`chapter_revisions`**, not
on `works` — the same aggregate the arena pool query already computes:
`SELECT c.work_id, SUM(cr.word_count) FROM chapters c JOIN chapter_revisions cr
ON cr.id = c.current_revision_id GROUP BY c.work_id`.

```rust
/// Notify every reader watching `work_id` that it has completed, once ever.
///
/// The `notified_at IS NULL` filter and the `mark_watched` that follows are one
/// operation's worth of intent: §54.5 says once per watch, ever, and the second
/// half of that is the mark. Cancelling rather than leaving it to re-fire is what
/// stops an unrecognised completion from re-notifying on the next unrelated edit.
pub async fn notify_completion(db: &Database, work_id: &str) -> Result<u64, sqlx::Error>
```

Called when a work's `completion` becomes `complete`. It must be **idempotent
under retry**: marking before notifying loses a notification, notifying before
marking can double-notify. Mark in the same transaction as the notification
insert, or accept at-least-once and say so in the doc comment — do not pretend it
is exactly-once without it.

`notifications::notify` **silently drops when the event is disabled for that
account** (it returns an id with no row written). So "marked as notified but no
notification exists" is reachable when a reader has the event off. Decide which
is correct: if the watch should still be consumed, mark regardless and accept
that a disabled reader sees nothing; if not, only mark when the insert happened.
Either is defensible — pick one, write it in the doc comment, and test it.

**Verify**

```sh
cargo test -p lorehaven-app --test wip_watch 2>&1 | grep 'test result'
```

Required:

| Test | Asserts |
|---|---|
| `a_watcher_is_notified_once_when_the_wip_completes` | one row; a second completion event adds none |
| `watching_an_already_complete_work_notifies_immediately` | notified at watch time |
| `a_withdrawn_watch_never_notifies` | no row after delete |
| `two_watches_on_one_work_notify_once` | `ON CONFLICT` holds |
| `completing_a_work_nobody_watches_is_not_an_error` | 0 notified |
| `the_watch_is_consumed_by_its_own_notification` | `notified_at` set, so no re-fire |

The second half of the last one is the "prove guards by breaking them" rule: set
`notified_at` unconditionally and confirm `a_watcher_is_notified_once…` goes red.

---

## Step 7 — Frontend

**Files:** `frontend/src/routes/Concierge.svelte`,
`frontend/src/routes/Concierge.test.ts`, `fetchConcierge` / `addWatch` /
`removeWatch` in `frontend/src/lib/api.ts`

The selector is three controls, not a form: mood chips from the vocabulary the
API returns, a minutes input, and the queue beneath. Each item shows its
`QueueReason` — a reader told "comfort" and shown a work with
`reason: Blend` can see the mood did not do what they asked.

**Verify**

```sh
cd frontend && node node_modules/vite/bin/vite.js build && \
  node node_modules/vitest/vitest.mjs run src/routes/Concierge.test.ts 2>&1 | tail -12
```

Build before the test: a stale `frontend/build` renders every route blank with
HTTP 200 while the test suite stays green.

---

## Step 8 — Tracker, spec status, definition of done

```sh
# requirements.csv: M45-22 planned -> implemented-fully-tested, with evidence.
# That file is CRLF. Do NOT rewrite it with Python's csv module — that converts
# every line ending and turns a 1-line edit into 698. Split on "\r\n", edit the
# one line, rejoin with "\r\n".

git add docs/spec.md docs/plans/m45-22-concierge.md docs/requirements.csv
git commit -m "M45-22: the personal concierge queue"
```

Update the `docs/plans/WHAT-IS-LEFT.md` "In flight" and "Also outstanding"
sections in the same commit.

**Definition of done**

- [ ] All §54.7 acceptance criteria have a named test, green on **both** engines.
- [ ] `cargo clippy --workspace --all-targets` reports 0 warnings.
- [ ] `cargo test --workspace --doc` is clean (a separate target; §9d0e0cd).
- [ ] Every new test has been seen red.
- [ ] 113/113 migrations on both dialects.
- [ ] No route missing from `ROUTE_TABLE`.
- [ ] Tracker row updated with evidence, not just a status.

## Traps

- **`seen` becoming real changes `/discovery` output.** Step 1 is not a concierge
  change; it is a feed change. Run the discovery suites.
- **`cargo test --workspace` may stall on this host.** Check
  `btrfs fi df /` and `ps -eo pid,stat,comm | awk '$2 ~ /^D/'` before believing a
  hang. See the `btrfs-metadata-exhaustion-stalls-io` skill.
- **Test binaries using relative paths must run from their own crate
  directory**, e.g. `crates/app`, not the repo root.
- **`git diff` is intercepted in this repo** and can report "No syntactic
  changes" for a file that genuinely differs. Use `diff <(git show HEAD:f) f`.
- **Never `git add -A` a directory while a test binary is running.**
