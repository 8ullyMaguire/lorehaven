# Milestone 5 — Jobs, storage, cache boundaries and secret management

Spec §10. Tag to leave behind: `v0.06-jobs`.

## Goal, stated as a journey

A signed-in writer asks the instance to do something that takes longer than a
request should. The server answers `202` with a job id instead of doing the work
inline; the page watches the job's progress and the job finishes. Then a second
job is cancelled mid-flight and the worker stops at the next checkpoint, leaving
the checkpoint it stopped at rather than running to completion.

Two things make this the milestone that everything after it rests on:

* **Importing is a background job** (M6). Nothing in M6 can be written honestly
  before there is a job model with leases and retries: an import "for now,
  inline in the request" times out on a 300-chapter work, and the retry does it
  all again from scratch.
* **Every export is a job that writes a file to storage** (M7), so
  content-addressed storage lands here too.

## What existed to build on

* `migrations/{sqlite,postgres}/0003_works.sql` — the `outbox_events` table M3
  has been writing to since it shipped, and which nothing had ever read.
  `docs/verification.md` listed that as an open risk.
* `crates/db/src/migrate.rs` — the embedded migration catalogue and its
  dialect-drift test. Any new migration must appear in both dialects with the
  same id.
* `crates/app/src/cli.rs` — `serve` already had `--no-migrate`, and the
  `Command` enum is where a new subcommand goes.
* `crates/app/src/auth.rs` — `RequireSession` and `RequirePseud`, which the job
  routes reuse rather than inventing a second session check.
* `crates/app/src/state.rs` — `AppState` carries the config and the database to
  the worker the same way it carries them to a request.

## The work, in order

### Task 1 — Migration 0005: the queue, the blob store and the secrets

Tables: `jobs`, `job_attempts`, `content_blobs`, `content_references`,
`encryption_keys`, `secrets`.

Retention comments written into the file, because each of them is a decision
somebody will otherwise "tidy up" wrongly:

* `jobs` and `job_attempts` are diagnostics, not history: a maintenance job
  deletes terminal rows after 30 days. `requested_by` survives the account's
  deletion as NULL so an operator can still see that *a* job ran.
* `content_blobs` are **never** deleted by cascade. A blob goes only when
  `content_references` has no row for its checksum, and that check is the only
  thing between an unused blob and data loss.
* `content_references` cascade with their owner (`work`, `chapter_revision`,
  `export`, `library_item`).
* `secrets` cascade with their owner; the ciphertext goes, and the key is never
  in the database at all.

The plan document listed `job_leases` and `source_revision_cache_entries` as
tables for this migration. They are not tables here: a lease is three columns on
`jobs` (owner, expiry, and the heartbeat that moves expiry), and the revision
cache belongs with the import adapters that populate it in M6. See
*Corrections* below.

### Task 2 — Domain: the state machine and the retry policy

`crates/domain/src/jobs.rs`: `JobState`, `JobKind`, `RetryPolicy`,
`next_attempt_at` (pure, `now` passed in), `can_cancel`, `may_retry`.

The jitter is a hash of the job id rather than a random number, so a retry's
scheduled time is reproducible in a test and a job does not re-roll its own
backoff every time it is read.

### Task 3 — Repository: `crates/db/src/jobs.rs`

`enqueue`, `claim_next`, `heartbeat`, `complete`, `fail`, `cancel`,
`requeue_expired_leases`, `progress`, `is_cancelled`, `find`, `jobs_for`,
`all_jobs`, `counts_by_state`, `attempts_for`, `purge_terminal_jobs`, `requeue`.

**The claim is one statement.** On SQLite
`UPDATE jobs SET … WHERE id = (SELECT id FROM jobs WHERE state = 'queued' AND
available_at <= ? ORDER BY priority DESC, available_at ASC LIMIT 1)` inside a
transaction; on PostgreSQL the same with `FOR UPDATE SKIP LOCKED` in the
sub-select and no transaction around it. Two workers racing must not both get
the same row, and a select-then-update cannot promise that.

`fail` reads the attempt count **and the row's `max_attempts`** under the lease,
then retries with the smaller of that and the worker's policy. Two budgets meet
there and the smaller wins: a worker configured to try less often must not
overrule the request, and a request for one attempt must not become five
because the worker's policy is generous.

### Task 4 — Storage: `crates/db/src/storage.rs`

`put`, `get`, `stat`, `reference`, `unreference`, `delete_if_unreferenced`,
`unreferenced`, `usage`. SHA-256 is the identity; files land at
`objects/<first two hex>/<checksum>`. `put` writes a temporary file in the same
directory and renames it into place, so a crash never leaves a half-written blob
under a name that claims to be complete, and re-putting the same bytes does not
touch `last_referenced_at` in a way that resurrects a blob something else is
collecting.

### Task 5 — Secrets: `crates/app/src/secrets.rs`

XChaCha20-Poly1305, a random nonce per record, the `key_id` on the row so a
rotation can re-encrypt lazily, and the associated data set to
`owner_type\u{1f}owner_id\u{1f}name` so a ciphertext moved to another row fails to
open instead of silently decrypting.

`Secret`'s `Debug` prints `<secret>` and nothing else, so a stray `?secret` in a
log line cannot leak one. `SecretKey::parse` accepts hex or base64 and takes
whichever reading is a 32-byte key, because a 64-character hex key is *also*
syntactically valid base64 — as 48 bytes — and a decoder that ran first would
otherwise reject a key an operator can legitimately write.

