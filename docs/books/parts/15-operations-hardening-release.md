# Part 15 — Operations, hardening and release

Checkpoint: `v0.19-integrations-ai-search`

The site is feature-complete. This part is about the difference between something
that works on your machine and something you are willing to leave running
unattended.

## 1. Checkpoint

```bash
git checkout v0.19-integrations-ai-search
```

## 2. What will work by the end

```bash
lorehaven doctor                    # 19 checks, each with a remedy
lorehaven serve                     # one binary: API, assets, worker
curl localhost:8080/health/live
curl localhost:8080/health/ready

# Operator surfaces
curl localhost:8080/api/v1/admin/queues
curl localhost:8080/api/v1/admin/stats
curl -X POST localhost:8080/api/v1/admin/privacy/export -d '{"account":"…"}'
```

And: a backup you have **restored**, a queue you have **drained**, and a release
tag with its verification notes attached.

## 3. Concepts

- **Diagnostics are a feature, not a script.** `doctor` is the same code path the
  server uses, so it cannot report something different from what is running.
- **Every operator tool is audited**, because an unaudited tool is a privilege
  escalation with a friendly UI.
- **Statistics are aggregates with a floor.** Small numbers are individual
  people (Part 13's dashboard rule, applied to the instance).
- **Abuse defence is mostly accounting**: rate limits, quotas, and the ability to
  turn a feature off without a deploy.
- **A backup is only a backup once you have restored it.**
- **Release notes are traceability.** Every claim about what works points at a
  test or a command whose output you have seen.

## 4. Commands

```bash
lorehaven migrate        # applies 0021_admin and everything before it
cargo fmt --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace
cd frontend && npm run check && npm test && npm run build
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0021_admin.sql` | audit log, feature flags, operator actions |
| `crates/app/src/doctor.rs` | the checks, each with a remedy |
| `crates/app/src/privacy.rs` | account data export and deletion |
| `crates/app/src/logging.rs` | structured logs, request correlation, redaction |
| `crates/app/src/limiter.rs` | the rate-limit table and its keys |
| `crates/app/src/safety.rs` | production startup refusals |
| `crates/app/src/version.rs` | the build identity reported everywhere |
| `crates/app/src/routes/admin.rs` | the operator doors, all audited |
| `docs/verification.md` | the evidence log |
| `docs/requirements.csv` | the requirement ledger, with honest statuses |

## 6. The code that matters

### `doctor`: the single source of truth about the environment

Every check reports one of four states, and a failing check names its remedy:

```text
[ok  ] database         sqlite reachable at …/lorehaven.sqlite
[FAIL] migrations       33 migration(s) pending: 0001_identity, …
                        fix: run `lorehaven migrate`
[warn] narration        engine "piper" is not usable: the `piper` binary is not on PATH
                        fix: narration editions stay unavailable until this is fixed
```

Three properties make it useful rather than decorative:

- **It shares code with the server.** The TTS engine in `doctor` is built by the
  same builder the worker uses, so they cannot disagree about what is available.
- **It distinguishes a failure from a degraded feature.** A missing `piper` is a
  warning: everything else works. A pending migration is a failure: the site is
  not the site you built.
- **It is honest about scope.** Nineteen checks is not "the instance is healthy";
  it is "these nineteen things are as they should be".

### Every operator action leaves a row

```sql
admin_audit (id, actor_account_id, action, subject, reason, at, request_id)
```

The door that performs the action writes the row **in the same transaction**.
Not "also": a privileged action without an audit row is indistinguishable from an
intrusion, by you, later.

And the rules that make moderation survivable (Part 11) apply here with more
force: an operator can see metadata about a private object to act on a report,
and cannot browse it for curiosity. If your admin UI has a "view all messages"
page, that page is the vulnerability.

### Statistics with a floor

```text
instance stats → aggregates, and any count below the floor is a band
```

The same rule as the creator dashboard, for the same reason: "1 active reader in
your town" is a person. Aggregate, band, and never expose a per-pseud breakdown
to an operator who has not been given that role explicitly.

### Abuse defence you can operate

| Control | Keyed by | Why |
|---|---|---|
| sign-in attempts | IP + account | credential stuffing |
| registration | IP | bulk accounts |
| comment posting | pseud + work | flooding |
| API tokens | token | one bad bot, one limit |
| import submissions | account | resource exhaustion |
| webhook deliveries | subscription | outbound abuse |

Plus **feature flags in the database**, so a feature under attack can be turned
off without a deploy: `flags (name, enabled, updated_by, updated_at)`. The flag
check is server-side and the UI hides what is off.

### Privacy: export and deletion

- **Export**: everything the account holds, in a machine-readable archive,
  delivered as a job (Part 5). The archive must not contain other people's data —
  a comment thread export contains the reader's comments, not everyone's.
- **Deletion**: a real deletion with a documented grace period. What cannot be
  deleted (an audit row, a ledger entry, a moderation case) is anonymised and
  the reason is recorded. "Deleted" that leaves the email address behind is a
  promise you will be held to.
- Both operations are jobs, both are audited, and both appear in the verification
  log with the commands that prove them.

### Migrations in production

```bash
lorehaven migrate --dry-run     # what would be applied
lorehaven migrate               # applies, in order, in a transaction per migration
```

Rules worth writing into your release process:

- **A migration is forward-only in the deployed binary.** Two versions of the
  binary may run against one schema during a rollout: make additive changes
  first, deploy, then remove the old shape in a later release.
- **Every migration runs on both dialects in CI**, or the PostgreSQL instance
  rots quietly until someone upgrades.
- **Backup before migrating**, and for a destructive change, verify the backup by
  restoring it into a scratch database.

### The release checklist

```text
[ ] cargo fmt --check, clippy -D warnings, workspace tests green
[ ] frontend check + tests + build green
[ ] doctor run on the release binary against a restored production backup
[ ] every requirement in the ledger has a status and evidence
[ ] verification log updated with literal output, not summaries
[ ] upgrade notes written: what changed, what to run, what to expect
[ ] tag cut from the commit that was tested (not from a later one)
```

The last line matters more than it looks: a tag that does not correspond to the
tested commit makes every claim in your release notes unverifiable.

## 7. Tests

- `cargo test --workspace` — the aggregate gate, run once at the end of the
  change, not after every file.
- `doctor` against a fresh database, a migrated database, a database with a
  missing storage root, and a database with a pending migration — four states,
  four honest reports.
- The privacy export of an account with one comment in a thread with three
  authors contains exactly one comment.
- Deletion removes the email address, and the audit row for the deletion remains.
- The rate limiter refuses at the configured rate and recovers after the window.

## 8. Expected UI behaviour

- The operator view shows queue depth, job failure rate and storage growth, all
  from real data.
- A disabled feature disappears from the UI rather than erroring.
- An account deletion asks for confirmation, states the grace period, and then
  does what it said.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `doctor` says production config is unsafe | development defaults in production | fix the named setting, do not silence the check |
| A migration works on SQLite, fails on PostgreSQL | dialect-specific SQL (`?::uuid`, `INTEGER` booleans, `RETURNING`) | run both dialects in CI, always |
| Restore "works" but images 404 | database restored without the blob store | back up both, and record the pairing |
| A queue grows without bound | a job kind that always fails transiently | classify fatal failures as fatal |
| Release notes cannot be reproduced | the tag is not the tested commit | tag what you tested |

## 10. Consequences

- **You are now responsible for what your instance does.** The abuse controls,
  the privacy tools and the audit trail are the parts that answer for it.
- **Every operator tool you build is a future breach's best tool.** Audit,
  limit, and prefer read-only.
- **Backups have a privacy dimension too.** An old backup contains data a user
  deleted. Decide your retention, and say so.

## 11. Checkpoint

```bash
git tag v1.0-release
```

Verified by: the full workspace suite, the frontend suite, `doctor` in four
states, a restore from backup into a clean directory, and the release checklist
walked in order with its output pasted into `docs/verification.md`.
