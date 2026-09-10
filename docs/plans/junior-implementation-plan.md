# The implementation plan, start to finish

This is the plan for the **whole website**: Lorehaven, a self-hosted fanfiction
platform with a forum. It is written for someone who knows Rust and Svelte but
has never seen this repository, and it is detailed enough to be followed from
an empty morning to a tagged release without asking anyone a question.

Two other documents sit beside it and neither is optional:

* `docs/spec.md` — *what* the platform must do, section by section. When this
  plan and the spec disagree, the spec wins and this plan is the thing that is
  wrong. Every milestone below names the spec sections it implements.
* `docs/plans/README.md` — the house rules and the reasoning behind them. They
  are repeated in Part 1 in shortened form because a rule you have to go and
  look up is a rule that gets skipped; the long version in `README.md` says why
  each one exists, with the defect that caused it.

`docs/requirements.csv` and `docs/verification.md` are the bookkeeping. They are
updated **as part of** finishing a milestone, in the same commit, never "later".

---

## Part 0 — How to work

### 0.1 Where the project is on the day you start

```text
Milestone 0  Platform skeleton          done     tag v0.01-running-app
Milestone 1  Design system, navigation  done
Milestone 2  Accounts, pseuds, privacy  done     tag v0.03-identity
Milestone 3  Drafts, chapters, publish  done     tag v0.04-publishing
Milestone 4  Reader, ratings, history   done     tag v0.05-reader
Milestones 5–18                         not built
```

`README.md` at the repository root explains how to run it. The short version:

```bash
just migrate        # create the database and apply every migration
just seed           # a development account and a little content
just serve          # http://127.0.0.1:8080
```

### 0.2 The loop, for every single milestone

Do not reorder these. The order is what makes the work checkable.

```text
1. read the milestone's spec section, in full, before opening an editor
2. migration  (both dialects, identical ids)
3. domain     types and pure policy functions, with their unit tests
4. repository the SQL, one statement written twice
5. routes     register them or they do not exist
6. pages      the interface
7. drive the journey by hand in a browser, both themes, 320px wide
8. write the tests that pin what the journey proved
9. update requirements.csv and verification.md
10. cargo fmt/clippy/test + the frontend build and tests
11. commit, then tag
```

Steps 7 and 8 are the pair that gets collapsed by people in a hurry and the pair
that matters most. A test written before the journey is a test written against
what you *meant*; a test written after it is a test against what the code does.

### 0.3 Definitions you will need

* **Vertical slice.** One journey end to end — migration, domain, repository,
  route, page — checked by hand, and only then broadened. Never all the
  migrations, then all the routes, then all the pages.
* **Dual dialect.** Every migration and every statement exists twice, for SQLite
  and PostgreSQL, with identical ids and identical semantics. A test fails if
  the migration ids drift apart.
* **The envelope.** Every collection answers with
  `{ "items": [...], "next_cursor": null }` (spec §3.3). Not a bare array.
* **A tag.** Every milestone ends with a git tag named in
  `docs/tutorial/README.md`. Tags are the milestones; do not invent new names.
* **Done.** A milestone is done when: `cargo test --workspace`,
  `cargo clippy --all-targets --all-features -- -D warnings`,
  `cargo fmt --all -- --check`, the frontend build and the frontend tests all
  pass, the journeys have been driven in a browser, `requirements.csv` has no
  `unsupported` row left for that milestone, and the tag exists.

### 0.4 The commands

```bash
# everything, in the order CI runs it
cargo fmt --all -- --check
cargo clippy --all-targets --all-features -- -D warnings
cargo test --workspace
bash frontend/scripts/fe.sh build
bash frontend/scripts/fe.sh test

# one acceptance test while you are working on it
cargo test -p lorehaven-app --test milestone_5 a_claimed_job_is_not_claimed_twice

# one crate's unit tests
cargo test -p lorehaven-db
cargo test -p lorehaven-domain
```

`bash frontend/scripts/fe.sh` exists because this checkout lives on a mount
where `node_modules/.bin` is not executable. Use it when `npm run` cannot find
its own binaries. It takes `build`, `test`, `check` and `dev`.

### 0.5 Commits

One commit per coherent change, with a message that says **what was wrong** as
well as what you did. Look at the existing log before writing yours; every
message in it explains the defect it fixes. A commit that says "update files"
tells the next person nothing, and the next person is you in four months.

Do not squash a milestone into one commit. The history is read during a
post-mortem; a 40-file commit makes that impossible.

### 0.6 When you find a bug

Fix it, and write the test that would have caught it, in the same commit. Eight
of this repository's defects were found this way and **none** of them were found
by reading the code. If the bug is in another milestone's area and small, fix it
anyway and say so in the message.

---

## Part 1 — The machine you are building on

### 1.1 The shape of the repository

```text
Cargo.toml                    workspace
crates/domain/                types, policies, the document schema. No I/O.
  error.rs                    AppError: every failure mode, with a code and a status
  ids.rs                      AccountId, PseudId, WorkId, ChapterId, RevisionId
  policy.rs                   authorization as pure functions
  content.rs                  content policy (can_access_content, MIN_PUBLIC_RATINGS)
  document.rs                 the restricted editor schema, to_sanitized_html()
  reading.rs                  position resolution, reading-time estimate
crates/db/                    every SQL statement, twice
  lib.rs                      Database, Backend, Database::sql(), sql_owned
  migrate.rs                  the embedded migration catalogue
  identity.rs content.rs collaboration.rs reading.rs sessions.rs outbox.rs
crates/app/                   the HTTP application
  server.rs                   build_router: the only place routes become reachable
  auth.rs                     sessions, RequireSession/RequirePseud/MaybeSession, CSRF
  http.rs                     ApiError, ApiResult, the error envelope
  limiter.rs                  per-class token buckets, fails closed
  routes/                     auth works reading pseuds settings collaborators meta health
  worker.rs                   (M5) the background worker
  assets.rs cli.rs config.rs doctor.rs logging.rs privacy.rs safety.rs seed.rs
frontend/                     SvelteKit-less Svelte 5 + Vite, embedded into the binary
  src/App.svelte              the shell and the route switch
  src/lib/router.ts           path → view; unbuilt paths resolve to `Planned`
  src/lib/api.ts              mirrors the routes exactly
  src/lib/components/         the primitives
  src/routes/                 one file per view
migrations/sqlite/000N_*.sql  and migrations/postgres/000N_*.sql — identical ids
docs/spec.md                  the specification
docs/plans/                   this plan
docs/requirements.csv         every requirement, with a status and evidence
docs/verification.md          the evidence behind every claim
docs/adr/                     decision records
```

### 1.2 The house rules, shortened

Read `docs/plans/README.md` §2 for the long form. These are the ones a newcomer
gets wrong:

**Database**

* Every statement is written twice and binds **only `String` and `i64`** (and
  `Option<…>` of those). PostgreSQL `uuid` columns are written `?::uuid` and
  read `id::text AS id`. This is what lets one row type decode on both engines
  (ADR 0004). A `f64` column is not allowed for exactly this reason — store a
  scaled integer instead (`position_permille`, `mean_permille`).
* `db.sql("… ?", "… ?::uuid")` picks the dialect and rewrites `?` to `$1…$n`
  for PostgreSQL. Use `db.sql_owned` when the statement is assembled at
  runtime, because `db.sql` borrows.
* Never put a literal `?` inside SQL text: it is always a placeholder.
* A function that needs several statements in one transaction writes the SQLite
  branch and the PostgreSQL branch out separately. Do not abstract over them.
* Every migration says, in a comment, its **deletion and retention** rule —
  what cascades, what soft-deletes, and why (spec §4.1).

**Errors and policy**

* Failures are `AppError`. A new failure mode gets a variant with a stable code,
  an HTTP status and a public message. Never return a raw `anyhow::Error`.
* Authorization lives in **pure functions** in `crates/domain/src/policy.rs`:
  `fn can_do_thing(actor, facts) -> Decision`. No database, no clock, no I/O.
  `if account_id == ...` inside a handler is the thing this rule forbids.
* Reading content goes through `can_access_content` and nothing else. If you
  need a second check, add a fact to `ContentFacts` instead — a second check is
  how restricted content eventually leaks.
* A resource the caller may not reach is `404`, not `403`, whenever saying
  "forbidden" would confirm that it exists. `AppError::NotFound` takes a coarse
  noun ("work"), never an identifier.

**Optimistic concurrency**

* Every editable row has `version`. Every mutating statement carries
  `… AND version = ?`. Zero rows affected means the caller lost a race: re-read
  and return `AppError::RevisionConflict { expected, actual }`. **Never**
  `SELECT` then `UPDATE` without the version predicate.

**HTTP**

* A route that is not registered in `build_router` does not exist. There is no
  discovery.
* Every route tree is wrapped in `classified(...)`, which declares its rate-limit
  class. The limiter **fails closed**: a tree merged without `classified` returns
  500 to every request, and this has already happened once.
* Cookie-authenticated state changes need the CSRF layer, which
  `build_router` applies to the `account_routes` subtree. `Write`-class routes
  belong under it.
