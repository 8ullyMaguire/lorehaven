# Build plan — the rest of Lorehaven

This directory is the working plan for the milestones that are **not yet
built**. `docs/spec.md` says what the platform must do; these files say what to
type, in what order, and how to check it. They are written for someone who
knows Rust and Svelte but has never seen this repository.

> **Status note (2026-09-24):** the live forward plan is now
> `docs/plans/remaining-work.md` (ADR 0024 — the gap to the consolidated
> from-scratch spec: M52 rec registry, M53 adapter porting, M54 bot port,
> M55 OpenAPI, M56 M45/M47 residuals). The junior implementation plan below
> is the historical map of M0–M47 and its §0.1/§0.2 tables are stale —
> `docs/requirements.csv` is the row-by-row authority, and
> `docs/handoff.md` describes where the build stands today.

> **Status note (2026-09-14):** this README's milestone list predates the
> junior implementation plan and does not match its M-numbering — treat
> `junior-implementation-plan.md` (§0.2 for the numbering map, §15 for the
> current milestone) as the live plan, and `docs/sessions/2026-09-14.md` as
> the hand-off describing exactly where the build stands today.

Milestones 0–5 are done and Milestone 6 is **partly built** — its machinery is
finished and its pages are not, so the work below is what remains of it.
`docs/verification.md` records the evidence for each claim, and
`docs/requirements.csv` records every requirement with a status. Both are updated
**as part of** finishing a milestone, never afterwards.
`docs/plans/milestone-06-imports.md` records what M6 actually became, where that
differs from the plan below, and what is left before it can be tagged.

---

## 1. The order of work, and why

```text
M4  Reader, ratings, history, work pages      ← done, tag v0.05-reader
M5  Jobs, storage, cache, secrets             ← done, tag v0.06-jobs
M6  Imports, source credentials, batches
M7  Exports, device delivery, offline
M8  Library, saved views, bookmarks
M9  Taxonomy, body search, query language
M10 Discovery, private taste influence
M11 Comments, forums, groups, messaging
M12 Collections, challenges, requests, events
M13 Trust, reports, quorum, appeals
M14 Credits, fair queues, billing
M15 Marketplace, extension isolation
M16 Public API, feeds, push, federation, AI
M17 Administration, statistics, abuse, privacy, ops
M18 Hardening and release
```

The order is a dependency order, not a preference. Three examples, because a
junior developer will otherwise be tempted to reorder:

* **M5 before M6.** Importing is a background job: nothing in M6 can be
  written honestly before there is a job model with leases and retries.
  Writing an import "for now, inline in the request" is the specific mistake
  this order prevents — a request that fetches a 300-chapter work times out,
  and the retry does it all again from scratch.
* **M5 before M7.** Every export is a job that writes a file to storage.
* **M9 before M10.** Two of the recommendation engines read an inverted index
  and a tag graph; neither exists until M9 has built them.

Within a milestone the order is always the same, and it is the order the
vertical-slice rule in `docs/spec.md` §1.3 demands:

```text
migration (both dialects)
→ domain types and policy functions
→ repository functions
→ routes
→ pages
→ the journey, driven by hand in a browser
→ tests that pin what the journey proved
→ requirements.csv + verification.md
```

Do **not** write all the migrations for a milestone, then all the routes, then
all the pages. Build one journey end to end, check it by hand, then broaden.

---

## 2. House rules a newcomer must follow

These are not stylistic. Each one exists because its absence already caused a
defect somewhere in this repository's history, and the commit messages say so.

### 2.1 The database layer

* Every statement is written **twice**: once for SQLite, once for PostgreSQL.
  `db.sql("... ?", "... ?::uuid")` picks one and rewrites `?` to `$1…$n` for
  PostgreSQL. See `crates/db/src/lib.rs` (`Database::sql`) and any function in
  `crates/db/src/content.rs` for the shape to copy.
* When a statement is assembled from shared column lists, use `sql_owned`
  (`crates/db/src/lib.rs`) — `db.sql` borrows, and a temporary `format!` result
  does not live long enough.
* Bind only `String` and `i64` (and `Option<…>` of those). PostgreSQL `uuid`
  columns are written as `?::uuid` and read with `id::text AS id`. This is what
  lets one row type decode on both engines. See ADR 0004.
* Never put a literal `?` inside SQL text: it is always a placeholder.
* A function that needs several statements in one transaction writes the SQLite
  branch and the PostgreSQL branch out separately. They are different
  transaction types; do not try to abstract over them. A `macro_rules!` that
  expands the shared body is acceptable and is used in
  `crates/db/src/content.rs::append_revision`.
* Every migration exists **twice**, with identical ids:
  `migrations/sqlite/000N_name.sql` and `migrations/postgres/000N_name.sql`.
  A test (`the_two_dialects_define_the_same_migration_ids`) fails if they drift.
  The next free number is **0004**.
* Every migration states, in a comment, its deletion and retention behaviour
  (spec §4.1). Not a summary — the actual rule, including what cascades and
  what deliberately does not.

### 2.2 Errors and policies

* Failures are `AppError` (`crates/domain/src/error.rs`). A new failure mode
  gets a variant there with a stable code, an HTTP status and a public message.
  Never return a raw `anyhow::Error` to a client: `AppError::Internal` masks it
  and logs the chain (the logging uses `?error`, not `%error`, precisely so the
  chain is visible).
