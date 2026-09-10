# Tutorial 00 — A running application

Checkpoint: `v0.01-running-app`

This chapter builds Milestone 0 of the plan: a repository that builds, a server
that starts on either database, migrations that can be inspected, a development
seed, health endpoints that mean different things, and a frontend page served
from the compiled binary.

Everything here is verified by a test as well as by hand — see
`docs/verification.md` for the results.

---

## 1. Starting checkpoint

Nothing. An empty directory.

## 2. What will work by the end

```bash
lorehaven doctor      # 17 checks; failing on pending migrations, with the fix named
lorehaven migrate     # applies 0001_identity
lorehaven seed --development
lorehaven serve
curl localhost:8080/health/live     # {"status":"ok", …}
curl localhost:8080/health/ready    # database + migrations + storage
curl localhost:8080/api/v1/meta     # instance name, build, content policy
```

…and `http://127.0.0.1:8080/` renders the Lorehaven shell — served out of the
Rust binary, with no Node process anywhere near production.

## 3. Concepts introduced

- **Layered configuration.** Argument → environment → file → default, resolved
  in one place so `serve` and `doctor` can never disagree about what is running.
- **Two health endpoints with different jobs.** Liveness must never touch the
  database; readiness must.
- **An embedded frontend.** The bundle is compiled into the binary, so a deploy
  is one file.
- **Per-dialect migrations with an embedded catalogue.** The schema travels with
  the executable, so the binary and the database cannot drift.
- **A domain crate with no I/O.** Rules that can be unit-tested without a
  database, an HTTP client, or a clock.
- **Startup safety as code.** Production refuses to start with development
  settings, and says which setting to change.

## 4. Commands