### Task 6 — The worker: `crates/app/src/worker.rs`

`lorehaven worker [--once]`, and `lorehaven serve --with-worker`. Loop: claim →
check for cancellation → do a unit of work → record progress → repeat → complete
or fail. On `SIGTERM`/`SIGINT` the worker stops claiming, finishes the unit in
flight, releases its lease and exits.

The outbox drain lives here and reads what M3 has been writing since it shipped.
An event is deleted only after its handler returns success; a topic nothing
handles yet is left in place, not marked delivered; a failing handler records
`last_error` and pushes the event out of the way for a while.

### Task 7 — Routes and pages

```text
GET    /jobs?cursor=…                the caller's own jobs, envelope
POST   /jobs/:id/cancel              the caller's own job
POST   /jobs                         development only, the diagnostic probe
GET    /admin/jobs?state=…&cursor=…  operators only
POST   /admin/jobs/:id/retry         operators only
```

Pages: `/jobs` and `/admin/jobs`.

`/admin` is gated on `config.administration.operator_account_id`, and there is
still no staff model. A non-operator gets **404**, not 403: "forbidden" would
confirm that the surface exists and that they are not on it. M13 replaces the
setting with a trust level, and the comment in `config.rs` says so.

## Acceptance for the milestone, restated as tests

| Requirement | Test |
|---|---|
| Two workers cannot claim one job | `a_claimed_job_is_not_claimed_twice` |
| A dead worker's lease comes back | `a_lease_that_expires_is_requeued` |
| Cancel stops at the next checkpoint | `a_cancelled_job_stops_at_the_next_checkpoint` |
| A retry waits for its backoff | `a_retry_uses_the_backoff` |
| A replayed key enqueues one job | `replaying_one_idempotency_key_enqueues_one_job` |
| Identical bytes are one blob | `the_same_bytes_stored_twice_share_one_blob` |
| One reference going keeps the blob | `deleting_one_reference_keeps_the_blob` |
| The last reference takes it | `deleting_the_last_reference_removes_the_blob` |
| A secret is not in the logs | `a_secret_is_not_in_the_logs` |
| A moved ciphertext does not open | `a_ciphertext_moved_to_another_row_does_not_open` |

Plus, because the journey above is the milestone's own statement of done:
`a_request_that_starts_a_job_gets_a_202_and_an_id`,
`an_operator_can_retry_a_failed_job`,
`the_admin_surface_is_gated_on_the_operator_account`,
`a_page_of_jobs_carries_a_cursor_that_resumes_it`,
`the_job_list_shows_only_the_callers_own_jobs`,
`one_pass_runs_one_job_and_says_so`,
`an_unknown_maintenance_task_is_a_fatal_failure`,
`the_sweep_deletes_only_old_terminal_jobs`,
`a_job_with_no_handler_fails_loudly`,
`an_outbox_event_is_deleted_only_after_its_handler_succeeds`,
`a_failing_outbox_handler_retries_with_a_reason`,
`job_progress_and_errors_reach_the_owner`,
`the_self_service_enqueue_refuses_anything_but_the_probe`,
`the_worker_can_be_pointed_at_a_queue_that_is_already_waiting`.

## Corrections made after this plan was written

Kept here rather than silently edited:

1. **`job_leases` is not a table.** The lease is `lease_owner` and
   `lease_expires_at` on `jobs`, renewed by `heartbeat`. A separate table would
   have to be kept in step with the row it leases, and the two would disagree
   the first time a worker died between the two writes.
2. **`source_revision_cache_entries` is not M5's.** The revision cache caches
   what an import adapter fetched; nothing populates it until M6 exists, so a
   table here would be schema with no writer and no reader. M6's migration
   carries it, next to the adapter that fills it.
3. **Quota enforcement is not M5's.** `BlobStore::usage` reports what is stored;
   a quota needs a limit and an account to hang it on, which arrive with M7's
   export limits and M17's storage view.
4. **`POST /jobs` exists, and only in development.** The route list in the plan
   had no way to start a job without M6. Rather than leave the journey
   undrivable, the milestone ships a self-service route that accepts the
   diagnostic `probe` maintenance job and nothing else, gated on
   `Environment::Development` with a 404 elsewhere. M6 replaces it with the
   endpoints whose payloads are the point.
5. **A transient-failure switch on the probe.** `{"fail": "…"}` makes the probe
   report a transient error, so the retry path is testable and an operator can
   rehearse it. Every other way a job can fail here is fatal.

## Pitfalls specific to this milestone

1. **Do not hold a database transaction across the network.** Claim in one
   transaction, release it, do the I/O, then record the outcome.
2. **A worker that mutates state must go through the same repository functions
   the web path uses.** The worker is a second entry point into every table this
   project has, and an invariant enforced in a route handler only holds in the
   route handler.
3. **A job kind with no handler must fail loudly.** A job that quietly
   "succeeded" without doing its work is the one outcome nobody notices until
   the data is wrong.
4. **`delete_if_unreferenced` is the only safe deletion.** A "clean up old
   blobs" job that deletes by age will delete a blob a reader is streaming.
5. **Do not add an `is_admin` column.** `operator_account_id` is a setting, and
   a setting is replaced in M13 without a migration over live data.
6. **The cursor is not decoration.** A list that ignores a malformed cursor
   restarts at page one and looks like it worked.
