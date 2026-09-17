# Part 1 — Foundations: a repository that can carry the whole site

Checkpoint: none. An empty directory.

By the end of this part you have a workspace, a configuration system that cannot
lie to you, a migration runner that carries both database dialects, one error
envelope for the whole application, a health endpoint that means something
different from the other health endpoint, and a frontend that is compiled into
the binary. Everything after this part is easier because of the choices here.

---

## 1. Checkpoint

An empty directory and a Rust toolchain.

```bash
cargo --version     # 1.8x or newer
node --version      # for the frontend build only
sqlite3 --version   # optional, for looking at the database by hand
```

## 2. What will work by the end

```bash
cargo build
lorehaven doctor          # 19 checks; fails loudly on pending migrations
lorehaven migrate         # applies 0001_identity
lorehaven seed --development
lorehaven serve
curl localhost:8080/health/live      # {"status":"ok",...} — never touches the database
curl localhost:8080/health/ready     # database + migrations + storage
curl localhost:8080/api/v1/meta      # instance name, build, content policy
```

And `http://127.0.0.1:8080/` renders the Lorehaven shell, served out of the Rust
binary.

## 3. Concepts

- **Five crates, one direction of dependency.** `domain` ← `db` ← `app`, with
  `scrapers` beside them and `test-support` under the tests. Nothing in `domain`
  knows what a database or an HTTP request is.
- **Two dialects, one schema per dialect.** `migrations/sqlite/` and
  `migrations/postgres/` hold the same migration numbers with dialect-appropriate
  SQL. Both catalogues are compiled into the binary.
- **Configuration is resolved once.** Argument → environment → file → default, in
  one function, so `serve` and `doctor` can never disagree about what is running.
- **Two health endpoints with different jobs.** Liveness must never touch the
  database: if readiness failures kill a process, a database blip becomes an
  outage. Readiness must touch everything that has to work before serving.
- **One error envelope.** One `AppError` type in the domain names every failure
  and its stable code; one `IntoResponse` in the app renders it. Every client
  sees the same shape.
- **Production refuses unsafe development settings at startup**, and says which
  setting to change.

## 4. Commands

```bash
cargo new --lib crates/domain
cargo new --lib crates/db
cargo new --bin crates/app
cargo new --lib crates/scrapers
cargo new --lib crates/test-support

cd frontend && npm install && npm run build && cd ..
cargo build
cargo test
cargo run -- doctor
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `Cargo.toml` | the workspace and one shared dependency set |
| `crates/domain/src/ids.rs` | UUID newtypes — random, never sequential |
| `crates/domain/src/error.rs` | the error taxonomy and the stable code list |
| `crates/domain/src/policy.rs` | content eligibility and the age state machine |
| `crates/db/src/lib.rs` | the dual-dialect `Database` handle |
| `crates/db/src/migrate.rs` | the migration runner and its ledger |
| `crates/db/build.rs` | embeds both migration catalogues at build time |
| `migrations/sqlite/0001_identity.sql` | the identity schema, SQLite |
| `migrations/postgres/0001_identity.sql` | the identity schema, PostgreSQL |
| `crates/app/src/config.rs` | configuration resolution and validation |
| `crates/app/src/safety.rs` | the production startup checks |
| `crates/app/src/http.rs` | the error envelope and request correlation |
| `crates/app/src/server.rs` | the router and the middleware stack |
| `crates/app/src/assets.rs` | embedded assets with an SPA fallback |
| `crates/app/src/routes/health.rs` | liveness and readiness |
| `crates/app/src/routes/meta.rs` | instance metadata |
| `crates/app/src/seed.rs` | idempotent development fixtures |
| `crates/app/src/doctor.rs` | the health report |
| `crates/app/build.rs` | build identity, and a bundle placeholder for clean checkouts |
| `crates/test-support/src/lib.rs` | the test harness every later part uses |
| `frontend/` | the Svelte shell, tokens and components |

## 6. The code that matters

### A domain crate with no I/O

`crates/domain` depends on neither `sqlx` nor `axum`. That is the whole point:
the interesting rules — who may read a draft, what makes a work eligible for a
search index, how a loan window is computed — are testable with no database, no
network and no clock. When you are tempted to put a rule in a route handler
because the data is already loaded there, remember that the worker and the
importers need the same rule, and they have neither a `Request` nor a session.

### The error envelope, defined once

```rust
// crates/app/src/http.rs
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let error = self.0;
        let status =
            StatusCode::from_u16(error.status_code()).unwrap_or(StatusCode::INTERNAL_SERVER_ERROR);
        let code = error.code();

        // Faults are logged with their full chain; refusals are not noise.
        // `?error` rather than `%error`: the `Display` form of an internal
        // error is masked by design, so the diagnostic detail only appears in
        // the `Debug` rendering.
        if error.is_fault() {
            tracing::error!(error = ?error, code = code.as_str(), "request failed");
        } else {
            tracing::debug!(code = code.as_str(), "request rejected");
        }
        // { "error": { "code", "message", "field_errors", "request_id" } }
    }
}
```

Three decisions are visible in those twelve lines, and all three matter later:

1. **The status and code come from the domain.** The route handler returns
   `Err(AppError::…)` and never picks an HTTP status by hand.
2. **Faults and refusals are logged differently.** A validation refusal is not an
   incident. If you log both at `error`, your logs stop being read.
3. **Internal detail stays in the logs.** `Display` is masked; only `Debug`
   reaches the log line. A client never sees a database error message.

Because `ApiError` is a local newtype wrapper, you satisfy Rust's orphan rule
without dragging `axum` into the domain crate.

### Migrations are embedded, per dialect

Build script embeds both catalogues; the runner keeps a ledger table. Two
properties you will rely on for the rest of the project:

- `doctor` can compare "migrations compiled into this binary" against "migrations
  applied to this database" and tell you exactly which are pending.
- A fresh checkout gets a **placeholder bundle** rather than a build failure, so
  `cargo build` works before you have run the frontend build.

### Config resolution, in one place

```toml
[site]
[server]        # bind, port, max_body_bytes
[database]      # url, max_connections, acquire_timeout_secs
[storage]       # root
[security]
[logging]
[assets]
[imports]
[tts]
[dev]
```

Every section ends up in one `Config` struct with `deny_unknown_fields`. If you
get the key wrong, the process refuses to start and says which key. That is worth
more than it sounds: it turns a silent misconfiguration ("why is nothing rate
limited?") into a startup failure.

### Assets, and the fallback you must get right

The frontend bundle is compiled into the binary and served by
`crates/app/src/assets.rs`, with an SPA fallback: an unknown path returns
`index.html` so client-side routes work on a hard refresh.

Get the ordering wrong and you have a bug that is invisible until someone debugs
an API client: a fallback that answers `/api/...` turns a mistyped or removed
endpoint into `200 text/html`. The fallback must apply only to paths that are
*not* API paths. Verify it by hand, once, in this part:

```bash
curl -i localhost:8080/api/v1/does-not-exist    # must be a JSON 404, not HTML
```

## 7. Tests

`crates/app/tests/milestone_0.rs` proves the vertical slice over real HTTP
requests against a real SQLite file:

```rust
//! * the application starts with SQLite;
//! * the application starts with PostgreSQL;
//! * a frontend page loads from the Rust executable;
//! * `/health/live` checks process liveness;
//! * `/health/ready` checks essential dependencies;
//! * production startup rejects unsafe development configuration.
```

The harness pattern introduced here is used by every later part, so copy it
exactly:

```rust
fn scratch_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "lorehaven-it-{tag}-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create scratch dir");
    dir
}

