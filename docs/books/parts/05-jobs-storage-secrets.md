# Part 5 — Jobs, storage, secrets and the worker

Checkpoint: `v0.05-reader`

Most of what makes this site useful is slow: fetching a story from another site,
converting an ebook, generating narration, sending a webhook, building an export.
None of it belongs in a request. This part builds the queue, the blob store, the
secret store and the worker that turn slow work into a status you can poll.

## 1. Checkpoint

```bash
git checkout v0.05-reader
```

## 2. What will work by the end

```bash
curl -X POST localhost:8080/api/v1/jobs -d '{"kind":"export","payload":{...}}'
# 202 { "job": { "id": "...", "state": "queued" } }

curl localhost:8080/api/v1/jobs/$JOB          # queued → running (with progress) → succeeded
curl localhost:8080/api/v1/jobs/$JOB/result   # the artifact, when there is one
```

A worker process — the same binary, `lorehaven serve` runs it in-process — takes
the job, does the work, stores the result as a content-addressed blob and records
what happened. Killing the worker mid-job leaves the job resumable or
`transient_failed`, never `running` forever.

## 3. Concepts

- **A job is a row, not a message.** The queue is a table with a lease. No
  broker, nothing to lose when it restarts.
- **Leases, not locks.** A worker claims a job for a bounded time and renews it.
  A crashed worker's job becomes claimable again instead of stuck.
- **Content-addressed storage.** A blob is named by the hash of its bytes. The
  same cover image uploaded twice costs one row and one file.
- **Secrets are encrypted at rest, and never read back into a response.** The
  key lives in a file outside the database.
- **Faults and refusals are classified.** A failure that will never succeed
  (`Unsupported source`, `no OCR program installed`) is fatal; a network blip is
  transient. Getting this wrong is how a queue spends the night retrying
  something that cannot work.
- **Maintenance is a job too.** Expiring leases, dropping old terminal jobs and
  sweeping expired state runs on the same timer.

## 4. Commands

```bash
lorehaven migrate        # applies 0005_jobs_and_storage
cargo test -p lorehaven-app --test milestone_5
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0005_jobs_and_storage.sql` | jobs, blobs, secrets, outbox |
| `crates/domain/src/jobs.rs` | job kinds, states, retry policy, backoff |
| `crates/db/src/jobs.rs` | claim, lease, renew, complete, fail, sweep |
| `crates/db/src/storage.rs` | blob metadata and reference counting |
| `crates/db/src/secrets.rs` | encrypted secret rows |
| `crates/db/src/outbox.rs` | events written in the same transaction as the change |
| `crates/app/src/secrets.rs` | the key file and the encrypt/decrypt boundary |
| `crates/app/src/worker.rs` | the loop, the handlers, `maintenance_pass` |
| `crates/app/src/routes/jobs.rs` | submit, poll, result, cancel |
| `crates/app/tests/milestone_5.rs` | acceptance tests |
| `frontend/src/routes/Jobs.svelte` | the reader's own jobs |
| `frontend/src/routes/AdminJobs.svelte` | the operator's view of the queue |

## 6. The code that matters

### The job table shape

```sql
jobs (
  id, kind, payload_json,
  owner_account_id,            -- whose job it is (for visibility)
  state,                       -- queued | running | succeeded | failed | cancelled
  attempts, max_attempts,
  lease_owner, lease_expires_at,
  progress_current, progress_total,
  error_code, error_message,   -- the stable code, and a message safe to show
  dedupe_key,                  -- submit the same thing twice, get one job
  created_at, started_at, finished_at
)
```

Three columns earn their place immediately:

- `dedupe_key`: a double-clicked "Export" must not produce two exports. Make it a
  unique index over (kind, dedupe_key) while the job is not terminal.
- `progress_current/total`: a long job with no progress is indistinguishable from
  a hung one, and users poll.
- `error_code` alongside `error_message`: the code is for the client's logic, the
  message is for the human, and the message must not contain internal detail.

### Claiming work without a broker

```sql
-- one statement, in one transaction
UPDATE jobs SET state='running', lease_owner=?, lease_expires_at=?
WHERE id = (SELECT id FROM jobs
            WHERE state='queued'
               OR (state='running' AND lease_expires_at < now())
            ORDER BY created_at LIMIT 1)
```