* Handlers that need a session take `RequireSession` (or `RequirePseud`);
  handlers a visitor may reach take `MaybeSession`. Extracting the extractor
  **is** the authentication check.

**Frontend**

* Svelte 5 runes: `$state`, `$derived`, `$effect`, `$props`. No stores, no
  `export let`.
* A field component takes a `$bindable` value. `bind:value` without
  `$bindable()` compiles and silently does nothing; this shipped once and every
  form submitted empty.
* `api.ts` mirrors the routes exactly and never navigates on a failure.
* A linked-but-unbuilt destination resolves to `Planned` and says which
  milestone will fill it. Never to mock data.
* Server HTML is rendered with `{@html}` **only** for `sanitized_html` produced
  by `crates/domain/src/document.rs`.

**Honesty**

* Do not claim something works until you have run it. `verification.md` has a
  status vocabulary for exactly this; use `implemented but not executed` when
  that is the truth.
* A test asserts the property, not the implementation. "A stale save writes
  nothing at all" is a property. "update_work returns Ok(false)" is not.

---

## Part 2 — The milestones

Each milestone below has the same seven parts: the journey it delivers, the
migration, the domain code, the repository, the routes, the pages, the tests,
and the pitfalls. The house rules from Part 1 apply to every task and are not
repeated.

---

### Milestone 5 — Jobs, storage, cache and secrets

Spec §10. Tag `v0.06-jobs`. **This milestone unlocks M6 and M7; do not skip it.**

#### The journey

A signed-in writer uploads a file, and instead of the request doing the work the
server answers `202` with a job id; the page watches the job's progress and the
job ends. Then the writer cancels a second job mid-flight and the worker stops
at the next checkpoint rather than running to completion.

#### Why first

Importing is a background job. Every export is a job that writes a file to
storage. Nothing in M6 or M7 can be written honestly before there is a job model
with leases and retries. An import "for now, inline in the request" times out on
a 300-chapter work and the retry does it all again from scratch.

#### Migration 0005 — `jobs`, `job_attempts`, `content_blobs`, `content_references`, `encryption_keys`, `secrets`

```text
jobs
    id, kind, state, payload (TEXT, JSON), idempotency_key (nullable),
    priority INTEGER, attempts INTEGER, max_attempts INTEGER,
    available_at TEXT, lease_owner TEXT (nullable), lease_expires_at TEXT
    (nullable), progress_permille INTEGER, checkpoint TEXT (nullable),
    last_error TEXT (nullable), requested_by (nullable, accounts),
    created_at, updated_at, version
    UNIQUE (idempotency_key) WHERE idempotency_key IS NOT NULL
    INDEX (state, available_at)

job_attempts
    id, job_id, attempt INTEGER, started_at, finished_at (nullable),
    outcome TEXT (nullable), error TEXT (nullable), worker TEXT

content_blobs
    checksum TEXT PRIMARY KEY, storage_key TEXT, byte_size INTEGER,
    content_type TEXT, created_at, last_referenced_at
    -- content-addressed: the checksum *is* the identity

content_references
    id, checksum, owner_type, owner_id, created_at
    INDEX (checksum)          -- the deletion check reads this
    UNIQUE (checksum, owner_type, owner_id)

encryption_keys
    key_id TEXT PRIMARY KEY, algorithm TEXT, created_at, retired_at (nullable)

secrets
    id, owner_type, owner_id, name, key_id, nonce, ciphertext,
    created_at, updated_at, version
    UNIQUE (owner_type, owner_id, name)
```

Retention comments to write in the file:

* `jobs` and `job_attempts` are kept for 30 days after reaching a terminal state
  and then deleted by a maintenance job; they are diagnostics, not history.
  `requested_by` is kept even after the account is deleted, so an operator can
  see that *a* job ran — set it NULL on account deletion rather than cascading.
* `content_blobs` are never deleted by cascade. A blob is removed only when
  `content_references` has no row for its checksum, and that check is the only
  thing standing between an unused blob and data loss. Say so in the comment.
* `content_references` cascade with their owner (`work`, `chapter_revision`,
  `export`, `library_item`).
* `secrets` cascade with their owner. The ciphertext goes with the row; the key
  lives outside the database and is never stored in it.

#### Domain — `crates/domain/src/jobs.rs`

```rust
pub enum JobState { Queued, Leased, Running, Succeeded, Failed, Cancelled }
pub enum JobKind { Import, Export, Reindex, Notify, Thumbnail, Maintenance }

pub struct RetryPolicy { pub max_attempts: u32, pub base_delay: Duration,
                         pub backoff: f64, pub jitter_permille: u16 }

/// The moment a failed attempt may be retried. Pure: `now` is passed in.
pub fn next_attempt_at(attempt: u32, policy: &RetryPolicy, jitter_seed: u64,
                       now: OffsetDateTime) -> OffsetDateTime;

/// Whether a job may be cancelled in its current state. A terminal job may not.
pub fn can_cancel(state: JobState) -> bool;

/// The state a job moves to when a worker reports an outcome.
pub fn next_state(state: JobState, outcome: AttemptOutcome) -> JobState;
```

Unit tests: `a_first_failure_waits_the_base_delay`; `each_retry_waits_longer`;
`a_retry_is_never_sooner_than_the_base_delay` (the jitter must be able to make it
later but never earlier, or a stampede gets worse); `a_finished_job_cannot_be_cancelled`.

#### Repository — `crates/db/src/jobs.rs`

```text
enqueue(db, kind, payload, idempotency_key, requested_by) -> JobId
claim_next(db, worker, lease_secs, now) -> Option<Job>
heartbeat(db, job, worker, lease_secs) -> bool
complete(db, job, worker) -> ()
fail(db, job, worker, error) -> ()        // reschedules per the retry policy
cancel(db, job) -> bool
requeue_expired_leases(db, now) -> u64
progress(db, job, permille, checkpoint) -> ()
jobs_for(db, account, limit) -> Vec<JobRow>
```

**The claim is one statement.** Not a select followed by an update:

```sql
-- SQLite, inside a transaction
UPDATE jobs SET state = 'leased', lease_owner = ?, lease_expires_at = ?,
                updated_at = ?, version = version + 1
 WHERE id = (SELECT id FROM jobs
              WHERE state IN ('queued') AND available_at <= ?
              ORDER BY priority DESC, available_at ASC
              LIMIT 1)
RETURNING id;

-- PostgreSQL, no transaction needed around it
UPDATE jobs SET ... FROM (SELECT id FROM jobs
                           WHERE state = 'queued' AND available_at <= ?
                           ORDER BY priority DESC, available_at ASC
                           LIMIT 1
                           FOR UPDATE SKIP LOCKED) AS claimed
 WHERE jobs.id = claimed.id
RETURNING jobs.id;
```

Two workers racing must not both get the same row. That is the whole point of
`FOR UPDATE SKIP LOCKED`, and on SQLite the write lock does the same job.

#### Storage — `crates/db/src/storage.rs`

```text
put(db, bytes, content_type) -> (checksum, storage_key)   // idempotent
get(db, checksum) -> Option<Vec<u8>>
stat(db, checksum) -> Option<BlobStat>
reference(db, checksum, owner_type, owner_id) -> ()
unreference(db, checksum, owner_type, owner_id) -> ()
delete_if_unreferenced(db, checksum) -> bool
```

Files land at `storage/objects/<first two hex>/<checksum>`. `put` writes to a
temporary file in the same directory and renames it into place, so a crash never
leaves a half-written blob under a name that claims to be complete. Re-putting
the same bytes must not change `last_referenced_at` in a way that resurrects a
blob something else is deleting.

#### Secrets — `crates/app/src/secrets.rs`

`xchacha20poly1305`, a key from `LOREHAVEN_SECRET_KEY` or a key file, a random
nonce per record, and the `key_id` recorded on the row so a rotation can
re-encrypt lazily. The associated data is `owner_type|owner_id|name`, so a
ciphertext moved to another row fails to open rather than silently decrypting.

**Never log a plaintext or a key.** The `Debug` implementation of the wrapper
type prints `<secret>` and nothing else, so a stray `?secret` in a log line
cannot leak one. Write that as a test: `a_secret_is_not_in_the_logs`.

#### Worker — `crates/app/src/worker.rs`

Started by `lorehaven serve --with-worker` or by its own `lorehaven worker`
subcommand. Loop: claim → check for cancellation → do one unit of work →
heartbeat → repeat → complete or fail. Claims expire, so a worker that is killed
mid-job leaves a lease that `requeue_expired_leases` returns to the queue.

Graceful shutdown: on `SIGTERM`/`SIGINT`, stop claiming, finish the unit in
flight, release the lease, exit. A job interrupted mid-way must be resumable
from its `checkpoint`, not from the beginning.

#### Outbox delivery

The worker drains `outbox_events`, which Milestone 3 has been writing since it
shipped and which nothing has ever read (`verification.md` lists this as an open
risk). A failing topic retries with backoff and records `last_error`. Do not
delete an event until its handler returns success.

#### Routes and pages