```bash
# 1. Workspace
cargo new --lib crates/domain
cargo new --lib crates/db
cargo new --bin crates/app

# 2. Frontend
cd frontend && npm install

# 3. Everything else
cargo build
cargo test
cargo run -- doctor
cargo run -- migrate
cargo run -- seed --development
cargo run -- serve
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `Cargo.toml` | workspace, one dependency set shared by every crate |
| `crates/domain/src/ids.rs` | UUID newtypes; random, never sequential |
| `crates/domain/src/error.rs` | the error taxonomy and the stable code list |
| `crates/domain/src/policy.rs` | content eligibility and the age state machine |
| `crates/db/src/lib.rs` | the dual-dialect `Database` handle |
| `crates/db/src/migrate.rs` | the migration runner and its ledger |
| `crates/db/src/identity.rs` | identity repositories, one statement per dialect |
| `crates/db/build.rs` | embeds both migration catalogues at build time |
| `migrations/sqlite/0001_identity.sql` | identity schema, SQLite |
| `migrations/postgres/0001_identity.sql` | identity schema, PostgreSQL |
| `crates/app/src/config.rs` | configuration resolution and validation |
| `crates/app/src/safety.rs` | production startup checks |
| `crates/app/src/server.rs` | router and middleware stack |
| `crates/app/src/http.rs` | the error envelope and request correlation |
| `crates/app/src/assets.rs` | embedded asset serving with an SPA fallback |
| `crates/app/src/routes/health.rs` | liveness and readiness |
| `crates/app/src/routes/meta.rs` | instance metadata |
| `crates/app/src/seed.rs` | idempotent development fixtures |
| `crates/app/src/doctor.rs` | the health report |
| `crates/app/build.rs` | build identity, and a placeholder bundle for clean checkouts |
| `frontend/` | the Svelte shell, tokens and components |

## 6. Explanation of important code

### The error envelope is defined once

Every failure in the application is an `AppError` from the domain crate. The
domain says what the error *is* (its stable code and HTTP status); the server
crate says how it *renders*:

```rust
// crates/app/src/http.rs
impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        let request_id = current_request_id()
            .map_or_else(|| "unavailable".to_owned(), |id| id.to_string());
        // { "error": { code, message, field_errors, request_id } }
    }
}
```

The split matters. `AppError` needs no web framework, so the domain can be
tested — and later reused by the worker and the plugin host — without one. And
because the wrapper is a local type, the orphan rule is satisfied without
polluting the domain with transport concerns.

### Request IDs are a task-local, not a parameter

The request id is installed around the whole request future:

```rust
// crates/app/src/server.rs
let response = with_request_id(request_id_text, async move {
    let started = Instant::now();
    let mut response = next.run(request).await;
    tracing::info!(status = response.status().as_u16(), latency_ms, "request completed");
    ...
})
```

so that `into_response` can stamp it into the envelope without every handler
threading it through by hand. A client-supplied `x-request-id` is sanitised
first: an id containing a newline would otherwise be a log-injection primitive.

### The migration catalogue is embedded

`crates/db/build.rs` scans `migrations/sqlite/` and `migrations/postgres/` and
generates an `include_str!` list. The runner then:

- creates its ledger,
- **verifies the checksum of every already-applied migration before applying
  anything** — so an edited migration fails loudly instead of silently
  diverging between instances,
- applies each pending migration inside its own transaction.

A test asserts that both dialects define the same set of migration ids, which is
the cheapest possible guard against the two schemas drifting apart.

### Readiness is a real check

```rust
checks.insert("database", match db.ping().await { … });
checks.insert("migrations", match migrate::pending(db).await { … });
checks.insert("storage", match check_storage(&config.storage.root).await { … });
```

The storage check writes and removes a probe file. That is the only way to catch
the failures that actually happen — a read-only mount, a full disk, the wrong
owner — rather than merely an absent directory.

### Production refuses to start

```rust
// crates/app/src/safety.rs
if !config.security.cookie_secure && production {
    findings.push(Finding::fatal(
        "cookie-secure",
        "session cookies would be sent over plain HTTP in production",
        "set [security] cookie_secure = true (and serve the site over HTTPS)",
    ));
}
```

`doctor` prints all findings with their severity; `serve` refuses on any fatal
one. Both read the same function, so the diagnosis and the enforcement cannot
disagree.

## 7. Tests

```text
cargo test                                  # 91 passed
cd frontend && node node_modules/vitest/vitest.mjs run   # 25 passed
```

What the Rust tests actually assert:

| File | Asserts |
|---|---|
| `crates/app/tests/milestone_0.rs` | migrations apply once and are idempotent; handles and emails are unique case-insensitively; seeding is idempotent and the stored password verifies; liveness reports the build; readiness fails on an unmigrated database and passes after; metadata is real; the shell is served, unknown routes fall back to it, and missing files 404; request ids are echoed and sanitised; security headers are present; production refuses to start |
| `crates/db/src/migrate.rs` | both dialects have a catalogue, with identical ids, sorted and unique; checksums change with SQL |
| `crates/db/src/lib.rs` | backend inference refuses unknown schemes; passwords never reach the logs; placeholder rewriting is correct |
| `crates/domain/src/policy.rs` | drafts are invisible; explicit content is refused to an unknown-age actor; restricted works need a session; a blocked actor is refused; a declared adult is not treated as verified |

## 8. Expected UI behaviour

- `/` shows the Lorehaven wordmark, the navigation, and three real panels: the
  instance's build and content policy from `/api/v1/meta`, the service health
  from `/health/ready`, and a short list of what is built.
- Clicking **Library** navigates without a page load and shows "Not built yet",
  naming Milestone 8.
- The appearance selector switches between Reading Room, After Hours, Clear Day
  and Match system; the choice survives a reload.
- Everything above works with the keyboard alone, and a visible focus ring
  follows you.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| `/health/ready` returns 503 with a `migrations` failure | The schema is behind the binary | `lorehaven migrate` |
| `serve` refuses to start, naming `cookie-secure` | `LOREHAVEN_ENV=production` with development security settings | Set `[security] cookie_secure = true`, use an HTTPS `base_url`, unset `[assets] dir`, and leave seeding off |
| The page says "frontend not built" | `cargo build` ran before `npm run build` | `cd frontend && npm run build`, then rebuild the binary |
| `Unknown configuration key` at startup | A typo in `lorehaven.toml` | The message names the key; keys are checked deliberately |
| `seed` refuses to run | Not in development, or `[dev] seed_enabled = false` | Pass `--development`, or fix the configuration |
| On an NFS checkout, `npm run build` fails with `Operation not permitted` | The share does not allow executing `node_modules/.bin` symlinks | `node node_modules/vite/bin/vite.js build` |
| `lorehaven migrate | head` panics with a broken pipe | Rust ignores `SIGPIPE` | Known wart; see `docs/verification.md` |

## 10. Commit / checkpoint

```bash
git tag v0.01-running-app
```

Next: `docs/tutorial/01-design-system.md` — tokens, components, focus
management, and the navigation shell.
