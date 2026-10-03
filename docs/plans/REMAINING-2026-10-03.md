# Lorehaven — remaining work, 2026-10-03 (updated)

Rewritten after M45-57 steps 7–8 and M45-22 steps 2–4 landed. Base for this
revision: `828244f` (tagged `v0.55-source-adapters`). Everything below is what is
*still* to do, in the order it will be done.

## Environment this session needs before anything runs

| Fact | Value |
|---|---|
| Toolchain | `export PATH="$HOME/.cargo/bin:$PATH"` **first**. System `rustc` is broken (partial Arch upgrade, libLLVM 22.1 gone). |
| Docker daemon | `sudo systemctl start docker` — needed before any `sudo docker`. |
| PostgreSQL | Container `lh-pg-test` on **127.0.0.1:55433**, user `lorehaven`, db `postgres`, password `lorehaven`. Start with `sudo docker start lh-pg-test`. |
| Test URL | `export LOREHAVEN_TEST_PG_URL='postgres://lorehaven:***@127.0.0.1:55433/postgres'` — **both engines means running the suite twice**, once with this set and once without. |
| SQLite | Default. `data/lorehaven.sqlite`. |
| Dev DB | Default is SQLite. To migrate PostgreSQL you must set `LOREHAVEN_DATABASE_URL`; `lorehaven migrate` with no env var silently touches only SQLite. |
| btrfs | Metadata DUP pool ~12.6/13.0 GiB. Builds are SLOW, not stalled. Before believing a hang: `ps -eo pid,stat,etime,comm \| awk '$2 ~ /^D/'`. |
| Mutation runs | One `cargo test` on this suite is ~40 s. Nine mutations do not fit a 300 s cell — run them backgrounded via a script (`/tmp/conc_mut.py` shape), not inline in `execute_code`. |

## Landed this session

| # | Work | Plan step | Commit | Evidence |
|---|---|---|---|---|
| 1 | M45-57 HTTP routes | 7 | `828244f` | `source_adapter_routes.rs` 9/9 on SQLite **and** PostgreSQL |
| 2 | M45-57 §55.5 automated check | 8 | `828244f` | `declarative_check.rs` 24/24; 10 mutations red |
| 3 | M45-22 migration 0114 | 2 | `828244f` | 114/114 apply on both engines; `\d concierge_sessions` shows uuid/TIMESTAMPTZ/DOUBLE PRECISION |
| 4 | M45-22 domain `concierge.rs` | 3 | `276f91c` | 16/16 tests; 12 mutations red |
| 5 | M45-22 store `concierge_store.rs` | 4 | *this commit* | 13/13 on both engines; 10 mutations proven (see below) |

Three defects found and fixed rather than shipped, each with its own proof:

1. **`extract_handlers` was an infinite loop** (`route_inventory.rs`). A
   non-identifier character left `i` untouched and `continue` re-read it forever,
   so the route-inventory test hung at 100 % CPU printing nothing — indistinguishable
   from a slow disk. Fixed with `i += 1`, plus a test that runs the extractor over
   every route line in the workspace, because the only way to prove a loop
   terminates is to run it.
2. **Routes registered at `/api/v1/api/v1/...`.** The module spelled out the full
   prefix inside a router already nested under `/api/v1`. Nothing failed to compile
   and the inventory test passed — table and module agreed on the same wrong
   string. The symptom was a bare 405 on every call.
3. **`record_session` recorded "no budget" as a budget of zero.** `i32::try_from(
   queue.session_budget_minutes().unwrap_or(0)).ok()` turned an absent selector
   into a stored `0`, which is a *different session* — one saying the reader asked
   for a zero-minute read. Now `and_then(try_from)`.

## The list — what is left

| # | Work | Plan step | Done when |
|---|---|---|---|
| 6 | M45-22 routes | 5 | `routes/concierge.rs` green on both engines, `ROUTE_TABLE` rows added to `route_inventory.rs` |
| 7 | M45-22 WIP notify | 6 | 6 named tests; the consume-the-watch test proven red |
| 8 | M45-22 frontend | 7 | `Concierge.svelte` + tests; **rebuild before test** — a stale `frontend/build` renders every route blank with HTTP 200 |
| 9 | M45-22 tracker + docs | 8 | tracker row updated with real evidence, not "done" |
| 10 | M45-57 tracker + docs | 10 | M45-57 row says "Path A shipped, Path B gated" |

### Step 6 detail — routes

Four endpoints behind `RequireSession`, all under `/api/v1/concierge`:

- `GET  /concierge/moods` — §54.2's list. The `SessionSelector::validate` error
  message already names the available moods, so this route is thin.
- `POST /concierge/sessions` — render + record. Mood must be validated against
  `moods_in_use` first; an unknown mood is 422 with the list, **not** a fallback to
  the unfiltered queue (§54.6's "matches nothing is not a fallback").
- `GET  /concierge/sessions` — the reader's own, via `sessions_for`.
- `POST /concierge/watches` / `DELETE /concierge/watches/{work_id}` — §54.5.

Wiring already exists: `moods_in_use`, `record_session`, `sessions_for`,
`add_watch`, `remove_watch`, `is_complete` are all in the store.

### Step 7 detail — WIP notify

The completion path already exists somewhere in the work-completion code. The new
part is: `pending_watches_for_work` → `notifications::notify` → `mark_watched`.
The load-bearing test is that a second pass over the same work notifies nobody,
which requires asserting on `mark_watched`'s boolean rather than on the absence of
a duplicate row (the UNIQUE constraint makes duplicates impossible anyway).

### Out of scope here, on purpose

- **Path B (WASM), §55.4.** §55.6 gates it. `scripts/check-wasm-gate.py` fails the
  build if a WASM runtime is adopted; `wasmi 0.4` does not compile on this toolchain.
- **§36.11 mood journal.** §54 reuses §15.8's mood *vocabulary*; the journal is
  separate work and the plan says so.
- **The pre-existing CI reds.** `scripts/check-sqlite-migration-syntax.py --self-test`
  (5 cases) and `scripts/check-pg-uuid-casts.py` both reproduce on a clean stash.
  Named in the final report rather than quietly fixed or ignored.
- **The 27 `planned` + 6 `specified` tracker rows** (M45-23 north-star, M45-25 …
  M45-55, M46-05 search). Each is a multi-week spec of its own. Listed so the
  number is honest rather than implied.

## Trap log for whoever continues

- sqlx does **not** translate `?1` → `$1` for PostgreSQL. A test fixture that
  forgets the rewrite reaches the server as a literal `?` and fails with
  `operator does not exist: ? integer`. Reuse `preread_store.rs`'s `exec` helper.
- `?1#u` markers: the fold must try `?N#u` **before** `?N`, or the `#u` survives
  into the SQL as a column named `u`. A doubled `?1#u#u` is the same failure.
- `work_tags.work_id` has a real FK on PostgreSQL and **none** on SQLite. Tagging
  a work before inserting it passes on SQLite and fails on PostgreSQL.
- `works.owner_pseud_id` is NOT NULL and FK-constrained: every work fixture needs
  its own pseud and account.
- A per-file SQL helper is a liability. `moods_in_use`'s first version cast
  `works.id::text = wt.work_id`, which PostgreSQL accepts and returns nothing for —
  a silently empty list, not an error.