```text
POST   /jobs/:id/cancel                 Write class, RequireSession
GET    /jobs?cursor=…                   the caller's own jobs, envelope
GET    /admin/jobs?state=…&cursor=…     operators only
POST   /admin/jobs/:id/retry            operators only
```

The `admin` routes need an operator, and **there is no staff model yet**. Add
`config.administration.operator_account_id: Option<AccountId>` and gate the
routes on it, with a comment saying plainly that M13 replaces this with a trust
level. Do not invent a boolean `is_admin` column — it will survive into
production and be wrong.

Pages: `/jobs` (the caller's own queue, with cancel) and `/admin/jobs` (a table
with filters, retry, cancel). A job in progress shows its `progress_permille` and
its checkpoint; a failed job shows `last_error`.

#### Tests

```text
a_claimed_job_is_not_claimed_twice
a_lease_that_expires_is_requeued
a_cancelled_job_stops_at_the_next_checkpoint
a_retry_uses_the_backoff
replaying_one_idempotency_key_enqueues_one_job
the_same_bytes_stored_twice_share_one_blob
deleting_one_reference_keeps_the_blob
deleting_the_last_reference_removes_the_blob
a_secret_is_not_in_the_logs
a_ciphertext_moved_to_another_row_does_not_open
```

#### Pitfalls

1. **Cancellation is checked between units of work, not only at the start.** A
   cancel that only takes effect at the beginning is a lie about a job that runs
   for ten minutes.
2. **A lease must expire.** Without expiry, one killed worker takes a job out of
   the queue forever.
3. **`delete_if_unreferenced` is the only safe deletion.** A "clean up old
   blobs" job that deletes by age will delete a blob the reader is streaming.
4. **Do not hold a database transaction across the network.** Claim in one
   transaction, release it, do the I/O, then record the outcome.
5. **The worker is a second entry point into every table you have.** Anything it
   writes must go through the same repository functions, or the invariants will
   hold in the web path and not in the worker.

---

### Milestone 6 — Imports, source credentials, batches and preservation

Spec §14 tags `M6`; the import flow is spec §12.2. Tag `v0.07-imports`.

#### The journey

A reader pastes a work's URL from a supported source, sees a metadata preview
(title, author, chapter count, rating), picks a destination and confirms. The
import is queued as a job, they watch it fetch chapters one by one, and it ends
as a library item they can read. A second run of the same URL updates the item
instead of duplicating it.

#### Migration 0006 — import framework

```text
sources              id, key, display_name, adapter_version, enabled,
                     capability_json, created_at, updated_at, version
source_credentials   id, account_id, source_key, secret_id, label,
                     expires_at (nullable), last_checked_at (nullable),
                     status TEXT, created_at, updated_at, version
                     UNIQUE (account_id, source_key, label)
import_jobs          id, job_id (jobs), account_id, source_key, source_url,
                     destination_type, destination_id (nullable),
                     dry_run INTEGER, state TEXT, report_json (nullable),
                     created_at, updated_at, version
import_chapters      id, import_job_id, source_chapter_key, ordinal INTEGER,
                     state TEXT, content_blob_checksum (nullable),
                     chapter_id (nullable), note (nullable)
                     UNIQUE (import_job_id, source_chapter_key)
library_items        id, account_id, work_id (nullable), source_key,
                     source_work_key, title, author_text, summary,
                     last_synced_at (nullable), provenance_json,
                     created_at, updated_at, version
                     UNIQUE (account_id, source_key, source_work_key)
```

Retention: `import_jobs` and `import_chapters` are **permanent** — they are the
provenance record, and `library_items.provenance_json` points back at them.
`source_credentials` cascade with the account, and the row's `secret_id` cascade
removes the ciphertext with it. An imported copy is never overwritten
destructively (spec §14.4): a new snapshot is created and the reader's notes,
shelves, bookmarks, ratings and progress are mapped onto it, with a visible
notice for removed, reordered or substantially changed chapters.

#### Domain — `crates/domain/src/imports.rs`

```rust
pub struct SourceCapabilities { pub chapters: bool, pub metadata: bool,
    pub authentication: AuthKind, pub incremental: bool, pub rate_hint: Option<u32> }

pub struct FetchedWork { pub source_key: String, pub title: String,
    pub author_text: String, pub summary: String, pub chapters: Vec<FetchedChapter> }

/// What an import would do, decided without touching anything.
pub enum ImportPlan { Create, Update { changed: Vec<ChapterChange> }, NoChange }
pub fn plan_import(existing: Option<&LibraryItem>, fetched: &FetchedWork) -> ImportPlan;

/// Whether a duplicate is the same work under a different source key.
pub fn looks_like_a_duplicate(a: &FetchedWork, b: &LibraryItem) -> bool;
```

#### The source adapter trait

```rust
#[async_trait]
pub trait SourceAdapter: Send + Sync {
    fn key(&self) -> &'static str;
    fn capabilities(&self) -> SourceCapabilities;
    async fn preview(&self, url: &str, creds: Option<&Credentials>) -> Result<FetchedWork>;
    async fn fetch_chapter(&self, work: &FetchedWork, ordinal: u32,
                           creds: Option<&Credentials>) -> Result<FetchedChapter>;
}
```

Adapters live in `crates/scrapers/` (a new crate), one module per source, each
with its own recorded fixtures. **Never write an adapter against the live site
in a test**: record the response to `tests/fixtures/<source>/<case>.html` and
parse the file. A test that reaches the network is a test that fails on a plane.

Rate limits: honour the source's `robots.txt`, wait between requests, and put the
wait in the adapter so no caller can forget it. A 429 from a source is a *retry
later*, not a failure of the import.

#### Routes

```text
GET    /imports/sources                     the catalogue, with capabilities
POST   /imports/preview                     { url } → FetchedWork, no writes
POST   /imports                             { url, destination, dry_run } → 202 + job id
GET    /imports/:id                         the report
POST   /imports/:id/retry-failed-chapters   re-fetch only what failed
GET    /source-credentials                  the caller's connections
PUT    /source-credentials/:source          store a credential (Write class)
DELETE /source-credentials/:source/:label
GET    /library/items?cursor=…              the caller's imported works
```

`POST /imports/preview` reads and parses but **writes nothing**. That is what
makes "preview" honest; a preview that has already imported is a trap.

#### Pages

`/import` — a URL box, the preview, a destination picker, a dry-run option, a
confirm button. `/library` gains a list of imported items with their provenance
and a "check for updates" action. The job page from M5 shows the import's
progress chapter by chapter.

#### Tests

```text
a_preview_writes_nothing
an_import_is_queued_and_does_not_block_the_request
 importing_the_same_url_twice_updates_rather_than_duplicates
a_failed_chapter_is_retried_without_refetching_the_rest
an_expired_credential_is_reported_before_the_import_starts
an_imported_copy_reports_removed_and_reordered_chapters
the_adapter_is_not_called_when_the_source_is_disabled
```

#### Pitfalls

1. **Never store a source password in plain text.** It goes through
   `crates/app/src/secrets.rs` from M5 or it does not go anywhere.
2. **Never let an adapter see the database.** It gets a URL and credentials and
   returns data. An adapter that writes rows cannot be tested and cannot be
   audited.
3. **A source that changes its HTML must fail loudly.** A parser that silently
   returns zero chapters produces an empty library item that looks like success.
4. **`library_items` is per account.** Two readers importing the same URL get
   two items; they are private copies, not catalogue entries.
5. **Preservation imports are a different thing and need a permission basis**
   (spec §14.5). Do not build them by waving a flag in M6; they arrive in M17
   with the operator role and a dry-run report.

---

### Milestone 7 — Exports, device delivery and offline reading

Spec §15. Tag `v0.08-exports`.

#### The journey

A reader exports a work as EPUB, is told where it will be delivered, picks a
format, confirms a privacy notice, and downloads it when the job finishes. Then
they install the site on their phone, go offline, and still read a chapter they
opened before.

#### Migration 0007 — exports and offline

```text
export_jobs        id, job_id, account_id, subject_type, subject_id,
                   format TEXT, options_json, privacy_acknowledged_at,
                   state, output_blob_checksum (nullable), created_at,
                   updated_at, version
download_grants    id, export_job_id, token_hash, expires_at, used_at
                   (nullable), single_use INTEGER, created_at
device_deliveries  id, export_job_id, target TEXT, address, state,
                   last_error (nullable), created_at, delivered_at
user_devices       id, account_id, label, push_subscription_json (nullable),
                   last_seen_at, created_at, updated_at
```

Retention: exports and their output blobs are deleted 7 days after creation by a
maintenance job; the `download_grants` row goes with it. `device_deliveries`
persist for diagnostics and are not cascaded away, because "we sent it and it
bounced" is exactly the thing you need later. The delivery address is personal
data: it is written to the audit log as a hash, never in the clear.

#### Formats

```rust
pub enum ExportFormat { PlainText, Html, Epub, Pdf, Mobi }
pub fn render(work: &ExportWork, format: ExportFormat, options: &ExportOptions) -> Result<Vec<u8>>;
```