async fn scratch_database(dir: &Path) -> Database {
    let url = format!("sqlite://{}/lorehaven.sqlite?mode=rwc", dir.display());
    Database::connect(&DatabaseConfig::new(url))
        .await
        .expect("connect to scratch database")
}
```

One scratch directory per test, named by process and thread — because tests run
in parallel in one process, and two tests sharing a database is a flake you will
spend a day on.

```bash
cargo test -p lorehaven-app --test milestone_0
```

## 8. Expected UI behaviour

`/` renders the shell with the site name. There is no content yet, and the page
says so rather than showing mock data. That is a rule for the whole project:

> No screen displays mock data. Pages that exist show real values; routes that
> are linked but unbuilt say so plainly.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `doctor` lists every pending migration | you have not run them | `lorehaven migrate` |
| `doctor` says production config is unsafe | development defaults with `environment = "production"` | read the named setting and fix it; do not silence the check |
| Frontend route 404s on hard refresh | the SPA fallback is missing or ordered after the 404 | see §6 — fallback last |
| `/api/v1/anything-unknown` returns HTML | the fallback is answering API paths | exclude the API prefix from the fallback |
| Tests pass alone, fail together | two tests share a database file | one scratch dir per test (see §7) |
| `cargo build` fails on a missing frontend bundle | the build script has no placeholder | emit an empty bundle and let `assets` report it |

## 10. Consequences

- **SQLite is the default, and it is a real choice.** A single file, no server,
  backups by `cp`. It is the honest default for a self-hosted instance; it is
  also a write-concurrency ceiling you should know about before you promise
  anyone a busy instance.
- **Two dialects cost you on every migration.** From this point on, every schema
  change is two files. Budget for it; do not let one dialect rot.
- **Production safety checks are not advisory.** The startup refusal is the only
  thing standing between a copy-pasted development config and a public instance
  with debug cookies.

## 11. Checkpoint

```bash
git tag v0.01-running-app
```

Verified by: `cargo test -p lorehaven-app --test milestone_0` (SQLite and
PostgreSQL), `cargo clippy --workspace --all-targets` clean, and by hand:
`doctor`, `migrate`, `seed`, `serve`, the three curls in §2.

Still owed at this point: no authentication, no content, no worker. The next
part is identity, and it is the part where the privacy rules start to matter.