* Authorization lives in **pure functions** in `crates/domain/src/policy.rs`
  and `crates/domain/src/content.rs`: `fn can_do_thing(actor, facts) ->
  Decision`. No database, no clock, no I/O. The handler loads the facts and
  asks. A `if account_id == ...` in a handler is the thing this rule forbids.
* **Reading content goes through `can_access_content`** and nothing else. If
  you find yourself writing a second check — "this one is for the reader, that
  one is for the download" — you are about to leak restricted content. Add the
  fact the check needs to `ContentFacts` instead.
* A resource the caller may not reach is `404`, not `403`, whenever saying
  "forbidden" would confirm that it exists (spec §3.3). `AppError::NotFound`
  takes a coarse noun ("work"), never an identifier.

### 2.3 Optimistic concurrency

Every editable row has `version`. Every mutating statement carries
`... AND version = ?` in its `WHERE` clause. Zero rows affected means the caller
lost a race, and the handler re-reads the row and returns
`AppError::RevisionConflict { expected, actual }` so the client can say what it
was racing. **Never** `SELECT` then `UPDATE` without the version predicate.

### 2.4 The HTTP layer

* Routes are declared in `crates/app/src/routes/<area>.rs` and registered in
  `crates/app/src/server.rs::build_router`. A route that is not registered is
  not reachable — there is no discovery.
* Every route tree is wrapped in `classified(...)`, which declares its
  rate-limit class (`Auth`, `Write`, `Search`, `Default`) and installs the
  limiter. The limiter **fails closed** when a request carries no class, so a
  route tree merged without `classified` returns 500 for every request. This
  has already happened once.
* The class marker must be layered *outside* the limiter; `classified` does
  this for you, which is why you use it rather than writing the layers yourself.
* Cookie-authenticated state changes need the CSRF layer, which is applied to
  the `account_routes` subtree in `build_router`. `Write`-class routes belong
  under it.
* Signing-in-required handlers take `RequireSession` (or `RequirePseud`);
  handlers a visitor may reach take `MaybeSession`. Extracting a `RequireSession`
  *is* the authentication check.
* Collections answer with `{ "items": [], "next_cursor": null }` (spec §3.3).
  Existing endpoints that return a bare array are a known inconsistency; new
  cursor-paginated endpoints must use the envelope.

### 2.5 The frontend

* Svelte 5 runes (`$state`, `$derived`, `$effect`, `$props`). No stores, no
  `export let`.
* Field components take a **`$bindable` value**. `bind:value` without
  `$bindable()` on the child compiles and silently does nothing; this shipped
  once and every form submitted empty. `FieldBinding.test.ts` now pins it.
* The API client (`frontend/src/lib/api.ts`) mirrors the routes exactly and
  never navigates on a failure. Types come from the server's response shapes;
  where the server omits a field, the type has no field.
* Router paths live in `frontend/src/lib/router.ts`. A linked-but-unbuilt
  destination resolves to `Planned` and says which milestone will fill it —
  never to mock data.
* HTML from the server is rendered with `{@html}` **only** for
  `sanitized_html` produced by `crates/domain/src/document.rs`. Never render
  author text any other way.
* Run the frontend with `bash frontend/scripts/fe.sh build|test|check` if
  `npm run` cannot find the binaries (this checkout lives on an sshfs mount
  where `node_modules/.bin` is not executable).

### 2.6 Testing and honesty

* Rust: `cargo test --workspace`. Acceptance tests live in
  `crates/app/tests/milestone_N.rs`, run against the **real router**, a real
  SQLite file and a cookie jar that mimics a browser. Copy the harness from
  `milestone_3.rs`.
* Frontend: `vitest`, in `*.test.ts` beside what it tests.
* A test asserts the property, not the implementation. "A stale save writes
  nothing at all" is a property; "update_work returns Ok(false)" is not.
* When you find a bug while doing anything else, fix it and write the test
  that would have caught it. The session summaries list eight defects found
  this way, all of which reading the code had not revealed.
* **Do not claim something works until you have run it.** `verification.md`
  has a status vocabulary for exactly this; use `implemented but not executed`
  when that is the truth.
* Every milestone ends with a tag named in `docs/tutorial/README.md`
  (`v0.06-jobs` is the latest), and with `requirements.csv` rows whose
  `evidence` column names a command or a test — not a file path alone.

---

## 3. The files in this directory

| File | Covers |
|---|---|
| `junior-implementation-plan.md` | **Start here.** The whole website, start to finish: prerequisites, the workflow loop, every milestone from 5 to 18 with its migration, domain types, routes, pages, tests and pitfalls, then the frontend rules and the cross-cutting work. Written so someone who has never seen this repository can implement a milestone without guessing. |
| `milestone-04-reader.md` | Milestone 4 in full: built, with its verification in `docs/verification.md` |
| `milestone-05-jobs.md` | Milestone 5 in full: built, with its verification in `docs/verification.md` |
| `milestones-05-18.md` | The remaining milestones, in summary form |
| `cross-cutting.md` | Work that every milestone touches: verification, migrations, performance budgets, accessibility |

`junior-implementation-plan.md` and `milestones-05-18.md` cover the same ground at
different depths. When they disagree, the junior plan wins and the summary is
corrected in the same commit.

Read the milestone you are about to build, in full, before opening an editor.