EPUB is built from the sanitized HTML the reader already renders, plus a
generated OPF/NCX and one XHTML file per chapter. CSS lives in one file inside
the package; a chapter's own styling is not carried over, because the reader's
`reader_theme` is not the exporter's business.

PDF and MOBI shell out to `pandoc` and `ebook-convert`. `doctor` already detects
them; the exporter must use the detected path, report a clear error when the
tool is missing, and never pretend it produced a file it did not. Mark those two
formats `unsupported` in the interface when the tool is absent — the interface
must not offer what the server will refuse.

#### Offline (PWA)

* A service worker registered from `frontend/src/main.ts`.
* Cache-first for the app shell and its hashed assets; they are immutable.
* Network-first with a cache fallback for `GET /works/:id/chapters/:chapter`, so
  a chapter read once is readable offline.
* **Never cache a response for a mutation, and never cache a response carrying
  `Cache-Control: no-store`.** A cached 200 for `GET /auth/me` after a sign-out
  is a security bug, not a performance win.
* The offline reading list is opt-in per work, stored in IndexedDB, and the
  reader is told what is available offline and what is not.

#### Tests

```text
an_export_is_a_job_not_a_request
the_download_grant_expires_and_is_single_use
the_plain_text_export_matches_the_rendered_text
an_epub_export_opens_and_contains_every_chapter
an_unsupported_format_is_refused_before_a_job_is_created
the_export_privacy_notice_must_be_acknowledged
```

#### Pitfalls

1. **A download URL is a capability.** It is a random token, stored hashed, and
   it expires. Do not put the work id in it.
2. **Do not export a draft.** The export loads through `can_access_content`, the
   same as the reader. A second check here is the leak.
3. **The service worker must not survive a deploy.** Version the cache and
   delete the previous one on activate, or readers get a stale bundle forever.
4. **Never generate an empty file and call it success.** A work with no chapters
   is an error the reader can act on.

---

### Milestone 8 — Library, saved views, bookmarks and updates

Spec §16. Tag `v0.09-library`.

#### The journey

A reader's library is a place: shelves they made, private tags, reading statuses,
bookmarks with notes, a list of what updated since they last looked, and their
storage usage with a way to free space.

#### Migration 0008

```text
shelves           id, account_id, name, description, is_public INTEGER,
                  position INTEGER, created_at, updated_at, version
shelf_items       id, shelf_id, library_item_id, position INTEGER,
                  created_at, UNIQUE (shelf_id, library_item_id)
bookmarks         id, account_id, subject_type, subject_id, chapter_id
                  (nullable), position_permille (nullable), note,
                  created_at, updated_at, version
private_tags      id, account_id, subject_type, subject_id, tag,
                  created_at, UNIQUE (account_id, subject_type, subject_id, tag)
reading_status    id, account_id, subject_type, subject_id, status TEXT,
                  started_at, finished_at (nullable), updated_at, version
saved_views       id, account_id, name, query_json, created_at, updated_at
storage_usage     -- a view or a materialised column, not a new table
update_checks     id, account_id, library_item_id, checked_at,
                  found_changes INTEGER, report_json
```

Retention: everything here is private to the account and cascades with it.
`private_tags` and `reading_status` are **not** the public taxonomy of M9 — say
in the comment that a public tag lives in M9's `work_tags` and never here.
`update_checks` keeps 90 days.

#### Domain

```rust
pub enum ReadingStatus { WantToRead, Reading, OnHold, Dropped, Finished }
pub struct LibraryQuery { pub shelves: Vec<String>, pub tags: Vec<String>,
    pub statuses: Vec<ReadingStatus>, pub source: Option<String>,
    pub updated_since: Option<OffsetDateTime>, pub sort: LibrarySort }

/// A saved view is a query, checked before it is stored.
pub fn validate_query(query: &LibraryQuery) -> Result<(), QueryError>;
```

#### Routes

```text
GET/POST/PATCH/DELETE  /shelves[/:id]
POST/DELETE            /shelves/:id/items/:libraryItemId
GET/POST/PATCH/DELETE  /bookmarks[/:id]
PUT/DELETE             /library/items/:id/tags/:tag
PUT                    /library/items/:id/status
GET/POST/DELETE        /saved-views[/:id]
GET                    /library/items?<LibraryQuery as query params>   envelope
POST                   /library/updates/check                          → 202 + job
GET                    /library/storage
```

#### Pages

`/library` becomes a real library: a sidebar of shelves, a filter bar (source,
tag, status, updated-since), a grid of item cards, batch selection with batch
actions, and a storage panel with a "free space" action that says what it will
delete before it deletes it.

#### Tests

```text
a_shelf_is_private_until_it_is_published
a_private_tag_is_not_a_public_tag
batch_delete_removes_only_the_selection
a_saved_view_round_trips_its_query
deleting_a_library_item_leaves_the_reader_s_bookmarks_alone
storage_usage_matches_the_sum_of_the_items
```

#### Pitfalls

1. **Do not join `private_tags` into anything a second account can see.** This
   is the same rule as `reading_history_entry`.
2. **A private tag and a public tag are different rows in different tables.**
   A single `tags` table with an `is_private` column will be leaked by the first
   query that forgets the flag.
3. **Batch operations report per-item results**, not one boolean. "3 of 5
   removed, 2 were already gone" is the honest answer.

---

### Milestone 9 — Taxonomy, body search and the query language

Spec §17. Tag `v0.10-search`.

#### The journey

A reader searches for a work by fandom, relationship, character, rating, warning
and word count, with the results updating as they type, and finds a phrase in the
body of a chapter. A curator proposes a new tag and it enters a review queue.

#### Why before M10

Two of the recommendation engines read an inverted index and a tag graph.
Neither exists until this milestone has built them.

#### Migration 0009

```text
tags              id, slug, display_name, kind TEXT, parent_id (nullable),
                  created_at, updated_at, UNIQUE (kind, slug)
tag_aliases       id, tag_id, alias_slug, UNIQUE (alias_slug)
work_tags         work_id, tag_id, kind, source TEXT ('author'|'curator'|'auto'),
                  added_by (nullable), added_at, PRIMARY KEY (work_id, tag_id)
tag_proposals     id, proposed_by, kind, display_name, evidence, state,
                  reviewed_by (nullable), created_at, decided_at
work_search       -- SQLite: an FTS5 virtual table; PostgreSQL: tsvector + GIN
                  work_id, title, summary, body_text, taxonomy_text,
                  (tsvector column on PostgreSQL)
search_index_jobs id, work_id, revision_id, state, created_at, finished_at
```

Retention: tags are permanent and never cascade with a work — the taxonomy
outlives the works that used it. `work_tags` cascades with the work.
`tag_proposals` are kept after a decision, with `decided_at`, because "who asked
for this and who refused it" is the record that stops the same proposal arriving
every month.

**The two dialects differ here and that is allowed.** SQLite gets FTS5 with a
`porter unicode61` tokenizer; PostgreSQL gets `tsvector` with a GIN index. The
repository exposes the same functions; only the SQL differs. Write the divergence
down in an ADR, because it is the first place the two engines are not merely
syntactic variants of each other.

#### Domain — `crates/domain/src/search.rs`

```rust
pub struct WorkQuery { pub text: Option<String>, pub fandom: Vec<String>,
    pub characters: Vec<String>, pub relationships: Vec<String>,
    pub rating: Vec<Rating>, pub warnings: Vec<String>, pub status: Vec<String>,
    pub words: Option<RangeInclusive<u32>>, pub updated_since: Option<OffsetDateTime>,
    pub sort: SearchSort, pub cursor: Option<Cursor> }

/// Parses the query language (`fandom:hp rating:teen words:>5000 "a phrase"`).
pub fn parse_query(input: &str) -> Result<WorkQuery, QueryError>;

/// Builds the search terms. Pure, so it is testable without a database.
pub fn to_terms(query: &WorkQuery) -> SearchTerms;

/// The score contribution of each term, so ranking is explainable.
pub fn score(hit: &SearchHit, terms: &SearchTerms) -> i64;
```

**Unknown metadata is explicit** (spec §17): a work whose fandom nobody has
recorded is not "all fandoms" and is not hidden either. It is returned with a
marker, and the interface says "fandom not recorded" rather than guessing.

#### Routes

```text
GET    /search                         the query language, envelope
GET    /search/suggest?q=…             tag suggestions, ≤20, fast
GET    /tags/:kind/:slug               a tag page with its works
POST   /tags/proposals                 a proposal (Write)
GET    /tags/proposals?state=…         the queue
POST   /tags/proposals/:id/decide      accept or refuse, with a reason
```

#### Pages

`/search` with a filter rail that mirrors `WorkQuery` exactly, a result list with
the score explanation behind a disclosure, and a tag page. The filter rail and the
query string are two views of one object: changing a filter changes the URL, and
a shared URL reproduces the search.

#### Tests

```text
a_search_by_fandom_and_rating_narrows_the_result
a_phrase_search_finds_text_inside_a_chapter
a_work_with_no_fandom_is_marked_not_hidden
an_index_is_rebuilt_when_a_chapter_changes
an_unknown_filter_is_refused_rather_than_ignored
the_query_language_round_trips_through_a_url
a_proposal_needs_a_decision_and_a_reason
```