On PostgreSQL add `FOR UPDATE SKIP LOCKED` to the sub-select so two workers do
not fight. On SQLite the write lock already serialises this. The renewal is the
same statement against your own lease: if the row is no longer yours, stop
working — someone else has your job.

### The worker loop, and why handlers are separate

```rust
// crates/app/src/worker.rs
loop {
    maintenance_pass(&db).await?;      // leases, retention, expiry sweeps
    if let Some(job) = jobs::claim(&db, &worker_id).await? {
        let outcome = handle_job(&state, &job).await;   // one match arm per kind
        jobs::finish(&db, &job, outcome).await?;
    } else {
        sleep(poll_interval).await;     // and a jittered backoff when idle
    }
}
```

`handle_job` is one `match` over `JobKind`, and each arm is a module: imports,
exports, derivatives, narration, webhooks. That keeps the worker free of domain
logic — the handlers reuse exactly the same domain functions the routes use.

### Storage: hash first, store second

```text
store_blob(bytes) -> BlobHandle { checksum, size, media_type }
```

- The checksum is computed before writing; the path is derived from it.
- Storing the same bytes twice is idempotent and returns the same handle.
- Reads go through one function that enforces the caller's permission; blobs are
  never served by path from the filesystem.
- **Never serve a blob by guessing.** If a media row is missing, 404 — an
  unauthenticated path that walks the storage root is a directory-traversal bug
  waiting to be found.

### Secrets

```bash
# first run, development
WARN no secret key was configured; generated one for development.
     Back it up or set LOREHAVEN_SECRET_KEY before this instance holds anything you care about.
     path=./data/secret.key
```

The rule for the rest of the project: a stored secret is decrypted at the moment
of use, in one module, and never returned by any API — not even an admin one. When
you add source credentials (Part 7) and webhook signing keys (Part 12), they are
columns in the same encrypted shape, and the "never returned" rule is the same
rule.

### `maintenance_pass` and its report

```rust
struct PassReport {
    leases_reclaimed: usize,
    terminal_jobs_pruned: usize,
    loans_expired: usize,        // Part 13
}
```

Two habits worth copying: the pass returns a report you can log (so an operator
can see the queue is healthy without querying it), and it is **idempotent** — run
it twice in a row and the second run changes nothing. Maintenance code that is
only safe to run once is a trap for whoever runs it manually.

## 7. Tests

`milestone_5.rs` asserts:

- a job is claimable exactly once; a second claim returns nothing;
- a job whose lease expires becomes claimable again, and the first worker's later
  completion does not overwrite the second's result;
- the same `dedupe_key` twice while non-terminal yields one job;
- a fatal failure is not retried; a transient failure is, with backoff, and stops
  at `max_attempts`;
- storing identical bytes twice yields one blob path;
- a secret is never present in any API response, including an admin listing;
- `maintenance_pass` twice in a row produces the same state and a report.

## 8. Expected UI behaviour

- Submitting an export gives you a job you can watch, with progress that moves.
- A finished job offers its result, and the result is still there tomorrow.
- A cancelled job stops, and says it was cancelled rather than failed.
- The admin job view shows the queue's real depth, not zero.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Jobs stuck at `running` | no lease expiry, or the worker died without releasing | lease + reclaim on the next pass |
| One job runs twice, producing two artifacts | the claim and the state change were separate statements | claim and transition in one statement |
| Disk fills up | blobs are written and never referenced | reference counting, plus a sweep that reports what it would remove |
| A retry storm hits a dead host | transient classification is too generous | classify on the error, not on the fact that it failed |
| Secrets appear in logs | the error message carried the payload | log the code and the job id; never log the payload |

## 10. Consequences

- **Anything an operator can see, a breach can see.** The admin job view gets the
  payload *shape*, not the payload, unless the payload is already public.
- **The blob store is the backup that matters.** The database tells you what
  should exist; the files are the content. A backup that takes only the database
  restores a site where every cover image is a 404.
- **Dedupe keys are a privacy feature.** Two readers importing the same URL
  should not be able to tell that someone else did it first.

## 11. Checkpoint

```bash
git tag v0.06-jobs
```

Verified by `milestone_5.rs` on both dialects, plus a manual run: submit an
export, kill the process mid-job, restart, watch the lease get reclaimed and the
job finish.