#### Pitfalls

1. **An unknown filter must be an error, not a silent no-op.** A search that
   quietly ignores `fandom:typo` shows everything and looks like it worked.
2. **Index maintenance is a job.** A chapter save enqueues a reindex; the search
   is stale for a moment and the page says so rather than lying.
3. **Do not search `private_tags`.** Ever.
4. **Explain the ranking.** A list of results with no explanation is
   unfalsifiable, and the first complaint about relevance cannot be answered.

---

### Milestone 10 — Discovery, private taste influence, recipes and dashboards

Spec §18. Tag `v0.11-discovery`.

#### The journey

A reader opens Discover and sees recommendations they can explain: each card
says why it is there ("because you finished X", "popular in a fandom you read").
They turn off one source of influence and the list changes. A blind-date card
shows a work without its author.

#### The rule that shapes the milestone

**Personal taste is private and off by default** (spec §18). The instance
operator chooses whether an aggregate signal exists at all; an individual
chooses whether their own reading contributes. Both switches exist, both default
to off, and the interface says which is which.

#### Migration 0010

```text
recommendation_settings  account_id PRIMARY KEY, use_history INTEGER,
                         use_ratings INTEGER, use_bookmarks INTEGER,
                         allow_blind_date INTEGER, updated_at, version
instance_discovery_config  id, aggregate_signals_enabled INTEGER,
                           updated_by, updated_at   -- instance-wide, one row
taste_signals            id, account_id, kind, subject_type, subject_id,
                         weight INTEGER, computed_at
                         -- derived, per account, never readable by another
recommendation_recipes   id, account_id (nullable = a built-in),
                         name, definition_json, is_public INTEGER, version
recommendation_runs      id, job_id, account_id, recipe_id, produced INTEGER,
                         created_at, finished_at
dashboards               id, account_id, name, layout_json, created_at, updated_at
```

Retention: `taste_signals` are derived data and are deleted when the reader turns
the feature off, not merely flagged — the point of the switch is that the data
stops existing. `recommendation_runs` keep 30 days. Built-in recipes have
`account_id IS NULL` and cannot be edited by a reader.

#### Domain — `crates/domain/src/discovery.rs`

```rust
pub struct Recommendation { pub work_id: WorkId, pub reason: Reason, pub score: i64 }
pub enum Reason { Finished(WorkId), SimilarTags(TagId), PopularInFandom(String),
                  SameAuthor(PseudId), BlindDate }

/// Whether a reader's signal may be used at all.
pub fn may_use(signal: SignalKind, settings: &RecommendationSettings,
               instance: InstanceDiscovery) -> bool;

/// Turns signals into candidates. Pure: rows in, candidates out.
pub fn recommend(signals: &[TasteSignal], candidates: &[Candidate],
                 limit: usize) -> Vec<Recommendation>;
```

Every recommendation carries a `Reason`. A recommendation with no reason is not
returned. That single rule is what makes the feature explainable.

#### Routes and pages

```text
GET    /discover                        the reader's own recommendations
GET/PATCH /settings/recommendations      the per-reader switches
GET/PATCH /admin/discovery               the instance switch, operators only
GET/POST/PATCH/DELETE /recipes[/:id]
GET/POST/PATCH/DELETE /dashboards[/:id]
```

`/discover` shows the reason on every card, a link to the settings, and — when
the instance switch is off — a plain statement that recommendations are disabled
here rather than an empty list pretending nobody has anything to recommend.

#### Tests

```text
recommendations_are_off_until_the_reader_turns_them_on
turning_the_setting_off_deletes_the_signals
a_recommendation_always_carries_a_reason
an_instance_with_signals_disabled_computes_nothing
a_blind_date_hides_the_author_until_it_is_opened
a_reader_cannot_see_another_readers_signals
```

#### Pitfalls

1. **A "similar readers also liked" feature is a disclosure.** If the instance
   operator enables aggregation, the interface must say so in plain words.
2. **Do not compute recommendations in a request.** They are a job, cached, and
   a reader sees the last run with its timestamp.
3. **A blind date that reveals the author in the markup is not blind.** Check
   the network tab, not the screen.

---

### Milestone 11 — Comments, forums, groups and messaging

Spec §12, §19. Tag `v0.12-community`.

#### The journey

A reader leaves a comment on a chapter; because the author has not opted into
auto-delivery, it is held for a moderator and the reader is told so in those
words. Elsewhere a group holds a discussion thread, and two readers exchange a
private message that respects a block.

#### Migration 0011

```text
comments          id, subject_type, subject_id, author_pseud_id, body,
                  classification TEXT ('positive'|'ambiguous'|'negative'),
                  state TEXT ('delivered'|'held'|'hidden'|'approved'),
                  parent_id (nullable), created_at, updated_at, version,
                  deleted_at
comment_revisions id, comment_id, body, edited_at
classifications   id, comment_id, decided_by, decision, rationale, decided_at
forums            id, slug, name, description, visibility, created_at, version
forum_threads     id, forum_id, title, author_pseud_id, pinned INTEGER,
                  locked INTEGER, created_at, updated_at, version
forum_posts       id, thread_id, author_pseud_id, body, parent_id (nullable),
                  created_at, updated_at, version, deleted_at
groups            id, slug, name, description, visibility, created_at, version
group_members     id, group_id, pseud_id, role, joined_at, UNIQUE (group_id, pseud_id)
messages          id, sender_pseud_id, recipient_pseud_id, body, read_at
                  (nullable), created_at, deleted_at
blocks            id, blocker_pseud_id, blocked_pseud_id, created_at, UNIQUE (…)
mutes             id, muter_pseud_id, muted_pseud_id, created_at, UNIQUE (…)
```

Retention: `comments`, `forum_posts` and `messages` soft-delete so a moderation
record survives; a hidden comment is kept with its classification and its
decision. `classifications` are permanent — they are the evidence for a sanction.
`blocks` and `mutes` are one-way and permanent until removed by the person who
set them: **a block is never removed by blocking back.**

#### The classification pipeline (spec §12.4)

```rust
pub enum Classification { Positive, Ambiguous, Negative }

/// The rule, in one pure function, so the same comment cannot be classified
/// two ways in two code paths.
pub fn classify(comment: &str, author_prefs: FeedbackPreferences) -> Classification;

pub enum Delivery { AutoDeliver, HoldForReview, Hide }
pub fn deliver(classification: Classification, prefs: FeedbackPreferences) -> Delivery;
```

The defaults are the spec's: positive is auto-delivered, ambiguous is held unless
the author has enabled auto-delivery, negative is hidden from the author, held
for a moderator, and **never surfaces publicly on the work page**. The reader is
told "Comment held for moderator review." — the honest sentence, not "posted".

Blocks are enforced in `can_access_content`'s neighbours, not in each handler: a
blocked reader must not be able to comment, reply, quote, react, message, or
appear in a list. Write a single `fn may_interact(actor, target, facts) -> Decision`
in `policy.rs` and route every one of those through it. The moment there are two
checks, one of them will be missing.

#### Routes

```text
GET/POST/PATCH/DELETE  /works/:id/comments[/:commentId]
POST                   /comments/:id/approve | /hide
GET                    /forums  /forums/:slug/threads  /threads/:id
POST                   /forums/:slug/threads  /threads/:id/posts
GET/POST               /groups[/:slug]  /groups/:slug/members
GET/POST               /messages  /messages/:id/read
GET/POST/DELETE        /blocks[/:pseudId]  /mutes[/:pseudId]
```

#### Pages

The work page gains a comment box that says where the comment will go before it
is sent. `/community` becomes a forum index; a thread is a paginated post list.
`/messages` is a two-pane inbox. A blocked person does not appear as a gap or a
"[blocked]" row — they are absent, and the interface does not explain the
absence, because explaining it discloses the block.

#### Tests

```text
a_negative_comment_is_never_shown_to_the_author
an_ambiguous_comment_is_held_unless_the_author_opted_in
a_blocked_reader_cannot_comment_reply_or_message
blocking_is_one_way_and_blocking_back_does_not_unblock
a_hidden_comment_keeps_its_classification_record
a_message_respects_a_block_set_after_it_was_started
```

#### Pitfalls

1. **The classification pipeline is one function.** A second implementation in
   a forum route is how the forum ends up nicer than the comment box.
2. **Do not leak the existence of a block** through an error message, a
   timestamp, or an ordering difference.
3. **A moderator's decision is recorded, with a reason.** An unexplained
   moderation action is indistinguishable from a bug.
4. **Rate-limit before storing.** Spec §12.4: repeated negative comments trigger
   rate limits. Check the limiter before the insert, not after.

---

### Milestone 12 — Collections, challenges, requests and events

Spec §20. Tag `v0.13-collections`.

#### The journey

A moderator opens a gift exchange: a sign-up window, a matching run, a deadline,
and a reveal. A reader requests a translation of a work and follows the request
until it is fulfilled.

#### Migration 0012

```text
collections        id, slug, name, description, owner_pseud_id, visibility,
                   closed INTEGER, created_at, updated_at, version
collection_items   id, collection_id, work_id, state, added_by, added_at,
                   UNIQUE (collection_id, work_id)
challenges         id, collection_id, name, rules, opens_at, closes_at,
                   signup_closes_at, created_at, version
challenge_signups  id, challenge_id, pseud_id, offers_json, wants_json, state,
                   created_at, UNIQUE (challenge_id, pseud_id)
challenge_matches  id, challenge_id, giver_pseud_id, recipient_pseud_id,
                   revealed_at (nullable), created_at
requests           id, requested_by, kind ('translation'|'podfic'|'art'|'beta'),
                   subject_work_id, description, state, fulfilled_by (nullable),
                   created_at, updated_at, version
events             id, slug, name, description, starts_at, ends_at, created_at
event_participants id, event_id, pseud_id, created_at, UNIQUE (event_id, pseud_id)
```

Retention: a collection is permanent; its items cascade. A sign-up cascades with
the account. A match is **never deleted** once revealed — the person who received
a gift must not lose the record of it.

#### Domain

```rust
/// Sign-ups, matched into pairs. Deterministic for a given seed so a re-run
/// produces the same pairing and can be audited.
pub fn match_participants(signups: &[Signup], seed: u64) -> Vec<Match>;

/// Whether sign-ups are open.
pub fn signup_is_open(challenge: &Challenge, now: OffsetDateTime) -> bool;
pub fn offers_satisfy(offers: &Signup, wants: &Signup) -> bool;
```

A matching run is a **job** (M5), produces a report, and can be reviewed before
the matches are announced. Never match inside a request.

#### Routes and pages

```text
GET/POST/PATCH         /collections[/:slug]
POST/DELETE            /collections/:slug/items/:workId
GET/POST               /collections/:slug/challenges[/:id]
POST                   /challenges/:id/signup | /withdraw
POST                   /challenges/:id/run-matching      → 202 + job
POST                   /challenges/:id/reveal
GET/POST               /requests[/:id]  /requests/:id/fulfil
GET/POST               /events[/:slug]
```

Pages: `/collections/:slug`, a challenge page with the rules, a sign-up form, and
a "your match" page that stays hidden until the reveal.

#### Tests

```text
matching_is_deterministic_for_a_seed
a_signup_before_the_window_is_closed_is_refused
a_match_is_not_visible_before_the_reveal
a_request_can_be_fulfilled_once
withdrawing_a_signup_removes_it_from_matching
```

#### Pitfalls

1. **Never reveal matches early**, including in an API response the page does
   not display.
2. **A request is not a work.** Do not create a draft when someone asks; the
   fulfil action links the eventual work.
3. **The exchange is a deadline-sensitive feature.** Every date is stored in UTC
   and rendered in the reader's zone.

---

### Milestone 13 — Trust, reports, quorum, appeals and process feedback

Spec §21. Tag `v0.14-governance`. **This is the prerequisite for M17.**

#### The journey

A reader reports a work. The report enters a queue. A trusted moderator proposes
hiding it; a second moderator confirms and it is hidden; the author appeals, and
the appeal is decided by two people who were not in the original decision.

#### The trust model — and this is the milestone's real content

There is **no `is_admin` column**. There is no `role` column. Access is a
function of a numeric `trust_level` on the account plus the facts of the
decision.

```rust
pub struct TrustProfile { pub level: u8, pub standing: Standing, pub since: OffsetDateTime }
pub enum Standing { Good, Probation, Suspended }

/// Every privileged action goes through exactly this.
pub fn may_exercise(actor: &TrustProfile, action: PrivilegedAction,
                    facts: &ActionFacts) -> Decision;
```

Rules to implement, with a test each:

* A higher `trust_level` may do everything a lower one may; capability is
  monotone in level.
* An actor may never decide a case in which they are the subject, the reporter,
  or a contributor to the work. `conflict_of_interest(actor, case) -> bool`.
* A quorum is met only by **distinct accounts**: two pseuds of one account count
  once. Write `quorum_met(decisions, required) -> bool` and a test named
  `two_pseuds_of_one_account_are_one_decision`.
* A suspended account keeps its history and loses its powers.

#### Migration 0013

```text
trust_levels      account_id PRIMARY KEY, level INTEGER, standing TEXT,
                  granted_by (nullable), granted_at, reason, updated_at, version
reports           id, reporter_pseud_id, subject_type, subject_id, reason,
                  detail, state, created_at, resolved_at
cases             id, report_id (nullable), kind, subject_type, subject_id,
                  state, opened_at, closed_at, outcome
decisions         id, case_id, actor_account_id, actor_pseud_id, action,
                  rationale, created_at
appeals           id, case_id, appellant_pseud_id, argument, state,
                  created_at, decided_at
appeal_decisions  id, appeal_id, actor_account_id, action, rationale, created_at
process_feedback  id, account_id, subject_type, subject_id, body, created_at
```

Retention: every one of these is **permanent and never cascaded**, including for
a deleted account. A moderation record that vanishes when someone leaves is a
moderation record that can be erased by leaving. Set the actor columns NULL and
keep the row; the pseud handle is stored denormalised on the decision so the
record still reads correctly afterwards.

#### Routes and pages

```text
POST    /reports                      any reader
GET     /moderation/queue             trust_level ≥ 1
POST    /moderation/cases/:id/decide  with a rationale
POST    /moderation/cases/:id/appeal
GET     /moderation/appeals           deciders who were not in the original
POST    /moderation/appeals/:id/decide
POST    /feedback                     process feedback, any reader
GET/PATCH /admin/trust/:accountId     level changes, with a reason
```

Pages: a report dialog that says what happens next, `/moderation` with the queue
and a decision form that shows the case's full history, and an appeal page.

#### Tests

```text
a_trust_level_is_monotone_in_capability
an_actor_cannot_decide_their_own_case
two_pseuds_of_one_account_are_one_decision
a_second_moderator_is_required_for_a_repeat_offender
an_appeal_cannot_be_decided_by_the_original_decider
a_decision_without_a_rationale_is_refused
a_suspended_account_keeps_its_record_and_loses_its_powers
```

#### Pitfalls

1. **Do not put a trust check inline in a handler.** Every privileged action
   goes through `may_exercise`, or the seventh one will forget.
2. **A quorum counted by pseud is a quorum of one person with two faces.** This
   is the single most likely way this milestone ships broken.
3. **An appeal heard by the original decider is not an appeal.**
4. **Never delete a moderation record.** Not on account deletion, not on case
   closure.

---

### Milestone 14 — Credits, fair queues, bounties and billing

Spec §22. Tag `v0.15-credits`.

#### The journey

A reader earns credits for contributions, spends them to jump a queue, and sees
every movement in a ledger they can read. A writer posts a bounty for a
translation and the credits move when it is fulfilled.

#### The rule that shapes the milestone

**Credits are a ledger, not a balance.** There is a `credit_entries` table and
the balance is its sum. A single `balance` column that gets `+=` is a bug waiting
for the first crash between the debit and the credit.

#### Migration 0014

```text
credit_accounts   account_id PRIMARY KEY, held INTEGER, created_at, updated_at
credit_entries    id, account_id, delta INTEGER, reason, ref_type, ref_id,
                  created_at, idempotency_key (nullable)
                  UNIQUE (idempotency_key) WHERE idempotency_key IS NOT NULL
                  -- append-only. No UPDATE, no DELETE, ever.
queue_tickets     id, account_id, subject_type, subject_id, kind, priority
                  INTEGER, credits_spent INTEGER, created_at, used_at
bounties          id, posted_by, subject_type, subject_id, amount, state,
                  claimed_by (nullable), created_at, expires_at, released_at
billing_periods   id, account_id, period_start, period_end, state, created_at
invoices          id, account_id, period_id, amount_minor INTEGER, currency,
                  state, provider_ref, created_at, paid_at
```

Retention: `credit_entries` are **append-only and permanent**; a correction is a
new opposing entry with a reason, never an edit. `invoices` are permanent for the
retention period the operator's jurisdiction requires and then archived, not
deleted.

Moving credits is one transaction: insert the debit, insert the credit, check the
balance is not negative, commit. All three or none.

#### Domain

```rust
pub struct Ledger { /* the entries, in order */ }
impl Ledger { pub fn balance(&self) -> i64; pub fn is_consistent(&self) -> bool }

/// A transfer is refused if it would take the sender below zero.
pub fn plan_transfer(from: &Ledger, to: &Ledger, amount: i64) -> Result<Transfer, CreditError>;

/// What a queue position costs, from the instance's published schedule.
pub fn queue_price(kind: QueueKind, schedule: &CreditSchedule) -> i64;
```

#### Routes and pages

```text
GET     /credits                       the balance, and the ledger, envelope
GET     /credits/ledger?cursor=…
POST    /credits/transfer              Write, idempotency key required
POST    /queue/tickets                 buy a priority ticket
GET/POST /bounties[/:id]  /bounties/:id/claim  /bounties/:id/release
GET     /billing/periods  /billing/invoices
```

Page: `/credits` shows the balance large, the ledger below it, and — this is the
point — a plain sentence for every entry saying what it was for. A ledger of
numbers is not a ledger.

#### Tests

```text
a_credit_transfer_is_one_transaction
replaying_a_transfer_idempotency_key_moves_credits_once
a_balance_can_never_go_negative
the_ledger_sum_is_the_balance
a_bounty_release_pays_exactly_once
a_correction_is_a_new_entry_not_an_edit
```

#### Pitfalls

1. **Never mutate a ledger row.** The audit value of the table is the whole
   reason it exists.
2. **Check the balance inside the transaction**, not before it.
3. **Billing is the one area where a bug is a legal problem.** Say plainly in
   the interface what is charged, when, and how to cancel; a subscription that
   cannot be cancelled is not shippable.

---

### Milestone 15 — Marketplace, extension isolation and gallery mechanics

Spec §19 extension marketplace, §4.4. Tag `v0.16-extensions`.

#### The journey

An operator installs an extension from a package, grants it two permissions, sees
what it did, and revokes it. An extension that asks for a permission nobody
granted fails and says so.

#### The architecture, decided before any code

An extension runs **out of process**, in a sandbox, speaking a narrow protocol to
the core. It does not get a database handle, a session, or the user's cookie. It
gets a capability token scoped to the permissions the installer granted.

```rust
pub struct ExtensionManifest { pub id: String, pub version: String,
    pub permissions: Vec<Permission>, pub entrypoint: String,
    pub max_memory_bytes: u64, pub max_runtime_ms: u64 }

pub enum Permission { ReadWork(Scope), WriteWork(Scope), Network(Vec<String>),
                      Storage(u64), Notify, Webhook(Vec<String>) }

/// The decision, pure: is this call inside the grant?
pub fn may_call(grant: &Grant, call: &ExtensionCall) -> Decision;
```

#### Migration 0015

```text
extension_packages   id, package_id, version, manifest_json, checksum,
                     signature (nullable), published_by, created_at
extension_installations  id, account_id (nullable = instance-wide), package_id,
                     version, state, installed_by, installed_at
extension_grants     id, installation_id, permission, scope_json, granted_at,
                     revoked_at (nullable)
extension_calls      id, installation_id, call, decision, decided_at,
                     duration_ms, error (nullable)
extension_revocations id, installation_id, reason, revoked_by, revoked_at
extension_ratings    id, package_id, account_id, stars, review, created_at
extension_purchases  id, package_id, account_id, credits_spent, created_at
```

Retention: `extension_calls` keep 30 days; they are the audit trail that makes
"what did this thing do" answerable. `extension_grants` are permanent while the
installation exists and are **not** silently re-granted on upgrade — an upgrade
that wants a new permission asks again, and the interface shows what changed.

#### Routes and pages

```text
GET     /extensions                    the gallery
GET     /extensions/:id                the detail page, with permissions
POST    /extensions/:id/install        → 202 + job
POST    /extensions/:id/revoke
GET/PATCH /extensions/:id/grants
GET     /extensions/:id/activity       what it called, and what was refused
```

Page: the install dialog lists every permission in plain words ("read the works
you write", "make network requests to these three hosts") and there is no
"allow all".

#### Tests

```text
an_extension_without_a_grant_is_refused
revoking_a_grant_stops_the_extension_at_the_next_call
an_upgrade_that_asks_for_more_permissions_is_held_for_review
an_extension_cannot_reach_the_database
a_timed_out_extension_is_killed_not_leaked
the_activity_log_records_a_refusal_as_well_as_a_call
```

#### Pitfalls

1. **Never run extension code in the server process.** One panic takes the site
   down; one escape reads the database.
2. **A permission is checked on every call**, not at install time only.
3. **Never log the arguments of a call that carries a secret.**
4. **Do not auto-upgrade.** An extension that changes under the installer is a
   supply-chain attack with a friendly name.

---

### Milestone 16 — Public API, bots, feeds, push, federation and optional AI

Spec §23. Tag `v0.17-integrations`.

#### The journey

A reader subscribes to their favourite author's feed in an RSS reader. A bot
posts a new chapter through an API token with one scope. A push notification
arrives on a phone with generic lock-screen text.

#### Migration 0016

```text
api_tokens        id, account_id, pseud_id, name, token_hash, scopes_json,
                  last_used_at, expires_at (nullable), created_at, revoked_at
webhooks          id, account_id, url, secret_id, events_json, state, created_at
webhook_deliveries id, webhook_id, event, attempt, response_status,
                   error, delivered_at
push_subscriptions id, account_id, endpoint, keys_json, created_at,
                   last_success_at, failure_count
feed_tokens       id, account_id, token_hash, scope, created_at, revoked_at
federation_state  -- only if the operator opts in; see spec §23
ai_settings       id, account_id (nullable = instance), feature, enabled,
                  provider, model, updated_at
```

Retention: `api_tokens` are stored **hashed** like session tokens; the plaintext
is shown once at creation and never again. `webhook_deliveries` keep 30 days.
`push_subscriptions` are deleted on a 410 from the push service, because a dead
subscription is not data worth keeping.

#### The rules

* **An API token is scoped.** A token with `read:works` cannot post a comment.
  `may_call(grant, call)` from M15 is the same function, reused.
* **Push text is generic on the lock screen** (spec §23): "A new chapter is
  available", never the title of a work whose reading is private.
* **Federation is opt-in and off by default.** If it is on, the instance
  publishes only what the author has made public, and the settings page lists
  exactly what is published. A federated instance that leaks a draft is the
  worst failure in this document.
* **AI is optional per feature, off by default, and the interface says when it
  is used.** An AI-suggested tag is marked as such and enters the M9 review queue
  rather than being applied.

#### Routes

```text
GET/POST/DELETE  /api-tokens[/:id]
POST             /webhooks  GET /webhooks/:id/deliveries
POST             /push/subscriptions  DELETE /push/subscriptions/:id
GET              /feeds/works/:pseudHandle.rss | .atom | .json
GET              /feeds/tag/:slug.rss
GET/PUT          /settings/integrations
POST             /admin/federation/enable   (operators only, with a warning)
```

The public API lives under `/api/v1/public/…` and is documented in
`docs/api.md` with a worked curl example per endpoint. An undocumented endpoint
is an accident.

#### Tests

```text
a_token_without_the_scope_is_refused
a_revoked_token_stops_working_immediately
a_feed_contains_only_published_works
a_lock_screen_message_does_not_name_the_work
a_410_from_the_push_service_deletes_the_subscription
federation_publishes_nothing_the_author_has_not_published
an_ai_suggestion_is_marked_and_queued_not_applied
```

#### Pitfalls

1. **Show the token once.** A token you can read back is a token anyone with a
   database dump can use.
2. **A feed is a cache-busting surface.** It must respect `withdrawn_at` and the
   work's visibility, and it must never carry a draft.
3. **Webhooks are SSRF.** Resolve the URL, refuse private address ranges, and
   time out.

---

### Milestone 17 — Administration, statistics, abuse defence, privacy and ops

Spec §24. Tag `v0.18-operations`.

#### The journey

An operator opens the admin dashboard, sees the queue lengths and the error
rates, exports a reader's data for a subject access request, and runs a backup
that restores on a second machine.

#### Migration 0017

```text
instance_settings    key PRIMARY KEY, value_json, updated_by, updated_at
statistics_daily     day, metric, value INTEGER, PRIMARY KEY (day, metric)
abuse_signals        id, kind, subject_type, subject_id, weight, observed_at,
                     action_taken (nullable)
audit_log            id, at, actor_account_id (nullable), actor_type
                     ('account'|'operator'|'system'|'extension'),
                     action, subject_type, subject_id, detail_json, request_id
                     -- append-only, never updated, never deleted
data_requests        id, account_id, kind, state, requested_at,
                     fulfilled_at, artefact_blob_checksum (nullable)
backup_runs          id, started_at, finished_at, byte_size, checksum,
                     destination, state, error (nullable)
```

Retention: `audit_log` is **append-only and permanent**, and it never contains
personal data in the clear — a subject is an id, an address is a hash. Privacy
jobs themselves are logged; a data export that leaves no trace is a data export
nobody can be held to.

#### The admin surface

```text
GET     /admin                          the dashboard: queue lengths, error rates, storage
GET     /admin/jobs  /admin/jobs/:id
GET     /admin/reports  /admin/cases
GET     /admin/users/:accountId         with the trust history and the audit trail
POST    /admin/backups                  → 202 + job
GET     /admin/audit?cursor=…           the audit log, read-only always
POST    /data-requests                  a subject access or erasure request
POST    /data-requests/:id/fulfil       → 202 + job
```

**The audit log has no delete route.** Not for an operator, not for the
instance owner. If you find yourself writing one, the requirement is wrong.

#### Privacy tooling

An export produces a machine-readable archive of everything the platform holds
about an account. An erasure anonymises rather than deletes where a moderation
record needs the row: `decisions.actor_account_id` becomes NULL and the pseud
handle is replaced by `[deleted]`, while the case itself survives. That is the
honest reading of "right to erasure" against "moderation record", and it must be
written in `docs/verification.md` where a reader can find it.

#### Tests

```text
the_audit_log_has_no_delete_path
an_erasure_anonymises_a_moderation_record_rather_than_destroying_it
a_data_export_contains_every_table_that_names_the_account
a_backup_restores_into_an_empty_database
an_operator_cannot_read_a_private_note
```

#### Pitfalls

1. **An operator is not a reader.** Trust level does not grant access to private
   notes, history, or ratings. If a case needs them, the case needs a warrant and
   a different feature.
2. **A dashboard that reads live tables will fall over.** Statistics are computed
   by a nightly job into `statistics_daily`.
3. **A backup that has never been restored is not a backup.** The restore is a
   numbered test, run in CI against SQLite and manually against PostgreSQL.

---

### Milestone 18 — Hardening and release

Spec §25. Tag `v1.0`.

This milestone is not a feature. It is the list of things you cannot ship
without.

#### Security

* Every route audited against `crates/domain/src/policy.rs`. Write the audit in
  `docs/verification.md` as a table: route, class, extractor, policy function,
  and the test that pins it. A route with a blank cell is a release blocker.
* `cargo audit` clean, or every advisory explicitly accepted with a written
  reason and a date.
* CSRF, CSP, `SameSite`, cookie flags, body limits and timeouts asserted in
  tests, not only configured.
* A password hashing parameter review (the cost factor), written down.
* Rate limiting exercised with a real burst against the running binary.

#### Correctness

* `cargo test --workspace` on SQLite **and** against a live PostgreSQL in CI.
  This is the open risk `verification.md` has carried since M0 and M18 is where
  it closes.
* The Playwright suite spec §23 asks for: one automated journey per milestone,
  driven in a browser. Milestones 2 and 3 both shipped frontend defects that the
  unit tests could not see.
* A load profile: the numbers in `docs/plans/cross-cutting.md` (a work page
  answers in under 200 ms at p95 with 100 concurrent readers) measured and
  recorded, with the machine's specification beside them.

#### Accessibility

* Every journey walked with a keyboard only, at 320 CSS pixels wide, with a
  screen reader, in all three themes and in both colour-scheme preferences.
* The results written into `verification.md` per screen, including the ones that
  failed and what was changed.

#### Documentation

* `docs/tutorial/` has a numbered page per milestone, each reproducible from an
  empty checkout.
* `docs/adr/` has a record for every decision this plan said to write down.
* `docs/api.md` covers the public API.
* A `CHANGELOG.md` whose entries are written for a reader, not a committer.

#### Data

* A restore rehearsed from the documented backup procedure on a different
  machine.
* A migration run against a copy of a real database, timed, with the downtime
  measured and stated.
* Retention jobs verified by running them and checking what they deleted.

#### Release

```text
tag v1.0
```

The release checklist is a file, `docs/release-checklist.md`, and the release is
not tagged until every box is ticked **with a command or a test next to it**.
Same rule as `verification.md`: an unticked box is fine, an untested tick is not.

---

## Part 3 — The frontend, across all milestones

The backend plan is per milestone. The frontend has its own shape and it is
worth reading once, here, rather than rediscovering it eighteen times.

### 3.1 The shell

```text
frontend/src/App.svelte      the header, the nav, the route switch, the drawer
frontend/src/lib/router.ts   matchRoute: path → RouteId
frontend/src/lib/api.ts      one function per route, mirroring it exactly
frontend/src/lib/session.svelte.ts   who is signed in, which pseud is acting
```

Adding a screen is three edits and all three are mandatory:

1. `router.ts` — a `RouteId`, a `matchRoute` branch, and a `PLANNED_ROUTES`
   entry if it is not built yet.
2. `App.svelte` — an `import` and a branch in the `{#if}` chain.
3. `api.ts` — the functions the page calls.

There is no fourth step and no discovery. A page that exists and is not in the
`{#if}` chain renders `NotFound`, which is exactly the bug that was found in
Milestone 4 when `/library/history` resolved to the history view but `App.svelte`
had no branch for it.

### 3.2 The rule for every page

**Do not offer what the server will refuse.**

* A visitor sees "Sign in to rate this work", not five stars that fail on click.
* A format whose converter is not installed is marked unavailable, not offered
  and then rejected.
* A reader below the public-rating threshold sees "not enough ratings yet", not
  a mean computed from two people.
* A permission the extension was not granted is not in its list.

This rule is not cosmetic. Every violation is a page that lies about the
server's behaviour, and the user finds out by being refused.

### 3.3 Components

`frontend/src/lib/components/` holds the primitives. Extend it; do not write a
fourth text input. A new component needs:

* `$props()` with `$bindable` on anything a form binds to,
* a visible focus state (`:focus-visible`),
* a label associated with the control,
* a test beside it if it has behaviour (see `Dialog.test.ts` for the shape).

As of M4 the missing listed primitives are the combobox and the richer work-card
variants (`requirements.csv` M1-03 tracks this). They arrive with M9's search and
M8's library respectively.

### 3.4 State

There are no stores. State is either:

* **local** — `$state` in a component,
* **shared and tiny** — a `.svelte.ts` module with runes
  (`session.svelte.ts` is the model),
* **on the server** — the truth, read with `api.ts`.

Never cache server state in a module-level variable to avoid a fetch. The second
tab is the test case and it will disagree.

### 3.5 Loading, empty and error

Every screen that fetches has four states and all four are designed:

```text
loading   a Skeleton that occupies the final layout's space
empty     an EmptyState that says what would be here and how to make one
error     an ErrorSummary with the request id, and a next action
content   the thing
```

An empty list rendered as nothing is indistinguishable from a failure. Say
"you have not read anything yet", not "".

### 3.6 Accessibility, per screen, every time

* Walk it with `Tab` only. If you cannot reach a control, it does not exist.
* Walk it at 320 CSS pixels. The breakpoint is 48rem/52rem depending on the
  shell; check both.
* Walk it in all three themes. `after-hours` is where contrast bugs live.
* Every image has an `alt`; every icon-only button has an `aria-label`; every
  form error is announced (`role="alert"` or a live region).
* Respect `prefers-reduced-motion` — `app.css` has the override and a component
  that animates must be inside it.

### 3.7 Performance budgets

From `docs/plans/cross-cutting.md`, restated because a budget nobody reads is a
budget nobody meets:

* Work page: under 200 ms p95 at 100 concurrent readers.
* Chapter read: under 300 ms p95, and the text starts rendering before the
  position and note requests have answered.
* Search: under 150 ms p95 for a query that returns 20 results.
* The main bundle stays under 200 KB gzipped; anything larger is a lazy import
  behind a route.

---

## Part 4 — Cross-cutting work

### 4.1 Verification, honestly

`docs/verification.md` uses a fixed vocabulary. Use it exactly:

```text
implemented-locally-tested      a command or a test exercises it, here
implemented-not-executed        the code exists and nothing has run it
partially-implemented           some of the requirement holds
unsupported                     not written
```

Every claim has an `evidence` value that is **a command or a test name**, never
a file path on its own. "`crates/app/src/routes/reading.rs`" is not evidence;
"`cargo test -p lorehaven-app --test milestone_4 a_private_rating_changes_no_public_number`"
is.

### 4.2 Migrations

* Two files per migration, identical ids, in `migrations/sqlite/` and
  `migrations/postgres/`.
* The comment at the top states the deletion and retention rule.
* A test fails if the ids drift.
* Never edit a migration that has been applied anywhere. Add the next one.
* Migrations are forward-only. There is no `down`. A mistake is corrected by a
  following migration, which is also the honest record of the mistake.

### 4.3 The database conventions, once more

Bind only `String` and `i64`. PostgreSQL uuids as `?::uuid`, read as `id::text`.
No floating point columns — scale to an integer and say the scale in the name
(`position_permille`, `mean_permille`, `amount_minor`).

### 4.4 Localisation

Spec §8 requires English and Spanish initially, and says not to advertise a
locale as complete until it has been reviewed. So:

* `frontend/src/lib/i18n/` with one file per locale, typed against the English
  one so a missing key is a compile error.
* No string concatenation across a variable: `"Read " + count + " chapters"`
  cannot be translated. Use a plural-aware formatter.
* Dates and numbers through `Intl`, with the locale from the account.
* Server-side messages: `AppError::public_message()` returns English today. Add
  a `message_key` to the variant and translate on the client, so a Spanish
  reader does not get an English error.

### 4.5 Retention and deletion, per table

Every migration states it; `verification.md` collects the summary. The rules that
recur:

* Private reading data cascades with the account.
* Moderation and audit records never cascade; they anonymise.
* Ledger entries are append-only and permanent.
* Content-addressed blobs are deleted only when unreferenced.
* A soft-deleted row keeps its `deleted_at` and its history, and every read path
  filters on it.

### 4.6 What to do when this plan is wrong

It will be, in places. When you find one:

1. Fix the code the way the **spec** says.
2. Fix this plan in the same commit.
3. If the spec itself is ambiguous, write an ADR with the options and the
   choice, and link it from the milestone.

A plan that is not corrected when it is wrong is worse than no plan, because the
next person trusts it.


