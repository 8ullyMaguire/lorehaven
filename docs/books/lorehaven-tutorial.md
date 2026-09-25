---
title: "Building Lorehaven"
subtitle: "A complete implementation tutorial: from an empty directory to a running fanfiction platform"
author: "Lorehaven contributors"
lang: en
rights: "Internal technical documentation"
---

# Building Lorehaven

A complete implementation tutorial: from an empty directory to a running
fanfiction platform.

Lorehaven is a self-hosted home for fanfiction: read without an account, write
without a publishing queue, import the library you already have, and take it
offline when you want it. It is built as one Rust binary — API, frontend, worker
and schema — with SQLite and PostgreSQL supported from the same code.

This book is for a developer who already knows the stack (Rust, Axum, SQLx,
Svelte, TypeScript) and wants to build the application. It teaches the order to
build in, the rules that must not be broken, the mistakes that are waiting, and
how to prove each part works.

## What you will build

| Part | What it covers | Checkpoint |
|---|---|---|
| 1 | Foundations: workspace, config, migrations, errors, assets | `v0.01-running-app` |
| 2 | Accounts, pseuds, sessions, privacy, age policy | `v0.03-identity` |
| 3 | Works, chapters, revisions, publishing | `v0.04-publishing` |
| 4 | The reader: ratings, reviews, notes, history, goals | `v0.05-reader` |
| 5 | Jobs, blob storage, secrets, the worker | `v0.06-jobs` |
| 6 | The positivity filter and feedback delivery | `v0.08-positivity` |
| 7 | Imports: fetching, adapters, sanitising, shelf CSVs | `v0.07-importing` |
| 8 | Library, shelves, taxonomy, the query language, search | `v0.11-search` |
| 9 | Discovery, exports, offline reading | `v0.12-discovery` |
| 10 | Comments, forums, groups, messaging, events | `v0.13-community` |
| 11 | Trust, governance, credits, fair queues | `v0.15-governance` |
| 12 | Marketplace, extensions, webhooks, themes | `v0.17-marketplace` |
| 13 | The generalized media platform: editions, derivatives, lending, narration | `v0.22-media` |
| 14 | Integrations: translation, public API, feeds, push, federation | `v0.19-integrations` |
| 15 | Operations, hardening and release | `v1.0-release` |
| Appendix | The patterns used in every part | — |

## The three rules this book will not let you break

1. **Completion means working behavior.** A feature is done when a real request
   produces a real result that a test asserts.
2. **404 over 401 for private objects.** Never confirm the existence of
   something the caller may not know about.
3. **Build the rule with the feature, and prove it with a test.** The privacy,
   age and visibility rules are not a later pass.

Everything else in the book follows from those three.

## How the reference implementation is described

Every file path, module, migration and command named in this book exists in the
reference implementation, and each part ends with the tests that prove it. Where
the reference implementation is incomplete, the part says so rather than
describing a feature that does not work — an honest gap is worth more than a
plausible paragraph, and it is the only thing that lets the next developer pick
up where you stopped.

# How to use this book

You are going to build Lorehaven: a self-hosted home for fanfiction where a
reader can read without an account, a writer can publish without a queue, and
either of them can take their library offline when they want it.

This book assumes you already know the stack — Rust, Axum, SQLx, SQLite and
PostgreSQL, Svelte, TypeScript, HTTP, and how to write a test. It does not teach
you those. It teaches you **this** application: its rules, its shape, the order
to build it in, and the mistakes that are waiting for you.

## What you are building, in one paragraph

One Rust binary. It serves the API, serves the compiled frontend, runs a worker
that does slow work (imports, exports, OCR, transcoding, narration), and carries
its own database schema with it. There is no Node process in production and no
separate frontend deployment. Two database dialects are supported from the same
code: SQLite for a person on a laptop, PostgreSQL for an instance with more than
one reader.

## The rule this book follows

The specification this project was built from has one rule that shapes
everything else:

> Completion means working behavior. A feature is done when a real request
> produces a real result that a test asserts. Not when the code compiles, not
> when the route exists, not when the happy path works by hand.

So every part of this book ends with tests that exercise the real router against
a real database, and with an honest statement of what is verified and what is
not. If you follow along, you will never be in a position where you have to
guess whether something works.

## How the parts are organised

Each part is one vertical slice: database, domain rules, HTTP doors, frontend,
tests. You can finish a part and have a working thing. Parts build on each other
in order — the checkpoint tags at the end of each part name the commit you
should be at before starting the next.

| Part | Builds | Checkpoint after |
|---|---|---|
| 1 | Foundations: workspace, config, migrations, errors, assets | `v0.01-running-app` |
| 2 | Accounts, pseuds, sessions, privacy, age policy | `v0.03-identity` |
| 3 | Works, chapters, revisions, publishing | `v0.04-publishing` |
| 4 | Reader, ratings, reviews, notes, history, goals | `v0.05-reader` |
| 5 | Jobs, blob storage, secrets, the worker | `v0.06-jobs` |
| 6 | The positivity filter and feedback delivery | `v0.08-positivity` |
| 7 | Imports: fetching, adapters, sanitising, shelf CSVs | `v0.07-importing` |
| 8 | Library, shelves, saved views, taxonomy, search | `v0.11-search` |
| 9 | Discovery, exports, offline reading | `v0.12-discovery` |
| 10 | Comments, forums, groups, messaging, events | `v0.13-community` |
| 11 | Trust, governance, credits, fair queues | `v0.15-governance` |
| 12 | Marketplace, extensions, webhooks | `v0.17-marketplace` |
| 13 | The generalized media platform: editions, derivatives, lending, narration | `v0.22-media` |
| 14 | Operations, hardening, release | `v1.0-release` |
| Appendix | The patterns you will use in every part | — |

## Every part has the same shape

1. **Checkpoint** — where you should be starting from.
2. **What will work by the end** — the commands you will be able to run.
3. **Concepts** — the ideas this part is actually teaching you.
4. **Commands** — what to type.
5. **Exact file changes** — what appears on disk.
6. **The code that matters** — the parts worth reading carefully, and why.
7. **Tests** — what is asserted and what that proves.
8. **Expected UI behaviour** — what you should see in a browser.
9. **Troubleshooting** — the failures you are most likely to hit.
10. **Consequences** — the privacy, safety or operational cost of what you built.
11. **Checkpoint** — the tag to leave behind, and what is still owed.

## Two habits that will save you

**Write the door, the rule and the test together.** The most common way a
project like this goes wrong is that the route gets written, the happy path
works, and the rule it was supposed to enforce (who may read this? what happens
to a private draft?) is discovered later — by someone else, in production.

**Prefer 404 to 401 when the object is private.** If a caller asks for something
they are not allowed to know exists, do not tell them it exists and refuse them.
Tell them it does not exist. This is a project-wide rule, and Part 1 shows you
where it lives in the code.

## A note on the ordering of parts 6 and 7

You may notice the parts do not follow the specification's milestone numbering
exactly: the positivity filter (Part 6) is built before the importers (Part 7)
even though importing is milestone 6 and the filter is milestone 7. The reason is
that imported text has to pass through the filter, so the filter has to exist
first. Every project has a couple of dependencies like this. Find yours early,
and write them down.

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

# Part 2 — Identity: accounts, pseuds, privacy and age

Checkpoint: `v0.01-running-app`

This is the part where the project stops being a web server and starts being a
site. It is also the part where you must be most careful, because every later
part inherits whatever you decide here about who may know what.

## 1. Checkpoint

```bash
git checkout v0.01-running-app
```

## 2. What will work by the end

```bash
curl -X POST localhost:8080/api/v1/auth/register \
  -H 'content-type: application/json' \
  -d '{"email":"ada@example.com","password":"…","handle":"ada"}'
# 201, a session cookie, and one pseud

curl localhost:8080/api/v1/me                    # the account and its pseuds
curl -X POST localhost:8080/api/v1/pseuds -d '{"handle":"ada-writes"}'   # a second pseud
curl localhost:8080/api/v1/me/dashboard          # later parts fill this in
```

And in a browser: register, sign in, switch pseud, set your content preferences,
and see a public pseud profile.

## 3. Concepts

- **Account and pseud are different things.** The account owns the login, the
  email, the billing relationship and the security history. Everything a reader
  sees — works, comments, shelves, ratings — belongs to a *pseud*.
- **Three extractors, three meanings.** `RequireSession` (signed in),
  `RequirePseud` (signed in *and acting as a pseud*), `MaybeSession` (may be
  anonymous). Choosing one is a design decision, not a convenience.
- **404 over 401 for private objects.** A caller who may not know an object
  exists gets "not found". A caller who may know it exists but may not act on it
  gets a refusal.
- **The age state is an input to policy, not a UI flag.** Content eligibility is
  computed from the age state on every read, in the domain crate.
- **Privacy classifications travel with the data**, not with the endpoint: the
  same row can be public, unlisted, followers-only or private.

## 4. Commands

```bash
lorehaven migrate                    # applies 0001_identity, 0002_sessions_and_settings
cargo test -p lorehaven-app --test milestone_2
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0001_identity.sql` | accounts, pseuds, blocks |
| `migrations/postgres/0001_identity.sql` | the same, PostgreSQL |
| `migrations/sqlite/0002_sessions_and_settings.sql` | sessions, settings, content preferences |
| `crates/domain/src/ids.rs` | `AccountId`, `PseudId`, `SessionId` |
| `crates/domain/src/policy.rs` | age states, content eligibility |
| `crates/domain/src/blocking.rs` | blocks and scoped mutes |
| `crates/domain/src/error.rs` | `AUTH_REQUIRED`, `FORBIDDEN`, `NOT_FOUND`, … |
| `crates/db/src/identity.rs` | accounts, pseuds, sessions |
| `crates/db/src/sessions.rs` | session issue, rotate, revoke |
| `crates/app/src/auth.rs` | `RequireSession`, `RequirePseud`, `MaybeSession` |
| `crates/app/src/crypto.rs` | password hashing, token generation |
| `crates/app/src/limiter.rs` | per-route rate limits |
| `crates/app/src/routes/auth.rs` | register, sign in, sign out, reset |
| `crates/app/src/routes/pseuds.rs` | pseud CRUD and switching |
| `crates/app/src/routes/settings.rs` | content preferences, privacy settings |
| `crates/domain/src/api_scopes.rs` | what a scope may do (used again in Part 12) |
| `crates/app/tests/milestone_2.rs` | the acceptance tests |
| `frontend/src/routes/Register.svelte`, `SignIn.svelte`, `Pseuds.svelte`, `Account.svelte` | the pages |

## 6. The code that matters

### Two identities, deliberately

```text
accounts:  email, password hash, age state, security history, settings
pseuds:    handle, display name, bio, the byline on everything readable
```

Almost every bug in a site like this comes from conflating them. A reader who
wants to post a review of their own work under a different name is not doing
anything wrong. A person deleting a pseud is not closing their account. The
account is the security boundary; the pseud is the visible identity. Keep that
line and the rest of the project stays simple.

### The extractors

```rust
// crates/app/src/auth.rs
pub struct RequireSession { pub user: SessionUser }   // must be signed in
pub struct RequirePseud  { pub user: SessionUser, pub pseud_id: PseudId }
pub struct MaybeSession  { pub user: Option<SessionUser> }
```

- Use `RequireSession` for things that belong to the account: settings, exports,
  session management, billing.
- Use `RequirePseud` for things that belong to a public identity: publishing a
  chapter, commenting, shelving a work.
- Use `MaybeSession` for anything a reader may do without an account. **This is
  the important one.** Public reading, public profiles, published works,
  published narration and published media must all be reachable with
  `MaybeSession`, and must apply the visibility rule themselves. A door that
  requires a session when the object is public is a bug that only shows up when
  someone links a story to a friend who does not have an account.

### The visibility rule, in one place

Later parts are going to need "may this caller read this work?" in a dozen
places: the work page, the media list, the narration door, the derivative
endpoint, the export. Write it once, in the works routes, and make it
`pub(crate)` so the other route modules use it too:

```rust
// crates/app/src/routes/works.rs — used by works, narration and derivative doors
pub(crate) fn actor_for(...) -> ...              // who is asking
pub(crate) fn reading_decision(...) -> ...       // may they read it
pub(crate) struct Reading { ... }                // the answer, with the reason
```

The behaviour it encodes:

| Caller | Object | Answer |
|---|---|---|
| anyone | published, public | read |
| signed in | published, unlisted | read, but not listed |
| contributor | own draft | read |
| anyone else | draft | **404** |
| anyone | explicit, age-ineligible | **404** |

That last row is a project-wide rule, not a per-endpoint one: *zero adult items
in any door*. A single handler that forgets it turns an explicit work into a
public link.

### Password and session handling

- Passwords: a memory-hard hash (Argon2id), never a fast digest. Store the
  parameters with the hash so you can raise them later.
- Sessions: a random opaque token in an `HttpOnly`, `SameSite=Lax` cookie. The
  database stores a hash of the token, not the token. `Secure` is set outside
  development — and `doctor` tells you when it is not.
- CSRF: state-changing requests authenticated by cookie require a token. The
  `doctor` check exists so you notice if the middleware ever gets dropped.
- Rotation: signing in on a new device must not extend every other session
  indefinitely, and signing out must revoke one session, not all of them.

### Rate limiting, from the start

Registration, sign-in, password reset and comment posting are the four endpoints
that get abused first. `crates/app/src/limiter.rs` applies per-route limits keyed
by the thing that actually costs you: IP for anonymous endpoints, account for
authenticated ones. Do this in the same part you build the endpoint; retrofitting
it after an incident means auditing every route.

## 7. Tests

`crates/app/tests/milestone_2.rs` covers:

- registration creates exactly one pseud, and the session works immediately;
- a duplicate email is refused with a field error, not a 500;
- the same password does not produce the same hash twice;
- signing out revokes the session server-side (the cookie alone is not enough);
- a blocked account cannot read the blocker's content, and cannot tell that the
  content exists;
- content preferences are stored per account and applied on the next read;
- an age-ineligible account gets 404 — not 403 — for an explicit work.

```bash
cargo test -p lorehaven-app --test milestone_2
```

Write the last one first. It is the test that tells you whether your visibility
rule is real or decorative.

## 8. Expected UI behaviour

- Registering takes you to the home page signed in, with one pseud.
- The header shows the active pseud, and switching it changes your byline
  everywhere without signing you out.
- Account settings hold email, password and sessions; pseud settings hold the
  public profile.
- Blocks are one-directional and invisible to the blocked party.
- Nothing shows a placeholder value where real data is missing.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Everything is 401 in a browser but fine in curl | cookie missing `SameSite` or the CSRF token is not sent | check the cookie attributes and the CSRF middleware order |
| A public work is 401 for a signed-out visitor | the door uses `RequireSession` | use `MaybeSession` and apply the visibility rule; Part 1's "expected UI" test catches it |
| Age-ineligible reader sees a title in a list | the filter ran on the detail door only | filter in the query, not in the renderer |
| Duplicate registration is a 500 | the unique violation is not mapped | catch the constraint and return a field error |
| Sessions survive sign-out | only the cookie was cleared | revoke the row too, and test it |

## 10. Consequences

- **The age policy is a promise with legal weight.** Once you claim an instance
  is not for children, the enforcement path is: registration, content
  eligibility, and the adult gate on every door. Keep them in one domain module
  so an auditor can read it in one sitting.
- **Pseud linkage is sensitive.** Nothing public may reveal that two pseuds
  belong to one account. That includes admin tools, exports and webhook payloads
  — you will revisit this rule in Parts 12 and 14.
- **Blocks must not become a discovery channel.** Blocking someone must not tell
  them they were blocked, and must not leak who you read.

## 11. Checkpoint

```bash
git tag v0.03-identity
```

Verified by `milestone_2.rs` on both dialects, plus a browser pass over register
→ sign in → switch pseud → settings. Owed from here on: every later door has to
pick its extractor deliberately, and the adult gate has to be applied to each new
door in the same commit that creates it.

# Part 3 — Works, chapters, revisions and publishing

Checkpoint: `v0.03-identity`

Everything in a fanfiction site is downstream of this part. If the work model is
wrong, four later parts inherit the mistake: comments anchor to chapters, the
reader remembers positions inside revisions, exports render a whole work, and the
positivity filter sees chapter text.

## 1. Checkpoint

```bash
git checkout v0.03-identity
```

## 2. What will work by the end

```bash
# Create a draft, add a chapter, publish it.
curl -X POST localhost:8080/api/v1/works -d '{"title":"Salt and Iron","kind":"fiction"}'
curl -X POST localhost:8080/api/v1/works/$WORK/chapters -d '{"title":"One"}'
curl -X PUT  localhost:8080/api/v1/chapters/$CH/revision -d '{"body":"…"}'
curl -X POST localhost:8080/api/v1/works/$WORK/publish

curl localhost:8080/api/v1/works/$WORK                 # the work, its chapters, its metadata
curl localhost:8080/api/v1/works/$WORK/revisions       # the revision history
```

A published work is readable by anyone, including signed-out readers, with the
chapters in reading order rather than upload order.

## 3. Concepts

- **A work is metadata; a chapter is a container; a revision is text.** Three
  levels, because that is what readers and writers actually change at different
  rates.
- **Drafts are private by construction.** Not "unlisted until published" — a
  draft has no published representation to leak.
- **Publishing is a transaction.** It either publishes the whole work or leaves
  it exactly as it was.
- **The revision cache is a cache, and it must be honest.** Reading a chapter
  must render the current revision, and a stale cache must never be served as if
  current.
- **Feedback preferences belong to the work, per chapter if needed.** A writer
  can ask for no critique on chapter one and everything on chapter twenty.

## 4. Commands

```bash
lorehaven migrate        # applies 0003_works and 0007_revision_cache
cargo test -p lorehaven-app --test milestone_3
cargo test -p lorehaven-app --test revision_cache
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0003_works.sql` | works, chapters, chapter revisions, tags |
| `migrations/sqlite/0007_revision_cache.sql` | the revision cache and its invalidation |
| `crates/domain/src/content.rs` | work states, chapter ordering, word counting |
| `crates/db/src/content.rs` | works, chapters, revisions |
| `crates/db/src/revisions.rs` | revision history and the cache |
| `crates/app/src/revisions.rs` | revision assembly for the API |
| `crates/app/src/routes/works.rs` | the work, chapter and publishing doors |
| `crates/app/src/routes/collaborators.rs` | co-authors and their permissions |
| `crates/app/tests/milestone_3.rs` | acceptance tests |
| `frontend/src/routes/Write.svelte` | the writer's dashboard |
| `frontend/src/routes/WorkEditor.svelte` | the editor (autosave lives here) |
| `frontend/src/routes/WorkPage.svelte` | the public work page |
| `frontend/src/lib/autosave.ts` | debounced draft saving |

## 6. The code that matters

### The three levels

```sql
works            id, owner_pseud_id, title, kind, lifecycle, language,
                 rating, created_at, updated_at
chapters         id, work_id, title, position, current_revision_id
chapter_revisions id, chapter_id, body, word_count, created_at, author_pseud_id
```

`current_revision_id` on the chapter is the single source of truth for "what a
reader sees". The revisions themselves are append-only. When you need "what did
this look like last week", you have it; when you need "what does it look like
now", you have one pointer and no ambiguity.

`position` is the reading order, and it is **not** the creation order. Writers
reorder chapters; importers append them out of order; a thread-scraped work
arrives back-to-front. Store an explicit position and sort by it — never by
timestamp.

### Publishing as a transaction

Publishing validates and then flips state:

```text
for each chapter: it must have a current revision
the work must have at least one chapter
the title must be non-empty
the work's feedback preferences must be readable
-- all of it inside one transaction
lifecycle = 'published', published_at = now
```

If any check fails, nothing changes: the writer keeps their draft, gets a field
error, and fixes one thing. A partial publish — some chapters public, some not —
is the kind of state you cannot get out of later.

### Draft visibility is the whole rule

The reading decision from Part 2 applies here and is worth restating as code you
should write deliberately:

```rust
// crates/app/src/routes/works.rs
match reading_decision(&actor, &work) {
    Reading::Allowed => { /* serve it */ }
    Reading::Denied  => return Err(AppError::NotFound), // 404, never 403
}
```

Only contributors (owner, co-authors with the right grant, admins acting through
an audited tool) may read a draft. Everyone else gets 404 — because a 403 on a
draft URL confirms the draft exists, which is itself information about a writer
who has not published anything yet.

### Autosave is a client concern, and a server contract

```ts
// frontend/src/lib/autosave.ts
// debounce edits, PUT the chapter revision, recover from a failed save
```

The server side of this must be boringly strict: a save is accepted only if it
carries the revision the client based it on. Otherwise you get the classic
silent data loss — two tabs, or a phone and a laptop, and the second save
overwrites the first. Return a conflict the client can resolve, and make the
client say so visibly instead of failing quietly.

### Word counts, once

`word_count` is stored on the revision, computed by one function in the domain
crate, and reused by: the work page, the reader, the export, the dashboard and
the search index. Compute it in the route and you will have five answers to the
same question, all slightly different.

## 7. Tests

`milestone_3.rs` asserts:

- a draft is 404 to a signed-out caller and to a different account;
- publishing is atomic: a work with an empty chapter fails, and the work is still
  a draft afterwards;
- chapters come back in `position` order, not insertion order;
- editing a chapter creates a new revision and moves `current_revision_id`;
- the word count matches the stored text;
- a save against a stale revision is refused with a conflict, not accepted.

`revision_cache.rs` asserts the cache is invalidated the moment a chapter is
edited — including the case where the edit happens while a read is in flight.

## 8. Expected UI behaviour

- The writer's dashboard lists drafts and published works with real counts.
- The editor autosaves, and shows a conflict when someone else's tab saved first.
- The public work page shows chapters in order, with word counts and the byline
  of the pseud, not the account.
- An unpublished work is not reachable from a link, a search, a feed, or an
  export.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Chapters appear in the wrong order | ordering by `created_at` | order by `position` |
| A reader sees an old chapter after an edit | the cache was not invalidated on the write path | invalidate in the same transaction as the revision insert |
| Two saves clobber each other | no base revision on the write | require and check it |
| Word counts differ between pages | computed in more than one place | one domain function, stored on the revision |
| A published work 404s for a signed-out reader | the door requires a session | `MaybeSession` + the visibility rule |

## 10. Consequences

- **Append-only revisions are a privacy surface.** A revision that was published
  stays in history unless you delete it deliberately. If a writer edits out a
  real name, the old revision still holds it: decide now whether history is
  public, private, or available to the author only, and say so in the UI.
- **`position` is a contract.** Anything that renumbers positions (an import, a
  merge, a fork) must do it in one transaction, or readers will see duplicates
  and gaps.
- **Draft metadata is still metadata.** Titles, tags and summaries of drafts
  leak intent. Exclude drafts from search, discovery, feeds and statistics
  everywhere — Part 8 and Part 9 have to re-apply this rule.

## 11. Checkpoint

```bash
git tag v0.04-publishing
```

Verified by `milestone_3.rs` and `revision_cache.rs` on both dialects, plus a
browser pass: create a draft, write two chapters, reorder them, publish, open the
work in a signed-out browser session. Owed: the reader (Part 4) still has no
memory, and nothing yet tells the author whether anyone read it.

# Part 4 — The reader: ratings, reviews, notes, history, goals

Checkpoint: `v0.04-publishing`

A site that publishes but does not remember is a blog. This part gives the
reader a memory: where they are in a work, what they thought of it, what they
noticed, and what they meant to read next.

## 1. Checkpoint

```bash
git checkout v0.04-publishing
```

## 2. What will work by the end

```bash
curl -X PUT localhost:8080/api/v1/works/$WORK/rating -d '{"stars":4,"is_public":true}'
curl -X POST localhost:8080/api/v1/works/$WORK/reviews -d '{"body":"…","is_public":true}'
curl -X PUT localhost:8080/api/v1/chapters/$CH/progress -d '{"position":0.42}'
curl localhost:8080/api/v1/me/history
curl localhost:8080/api/v1/me/goals
```

And in a browser: read a story, close the tab, come back and be offered the
chapter you stopped in the middle of.

## 3. Concepts

- **Ratings and reviews are different kinds of fact.** A rating is a number the
  reader can change or withdraw; a review is text with a publication state.
- **Progress is not history.** "Where I am" is current state; "what I read" is a
  log. Storing one as the other loses information you will want.
- **Private by default.** Notes and reading history belong to the reader. Making
  something public is an explicit act.
- **Goals are a promise to yourself**, and the honest way to render them is
  against real data, never a streak you cannot compute from the log.
- **Reactions are cheap and must stay cheap.** A quick reaction must not create
  a notification storm or an unbounded row per interaction.

## 4. Commands

```bash
lorehaven migrate        # applies 0004_reading
cargo test -p lorehaven-app --test milestone_4
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0004_reading.sql` | ratings, reviews, progress, history, notes, goals |
| `migrations/postgres/0004_reading.sql` | the same, PostgreSQL |
| `crates/domain/src/reading.rs` | progress maths, rating rules, goal arithmetic |
| `crates/db/src/reading.rs` | the reader's tables |
| `crates/app/src/routes/reading.rs` | the reader's doors |
| `crates/app/tests/milestone_4.rs` | acceptance tests |
| `frontend/src/routes/Reader.svelte` | the reader itself |
| `frontend/src/routes/History.svelte` | what I have read |
| `frontend/src/lib/reading.ts` | progress restore and local-first resume |

## 6. The code that matters

### Tables

```sql
rating          (pseud_id, work_id, stars, is_public, deleted_at)
review          (pseud_id, work_id, body, is_public, published_at, deleted_at)
reading_progress(pseud_id, work_id, chapter_id, position, updated_at)
read_history    (pseud_id, work_id, chapter_id, at)
note            (pseud_id, chapter_id, anchor_kind, anchor_value, body)
goal            (pseud_id, year, target, kind)
```

Two things to notice:

- **`deleted_at`, not `DELETE`.** A withdrawn rating must stop counting towards
  the average, and must be restorable. Soft deletion with a filter on every read
  is the cheap way to get both. Every aggregate must carry `deleted_at IS NULL` —
  this is exactly the kind of clause that gets forgotten in one of five queries
  and produces an average that disagrees with the count.
- **Progress is per work *and* per chapter.** A reader can be on chapter nine
  while only having finished it halfway. Store the unit you resume from.

### The public/private line

```text
rating.is_public = false   → counts for the reader, not for the author's average
review.is_public = false   → a private note-to-self, not a review
history and notes          → never public, never in an export addressed to anyone else
```

When you later build the creator dashboard (Part 6), the author's view counts
only public ratings and only published reviews. That is not a detail — a writer
seeing a private "I gave this two stars" is a privacy breach, not a feature.

### Resuming a read

Resolution order, and stop at the first hit:

1. server-side `reading_progress` for this pseud and work;
2. a local copy in the browser (for signed-out readers, and for offline reading
   in Part 9);
3. the start of the first chapter.

Write it in one function used by both the reader and the resume prompt, or the
two will disagree and the reader will be offered chapter three while sitting on
chapter nine.

### Reactions

Quick reactions (a heart, a bookmark, a "more like this") are stored as one row
per reader per target per kind, and the counts are aggregated on read. Do not
store a counter column you increment: you will need to subtract on withdrawal,
and a double-submit will silently corrupt it.

## 7. Tests

`milestone_4.rs` asserts:

- rating a work twice updates rather than duplicates;
- withdrawing a rating removes it from the average but keeps the row (restorable);
- a private rating does not appear in the work's public average;
- progress is stored and returned per chapter, and the resume point survives a
  chapter edit (the chapter exists, the revision changed);
- a private note never appears in a response to anyone else;
- a goal's progress is computed from the history log, not from a stored counter.

```bash
cargo test -p lorehaven-app --test milestone_4
```

## 8. Expected UI behaviour

- Opening a work you have read offers "continue in chapter nine" — and does not
  offer it if you finished.
- Ratings are editable and removable.
- Reviews show their own state: draft, published, or private.
- History is a real log with dates, not a synthetic streak.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| The average disagrees with the count | one query forgot `deleted_at IS NULL` | filter in every aggregate; test the pair together |
| Resume always restarts at chapter one | progress keyed on the revision, not the chapter | key on chapter id |
| History grows without bound | every page turn is a row | record a read once per chapter per session |
| A private rating shows on the author's dashboard | the dashboard query does not filter `is_public` | filter, and test it from the author's account |

## 10. Consequences

- **Reading history is the most sensitive data in the application.** It reveals
  what someone reads, when, and how far. It must be excluded from every export
  addressed to anyone but the reader, from every webhook payload, and from every
  administrator tool that is not audited.
- **Notes can contain anything.** Treat note bodies as user content subject to
  the same moderation surfaces as comments (Part 10) — or decide now that they
  are strictly private and never reportable, and enforce that they never surface
  in a report queue.

## 11. Checkpoint

```bash
git tag v0.05-reader
```

Verified by `milestone_4.rs` on both dialects plus a browser pass: read across
three chapters, reload, resume, rate, write a private note, and confirm none of
it shows to another account.

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

# Part 6 — The positivity filter and feedback delivery

Checkpoint: `v0.06-jobs`

This is the part that makes the site different from every other comment section,
and it is the part most likely to be built wrong. Read the rule twice before you
write any code:

> The filter exists to protect the writer's experience of their own work. It does
> not exist to punish the reader, and it must never lie to either of them.

## 1. Checkpoint

```bash
git checkout v0.06-jobs
```

## 2. What will work by the end

```bash
curl -X PUT localhost:8080/api/v1/works/$WORK/feedback-preferences \
  -d '{"mode":"positive_only","allow_list":["trusted-reader"]}'

curl -X POST localhost:8080/api/v1/works/$WORK/comments \
  -d '{"body":"the pacing in chapter four is astonishing"}'
# 201 { "comment": { ..., "receipt": "Comment posted." } }

curl -X POST localhost:8080/api/v1/works/$WORK/comments \
  -d '{"body":"this is garbage, stop writing"}'
# 201 { "comment": { ..., "receipt": "Comment held for moderator review." } }
```

The author sees the first, and does not see the second. The second commenter is
told their comment was held for review — which is true, and is all they are
entitled to know. Their comment is stored, not destroyed.

## 3. Concepts

- **Two outcomes, and only two.** `delivered` or `held`. A third state ("deleted
  because the filter disliked it") loses the text and with it the ability to
  review, appeal or learn.
- **The sender learns their own outcome, never the author's settings.** The
  receipt says "posted" or "held for review". It never says "this author only
  accepts positive comments" — that would let anyone probe a writer's settings by
  posting and reading the response.
- **Stored outcomes are never rewritten.** Classification rules improve; a
  comment classified last month keeps the outcome it was given, unless a human
  reviews it. Rewriting history makes the numbers unexplainable.
- **Rules first, models later.** A deterministic rule pass (insult patterns,
  hostility markers, spam shapes) runs first. An optional model pass is an
  additional input, not the definition of the feature, so an instance with no
  model still has a working filter.
- **The author's view is framed as what arrived.** No "held by the filter"
  counter, no scoreboard of how many people were silenced — the author's view
  counts what reached them.

## 4. Commands

```bash
lorehaven migrate        # applies 0010_positivity and 0015_comment_positivity
cargo test -p lorehaven-app --test milestone_7
cargo test -p lorehaven-domain positivity
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0010_positivity.sql` | classifications, author preferences, allow/deny lists |
| `migrations/sqlite/0015_comment_positivity.sql` | comment outcomes |
| `crates/domain/src/positivity.rs` | the rule pass, the outcomes, the receipts |
| `crates/db/src/positivity.rs` | classification storage and lookups |
| `crates/app/src/routes/feedback.rs` | the author's inbox, allow and deny |
| `crates/app/src/routes/comments.rs` | the comment door, with classification |
| `crates/app/src/routes/works.rs` | feedback preferences on the work |
| `crates/app/tests/milestone_7.rs` | acceptance tests |

## 6. The code that matters

### The outcome vocabulary

```rust
// crates/domain/src/positivity.rs
pub enum DeliveryOutcome { Delivered, Held }          // "delivered" | "held"

pub const fn sender_receipt(outcome: DeliveryOutcome) -> &'static str {
    match outcome {
        DeliveryOutcome::Delivered => "Comment posted.",
        DeliveryOutcome::Held      => "Comment held for moderator review.",
    }
}
```

The commenter's receipt is deliberately the same sentence for every reason a
comment was held: a hostile comment, a comment from someone on the author's deny
list, a comment on a work whose author chose "positive only". If the receipts
differed, they would be an oracle for the author's settings.

### Author preferences, and how they apply

```text
mode = open            → everything is delivered
mode = positive_only   → the rule pass decides; held if it flags the text
allow_list             → these pseuds bypass the filter
deny_list              → these pseuds are always held
```

The author's settings are read at classification time and stored on the
classification row, because the settings can change later and you must be able to
explain an old decision.

### Where classification runs

Classification runs **in the same transaction** as the comment insert. If it ran
asynchronously, there would be a window in which a hostile comment is publicly
visible — which is the entire thing the filter exists to prevent. That means the
rule pass must be fast and free of I/O. Put the model call behind the queue
(Part 5) only if you can hold the comment until the answer arrives; otherwise run
rules inline and treat the model as a re-classification of *already delivered*
text, with the outcome never rewritten except by a human.

### The author's inbox

`GET /api/v1/feedback/inbox` returns what arrived, per work: delivered comments,
and reviews. Two rules for this endpoint:

- it is the author's own pseud, so the door is `RequirePseud` and every row is
  filtered to works owned by that pseud;
- the payload contains no count of held items, and no per-reader identity beyond
  what the comment itself carries.

### What a moderator sees, and what the author does not

Held comments are visible to moderators (Part 11), with the classification reason
and the text intact. That is the mechanism that makes the filter reviewable: a
false negative is visible to a human, and the comment can be delivered manually —
which is a state change a human makes, with an audit row, never something the
rules do retroactively.

## 7. Tests

`milestone_7.rs` asserts:

- a hostile comment is held, and the author's inbox does not contain it;
- the held comment still exists, and is visible to a moderator;
- the receipt for a held comment is identical whether the cause was the text, the
  author's mode, or the deny list;
- a pseud on the allow list bypasses the rule pass;
- an author's dashboard counts delivered feedback only, and contains no string
  resembling a held-item count;
- the same comment body posted twice produces two classifications, and the stored
  outcome of the first is not rewritten by the second;
- an instance with no model configured still holds every text the rules flag.

## 8. Expected UI behaviour

- A held comment appears to its author exactly once, in their own view, labelled
  "held for review".
- The author's feedback inbox shows what arrived, framed as feedback.
- No screen anywhere shows the author a count of comments that were withheld.
- No screen shows a commenter anything about the author's filter settings.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Hostile comment briefly public | classification ran after the insert committed | classify inside the transaction |
| A commenter can detect the author's mode by probing | the receipt varies by cause | one receipt for every held case |
| The author's counts include held items | the query joins classifications without filtering `outcome` | filter on `outcome = 'delivered'` |
| Re-classifying changes old rows | a backfill rewrote outcomes | store the outcome once; changes are human actions with an audit row |
| The filter holds everything | the rule pass is too broad and nobody noticed | assert a corpus of ordinary praise still delivers |

## 10. Consequences

- **You are keeping a record of what people said that the author did not see.**
  That record is legitimate — it is the review queue — but it is also sensitive:
  it holds hostile text, and its existence must be documented in your privacy
  page, with a retention period.
- **The author's protection must not become a moderation blind spot.** Held text
  has to reach a human who can act, or the filter simply hides abuse from the
  people who could stop it.
- **A held comment is not a banned reader.** Nothing in this part sanctions
  anyone; escalation belongs to Part 11, where it is auditable.

## 11. Checkpoint

```bash
git tag v0.08-positivity
```

Verified by `milestone_7.rs` plus the domain rule tests, and by hand: post a
praise comment and a hostile one from two accounts, and confirm what each party
sees.

# Part 7 — Imports: fetching, adapters, sanitising, shelf CSVs

Checkpoint: `v0.08-positivity`

Two different things are called "import" in this project, and confusing them
costs a week:

1. **Works import** — fetch a story from another site and turn it into a work
   with chapters. Fetches the network, needs a source adapter, produces content.
2. **Shelf imports** — read a Goodreads or StoryGraph export and produce library
   rows for the reader. Never touches the network, never produces content.

They share a word and nothing else. This part builds both.

## 1. Checkpoint

```bash
git checkout v0.08-positivity
```

## 2. What will work by the end

```bash
# A URL import becomes a queued job, then a draft work.
curl -X POST localhost:8080/api/v1/imports/preview -d '{"url":"https://example.invalid/story/1"}'
curl -X POST localhost:8080/api/v1/imports -d '{"url":"…","kind":"work"}'
curl localhost:8080/api/v1/jobs/$JOB

# A shelf export becomes library rows, states included.
curl -X POST localhost:8080/api/v1/library/imports/csv \
  -H 'content-type: application/json' \
  -d '{"format":"storygraph","csv":"Title,Authors,ISBN,My Rating,Date Read,Review\n…"}'
# { "imported": 2, "kept_existing_state": 0, "refused": [ { "title": "line 3", "reason": "…" } ] }
```

## 3. Concepts

- **The fetcher is the only thing that talks to the outside world**, and it has
  its own rules: allow-listed schemes, bounded redirects, bounded body size, an
  honest `User-Agent`, and robots rules that are read rather than guessed.
- **An adapter is a pure function over documents.** Give it HTML, get chapters.
  It does no I/O, so it is testable against a saved file.
- **Sanitising is not optional and not a rendering concern.** Fetched text is
  cleaned *before* it is stored, so no later path — export, feed, API — can
  reintroduce what was stripped.
- **Imports are attributed.** An imported work carries its source URL, its
  adapter, and when it was fetched. That is the record that makes a takedown
  request answerable.
- **A CSV export is not a scrape.** It is a file the reader downloaded from a
  service about themselves. It gets no network call, no HTML parsing, and no
  assumptions about where any book's text is.

## 4. Commands

```bash
lorehaven migrate        # applies 0006_imports
cargo test -p lorehaven-scrapers
cargo test -p lorehaven-app --test milestone_6
cargo test -p lorehaven-app --test milestone_24
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0006_imports.sql` | imports, batches, source credentials, library items, provenance |
| `crates/scrapers/src/lib.rs` | the fetcher: schemes, redirects, size, robots |
| `crates/scrapers/src/html.rs` | document parsing helpers |
| `crates/scrapers/src/csv.rs` | Goodreads and StoryGraph shelf parsing, and the import plan |
| `crates/scrapers/src/csv/goodreads.rs`, `csv/storygraph.rs` | one file per export format |
| `crates/domain/src/imports.rs` | chapter identity, plan shape |
| `crates/db/src/imports.rs` | import rows, batches, `upsert_library_item` |
| `crates/app/src/imports.rs` | the job handler |
| `crates/app/src/routes/imports.rs` | preview, submit, cancel, and the shelf CSV door |
| `crates/app/tests/milestone_6.rs` | work-import acceptance tests |
| `crates/app/tests/milestone_24.rs` | shelf-import acceptance tests |
| `frontend/src/routes/Import.svelte` | the import page |

## 6. The code that matters

### The fetcher's contract

```rust
// crates/scrapers/src/lib.rs — the boundary between a pasted URL and the network
const MAX_REDIRECTS: usize = 5;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
```

Every one of these is a real incident shape: an unbounded redirect chain to an
internal address, a body that fills the disk, a `file://` URL that reads the
server's own filesystem. Refuse first, fetch second. And name the site honestly:
a descriptive `User-Agent` with a contact URL, because the alternative is being
blocked for good.

### Adapters as pure functions

```rust
// crates/scrapers/src/html.rs — one adapter per source, no I/O inside
pub fn chapters_from_document(html: &str) -> Result<Vec<PlannedChapter>>
```

Test each adapter against a saved document from the real site, checked into the
repository. That file is also your evidence of what the site actually served on
the day you wrote the adapter — which is worth more than a screenshot when the
site changes and a reader asks why their import looks wrong.

### The shelf import plan

The CSV path is where a junior developer is most likely to write something that
works and is subtly wrong, so it is worth walking through the real flow:

```rust
// crates/scrapers/src/csv.rs
pub struct ImportPlan { pub items: Vec<PlannedItem>, pub refused: Vec<RefusedRow> }

pub fn plan_shelf_import(shelf: &ImportShelf) -> ImportPlan
```

The plan is a **pure function of the parsed file**. Nothing in it touches the
database; the route walks the plan and writes. That is what lets you unit-test
every refusal rule without a server.

The rules the plan enforces:

| Input | Result |
|---|---|
| title and author present | an item |
| `Date Read` = `2019/03/09` | state `finished`, `finished_at` = `2019-03-09T00:00:00Z` |
| `Date Read` = `2019-12-31T10:11:12Z` | the timestamp, with its time |
| `Date Read` = `31 December 2019` | **refused**, naming the value as written |
| no title | **refused by file line** |
| rating, shelves, review text | carried in `provenance_json` |

Two details in that table are the ones people get wrong:

**The timestamp branch must be tested first.** A full RFC 3339 timestamp also
starts with a `YYYY-MM-DD` shape. If the date-only branch runs first, the time is
silently dropped and `2019-12-31T10:11:12Z` becomes midnight. This is a bug that
survives review because the date is right.

**A row the parser could not read is refused by line number.** A row with no
title has no other identity in the file. "3 rows skipped" leaves the reader
searching their export by eye; "line 3 has no title" does not. That is why the
parsers record the line numbers they skip instead of only counting them:

```rust
pub struct ImportShelf {
    pub rows: Vec<ShelfRow>,
    pub skipped: usize,
    /// The file lines the parser could not read, 1-based, header included.
    pub skipped_lines: Vec<usize>,
}
```

### Imported rows do not overwrite the reader

```rust
// crates/db/src/library.rs
pub async fn set_imported_reading_status(...) -> Result<bool>   // writes only if absent
```

A re-import reports `kept_existing_state` rather than overwriting a state the
reader set themselves. The mirror-image rule applies to the item row: the unique
key `(account, source, source_work_key)` makes a re-import update the same row,
so importing the same file twice is idempotent by construction rather than by a
check that could race.

### No invented URLs

```rust
source_url: format!("import://{format}/{}", item.source_work_key)
```

A shelf export carries no URL for a book. A fabricated `https://` link is a link
somebody will later follow and act on. An `import://` URI says where the row came
from and is not fetchable by anything. When in doubt, prefer a value that cannot
be mistaken for a live address.

## 7. Tests

`milestone_6.rs` (works import) asserts:

- a `file://` URL and a redirect chain are refused before any request is made;
- an oversized body is refused rather than buffered;
- a saved document produces the expected chapters, in order, with sanitised text;
- an imported work is attributed: source URL, adapter, fetch time;
- the import job is visible to its owner and to nobody else.

`milestone_24.rs` (shelf import) asserts:

- a StoryGraph-shaped file imports two rows and refuses the untitled one by name;
- the finished row carries the reader's `Date Read`, not the import time;
- re-importing the same file adds nothing and reports `kept_existing_state`;
- an unknown format is refused before anything is written.

## 8. Expected UI behaviour

- The import page shows a preview before it creates anything: the work, the
  chapter count, the source.
- A running import shows progress per chapter.
- The library page shows imported rows with their provenance, and never claims
  the instance holds the text of a book it does not have.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Import hangs | no overall timeout on the fetch | bound connect, read and total time |
| Chapters in the wrong order | the adapter took document order | order by the site's own ordinals; fall back to document order only when there are none |
| Imported text contains scripts or odd markup | sanitising happened at render | sanitise before storing |
| Re-import duplicates rows | no unique key on (account, source, source_work_key) | add it; make the write an upsert |
| Imported count is wrong after a re-import | counting rows written rather than states set | count the state changes, and report what was kept |
| `Date Read` loses its time | date-only branch matched first | check full timestamps first (see §6) |

## 10. Consequences

- **You are storing someone else's text.** The attribution record and the
  takedown path are not bureaucracy; without them a single complaint has no
  answerable response.
- **Credentials for source sites are secrets** on the Part 5 rules: encrypted at
  rest, decrypted only at use, never returned by an API, and included in the
  "revoke everything" path when an account is closed.
- **An imported shelf is a statement about a person's reading.** It is private
  data: never public, never in a shared export, and deletable in one action.

## 11. Checkpoint

```bash
git tag v0.07-importing
```

Verified by the `lorehaven-scrapers` unit suite, `milestone_6.rs` and
`milestone_24.rs`, and by hand: import a saved document from a test fixture and
import a StoryGraph CSV, then re-import each and confirm nothing duplicates.

# Part 8 — Library, shelves, taxonomy and search

Checkpoint: `v0.07-importing`

A reader with four hundred imported rows needs to find one of them. This part is
about making a large personal library navigable, and about the one thing every
list endpoint gets wrong the first time: pagination.

## 1. Checkpoint

```bash
git checkout v0.07-importing
```

## 2. What will work by the end

```bash
curl -X POST localhost:8080/api/v1/shelves -d '{"name":"read-again"}'
curl -X PUT  localhost:8080/api/v1/library/items/$ITEM/status -d '{"status":"finished"}'
curl -X POST localhost:8080/api/v1/library/items/batch -d '{"ids":["…"],"action":"remove"}'

curl 'localhost:8080/api/v1/library/items?sort=recent&limit=50'
curl 'localhost:8080/api/v1/works?q=rating:4..5 mood:hurt-comfort words:<20000&limit=20'
curl 'localhost:8080/api/v1/works?q=title:salt&limit=20&cursor=…'
```

## 3. Concepts

- **Shelves are the reader's, tags are everyone's.** A shelf is a private
  grouping with a name the reader chose; a tag is public metadata on a work.
- **Saved views are queries with a name.** Not a separate storage system.
- **A query language needs a parser and a compiler**, and the compiler must
  produce SQL with bound parameters — never string interpolation.
- **Fuzzy matching is for humans typing**, so it ranks. Exact matching is for
  filters, so it filters.
- **Pagination is part of the API contract, not an optimisation.** A list
  endpoint with a silent `LIMIT` is a bug that shows up as "my work disappeared
  from the library".

## 4. Commands

```bash
lorehaven migrate        # applies 0009_library and 0011_taxonomy
cargo test -p lorehaven-app --test milestone_8
cargo test -p lorehaven-app --test milestone_9
cargo test -p lorehaven-app --test milestone_10
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0009_library.sql` | library items, shelves, shelf items, reading status, bookmarks, saved views |
| `migrations/sqlite/0011_taxonomy.sql` | tags, moods, tag applications |
| `crates/domain/src/library.rs` | shelf rules, reading states, view definitions |
| `crates/domain/src/taxonomy.rs` | tag normalisation, mood vocabulary |
| `crates/domain/src/query.rs` | the query language: lexer and parser |
| `crates/domain/src/query_sql.rs` | the compiler: AST → SQL + bound parameters |
| `crates/db/src/library.rs` | the library's tables |
| `crates/db/src/taxonomy.rs` | tags and moods |
| `crates/db/src/search.rs` | the retrieval paths (and `search/` for the fuzzy one) |
| `crates/app/src/routes/library.rs` | shelves, status, bookmarks, saved views |
| `crates/app/src/routes/taxonomy.rs` | tags and moods |
| `crates/app/src/routes/search.rs` | the query door |
| `frontend/src/routes/Library.svelte`, `Search.svelte` | the pages |

## 6. The code that matters

### The query language, compiled not concatenated

```rust
// crates/domain/src/query.rs    parse("rating:4..5 mood:hurt-comfort words:<20000")
// crates/domain/src/query_sql.rs  →  (sql_fragment, Vec<BoundValue>)
```

The compiler's output is a fragment containing `?` placeholders and a list of
values. Every value the user typed is a bound value. A query language is the
single most attractive place in a codebase to build a SQL injection, because the
shortcut ("just format the number in") works, tests green, and is exploitable.

Two more rules for this module:

- **Unknown fields are an error, not a no-op.** `ratng:4` silently ignored means
  the reader believes they filtered and sees unfiltered results.
- **Limits are clamped, not refused.** `limit=100000` becomes the maximum, with
  the effective limit in the response, so a client can tell.

### Cursor pagination, done properly

The bug, in its usual form:

```sql
-- wrong: stable only if nothing is inserted while you page
SELECT … ORDER BY created_at DESC LIMIT 50 OFFSET 50
```

Offset paging over a table that is being written to skips and repeats rows. Use a
cursor that carries **the whole ordering key**, exactly what `ORDER BY` compares:

```text
ORDER BY position, created_at, id
cursor = "<position>|<created_at>|<id>"
next_cursor returned only when the page was full
```

`id` at the end is not decoration: without a unique final tiebreaker, two rows
with the same position and timestamp make the cursor ambiguous and the pager can
loop. Return `next_cursor` only for a full page, or every client will make one
extra empty request forever.

```bash
# the walk that proves it, in the tests
# seed five works, page with limit=2 → three pages, each item exactly once, in order
```

That test — not a single-page assertion — is what proves pagination is right.

### Fuzzy matching, and where it belongs

```text
q=title:salt         → exact/stemmed match, ranked, cheap
q=salt and iron      → full-text
q=salt and iorn      → fuzzy (edit distance) — ranked last, always
```

Fuzzy matching must never be the *only* path: it is expensive, and it produces
confident nonsense on short strings. Rank it below exact matches, cap its result
count, and never let it satisfy an exact field filter.

### `library_items` and its nullable `work_id`

An imported row has no work: it is a book the reader read elsewhere. A row that
came from a local work has a `work_id`. **Never** join library rows to works with
an inner join and call the result "the library" — half the rows vanish. This is
the same shape as the import rule in Part 7: the honest model is "a library row
that may point at a work", and every aggregate has to tolerate the null.

## 7. Tests

`milestone_8.rs` (library):

- a shelf is private to its owner; another account gets 404 for its id;
- setting a reading status twice is idempotent;
- a batch removal reports per-id outcomes rather than failing wholesale;
- a saved view round-trips and its stored query re-parses.

`milestone_9.rs` (taxonomy):

- tags are normalised (case, whitespace, punctuation) before storage;
- two spellings of a tag resolve to one tag;
- a mood filter returns only works tagged with that mood.

`milestone_10.rs` (search):

- each operator (rating, words, mood, tag, status) filters correctly;
- an unknown operator is a 400;
- `rating:4..5` is inclusive at both ends — off-by-one here is invisible until
  someone compares two pages;
- a two-page cursor walk returns each work exactly once;
- `limit=0` and a malformed cursor are refused;
- a signed-out reader never sees a draft in any result.

## 8. Expected UI behaviour

- Library filters are shareable URLs.
- "Saved views" appears in the sidebar with the reader's own names.
- Search shows which filters are active, not just a result list.
- Paging keeps scroll position and never repeats a row.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| A row appears on two pages | offset paging over a written table, or a non-unique cursor | cursor with the full ordering key |
| "My library is empty" | inner join to works | left join; imported rows have no work |
| `q=ratng:4` returns everything | unknown field ignored | 400 on unknown operators |
| Mood filter is slow | filtering in Rust after fetching | filter in SQL, with the mood table indexed |
| Two tags that look identical | normalisation applied on write only once, or not at all | normalise in one function used by every write path |
| Drafts leak into results | the visibility rule was applied in the detail door only | apply it in the query |

## 10. Consequences

- **A library is a reading history, and reading history is sensitive** (Part 4).
  Every library endpoint is `RequirePseud`, and none of them may be served by
  anyone but the owner — including an administrator, unless the tool is audited.
- **Tags are public and therefore a moderation surface.** Plan for tag abuse
  (slurs, spam) before you open tagging to everyone.
- **A saved view can encode a private query.** Do not put the raw query in a URL
  that is shared publicly, and never let a view be executed as another account.

## 11. Checkpoint

```bash
git tag v0.11-search
```

Verified by `milestone_8.rs`, `milestone_9.rs`, `milestone_10.rs` on both
dialects, plus a browser pass: import a shelf, shelve a few rows, save a view,
and page through search results twice.

# Part 9 — Discovery, exports and offline reading

Checkpoint: `v0.11-search`

Two halves of the same idea: get new things in front of a reader, and let them
take what they have away with them.

## 1. Checkpoint

```bash
git checkout v0.11-search
```

## 2. What will work by the end

```bash
curl 'localhost:8080/api/v1/discover'                    # a real front page
curl 'localhost:8080/api/v1/discover/recipes'            # the recipes behind it
curl -X POST localhost:8080/api/v1/exports -d '{"work":"…","format":"epub"}'
curl localhost:8080/api/v1/jobs/$JOB                     # the export, built by the worker
curl localhost:8080/api/v1/exports/$EXPORT/download
```

And in a browser: load the site, go offline, keep reading the work you had open,
then reopen the tab and see your progress reconciled with the server.

## 3. Concepts

- **Discovery is a query with a purpose, not a set of hand-picked shelves.** A
  "recipe" is a named, explainable rule ("finished works over 20k words tagged
  hurt/comfort that you have not opened").
- **Every recommendation must be explainable in one sentence.** If you cannot
  say why a work is on the page, it should not be on the page.
- **Taste influence is derived, private and inspectable.** A reader must be able
  to see what the instance thinks they like, and turn it off.
- **Exports are jobs.** An EPUB of a 400k-word work takes long enough to time out
  a request, and long enough that a client will retry — hence Part 5.
- **Offline is a first-class state, not an error state.** The reader must work
  with no network and reconcile honestly when it comes back.

## 4. Commands

```bash
lorehaven migrate        # applies 0008_exports and 0012_discovery
cargo test -p lorehaven-app --test milestone_11
cargo test -p lorehaven-domain exports
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0008_exports.sql` | export rows, formats, delivery |
| `migrations/sqlite/0012_discovery.sql` | recipes, taste signals, dashboards |
| `crates/domain/src/discovery.rs` | recipe evaluation, ranking, explanations |
| `crates/domain/src/exports.rs` | the export document model |
| `crates/domain/src/exports/epub.rs` | the EPUB builder |
| `crates/db/src/discovery.rs`, `db/exports.rs` | storage |
| `crates/app/src/exports.rs` | the export job handler |
| `crates/app/src/routes/discovery.rs`, `routes/exports.rs` | the doors |
| `frontend/src/routes/Discover.svelte`, `Exports.svelte` | the pages |
| `frontend/src/lib/offline.ts` | the offline queue and reconciliation |

## 6. The code that matters

### A recipe is a function with a name

```text
recipe "continue what you started":
  a work with reading progress < 100%
  and at least one chapter read
  and not finished
order by  updated_at desc
explain  "you read chapter 4 of 12 two days ago"
```

The `explain` clause is not decoration — it is the acceptance criterion. Write
the explanation function next to the query, and assert on it in tests.

### Taste signals

Signals are cheap, private rows: `(pseud_id, kind, subject, weight, at)`. They
come from things the reader already did — finished a work, rated it highly, read
three chapters of another. Two rules:

- **Never include a signal derived from a blocked party's content.**
- **Expose them.** `GET /api/v1/me/taste` (or the settings page) must show what
  the instance has inferred, with a way to clear it. A recommendation system the
  user cannot inspect is a surveillance system with a nicer font.

### The EPUB builder

```rust
// crates/domain/src/exports/epub.rs
// mimetype (stored first, uncompressed) → META-INF/container.xml →
// OEBPS/: content.opf, nav.xhtml, chapters, style.css
```

Details that will cost you an afternoon each if you get them wrong:

- `mimetype` must be the first entry and stored uncompressed, or readers refuse
  the file.
- Chapter order in `content.opf` must match the `spine` order; the nav document
  must list the same order.
- Escape everything: a chapter title containing `&` breaks XML, and the failure
  appears only in the reader app, not in your tests — unless you test with a
  hostile title.
- Attribution goes in the document: the author's pseud and the source URL for an
  imported work. An export that strips attribution is a licence problem.

Test the builder by asserting the ZIP entry order, the OPF spine and the presence
of each chapter's text — not by eyeballing a file in Calibre.

### Offline reading

```ts
// frontend/src/lib/offline.ts
// 1. cache the work's chapters when reading starts
// 2. queue progress writes locally when the network fails
// 3. on reconnect, reconcile: newest timestamp wins, and say so
```

The reconciliation rule has to be written down, because "merge" means nothing on
its own. Pick newest-wins for progress, never-wins for anything destructive, and
show the reader a line like "Synced — your offline progress was kept" rather than
silently choosing.

## 7. Tests

`milestone_11.rs` and the discovery/export tests assert:

- every recipe returns only published, readable works for the calling reader;
- a blocked work never appears in discovery for the blocker;
- the explanation for each recommended work is non-empty and derived from the
  query, not from a template that is always the same string;
- clearing taste signals changes the next discovery response;
- an EPUB has `mimetype` first and uncompressed, a spine matching the nav, and
  every chapter's text present;
- an export of a work the caller may not read is 404;
- an export job for a work edited mid-export either completes against a snapshot
  or fails cleanly — it never produces a file with missing chapters;
- offline progress queued while offline is applied once, not twice, on reconnect.

## 8. Expected UI behaviour

- The front page changes for a signed-in reader and is honest for a signed-out
  one (popular published works, not a personal feed that cannot exist).
- Every recommended work has a one-line reason.
- An export request shows a job, then a download, and the file opens in a real
  reader.
- Losing the network mid-chapter shows a quiet offline indicator, and reading
  continues.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| EPUB rejected by a reader app | `mimetype` not first or compressed | write it first, stored |
| Missing chapters in an export | the work was edited during the job | snapshot the revision ids at job start |
| Discovery empty for a new reader | recipes all depend on history | include a cold-start recipe over published works |
| Recommendations repeat forever | no exclusion of what was already opened | filter on read history |
| Duplicate offline progress | the queue is applied on every reconnect | clear the queue after a successful write, keyed by an idempotency token |

## 10. Consequences

- **Recommendations can out someone.** "Readers who liked X also liked Y" on a
  site with sensitive content can reveal what someone reads. Never surface a
  person in a recommendation the reader did not publish themselves.
- **An exported EPUB leaves your instance forever.** It carries the work and its
  attribution, and it cannot be recalled — which is a feature for the reader and
  a reason to be careful about what goes into it.
- **Offline caches hold content on a device you do not control.** Say so, and
  give the reader a way to clear it.

## 11. Checkpoint

```bash
git tag v0.12-discovery
```

Verified by the milestone tests plus a manual pass: build an EPUB of a large
work, open it in a real reader app, then read offline and reconcile.

# Part 10 — Comments, forums, groups, messaging and events

Checkpoint: `v0.12-discovery`

This is the part where the site gets a social surface, and therefore the part
where the abuse surface grows fastest. Build it with the moderation tooling in
the same part — not later. "We will add reporting when it becomes a problem" is
how a problem becomes unmanageable.

## 1. Checkpoint

```bash
git checkout v0.12-discovery
```

## 2. What will work by the end

```bash
# Paragraph-anchored comment on a chapter
curl -X POST localhost:8080/api/v1/works/$WORK/comments \
  -d '{"body":"this line is perfect","anchor":{"kind":"paragraph","chapter":"…","value":12}}'

# A forum topic with posts, a group with membership, and a notification.
curl -X POST localhost:8080/api/v1/forums/$F/topics -d '{"title":"…","body":"…"}'
curl -X POST localhost:8080/api/v1/groups -d '{"name":"…"}'
curl localhost:8080/api/v1/notifications
```

## 3. Concepts

- **An anchor is a pointer into a version of a thing, and it can rot.** Design
  for the case where the paragraph no longer exists.
- **Forums are a different shape from comments**: flat topics, ordered posts,
  per-topic subscriptions.
- **Groups are permission scopes**, not just lists of people.
- **Messaging is private and must stay out of every other system**: no counts in
  someone else's view, no payloads in webhooks, no content in admin lists.
- **Notifications are derived from the outbox**, so a notification can never
  exist for an event that did not commit.
- **Events and challenges are time-bounded**: they open, they accept entries, they
  close, and after they close nothing changes them.

## 4. Commands

```bash
lorehaven migrate        # applies 0013_community, 0014_events, 0023_notifications, 0027_comment_anchors
cargo test -p lorehaven-app --test milestone_12
cargo test -p lorehaven-app --test milestone_13
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0013_community.sql` | comments, forums, topics, posts, groups, memberships, messages, presence |
| `migrations/sqlite/0014_events.sql` | collections, challenges, requests, wishlists, writing events |
| `migrations/sqlite/0023_notifications.sql` | notifications and their read state |
| `migrations/sqlite/0027_comment_anchors.sql` | `anchor_kind`, `anchor_value`, `anchor_chapter_id` |
| `crates/domain/src/anchor.rs` | anchor validation rules |
| `crates/domain/src/community.rs` | thread rules, ordering, group permissions |
| `crates/domain/src/events.rs` | event lifecycle |
| `crates/db/src/community.rs`, `db/collaboration.rs`, `db/notifications.rs`, `db/events.rs` | storage |
| `crates/app/src/routes/community.rs`, `routes/events.rs`, `routes/notifications.rs` | the doors |
| `frontend/src/routes/Community.svelte`, `ForumCategory.svelte`, `ForumTopic.svelte`, `Notifications.svelte` | the pages |

## 6. The code that matters

### Anchors, and their two failure modes

```rust
// crates/domain/src/anchor.rs
// paragraph anchor: a chapter id and a non-negative integer
// timestamp anchor: HH:MM:SS(.fff), no chapter
```

The rules exist to stop the two failures anchors always have:

1. **A paragraph anchor without a chapter** is meaningless — paragraph twelve of
   what? Make it invalid rather than defaulting to chapter one.
2. **A timestamp anchor with a chapter id** is a contradiction; a timestamp is
   inside a time-based medium, a chapter is inside a text one.

And then the honest part: an anchor can point at a paragraph that no longer
exists, because the author edited the chapter. Decide the behaviour and write it
in the UI:

```text
anchor resolves   → show the comment inline with the paragraph
anchor does not   → show the comment with "on a paragraph that is no longer here"
```

Never silently drop the comment, and never silently re-attach it to whatever is
now at that offset.

### The comment door, end to end

Every comment post does five things in one transaction:

```text
1. validate the anchor (domain rule)
2. insert the comment
3. classify it (Part 6) and store the outcome
4. write the outbox event (Part 5)
5. side effects — notifications, counters — derive from the outbox, not here
```

If step 3 or 4 is outside the transaction, you get a visible hostile comment, or
a notification for a comment that does not exist. Both are bugs you will only see
under load.

### Forum topics versus comments

They look similar and should not share code:

| | comments | forum posts |
|---|---|---|
| ordering | by anchor, then time | strictly by time |
| editing | allowed, with history | allowed, marked as edited |
| nesting | none | none (flat, deliberately) |
| subscriptions | per work | per topic |
| moderation | per work, by the author | per board, by moderators |

Flat forums are a decision worth making explicitly: threading produces the
worst-behaved comment sections on the internet, and you are already building a
filter for a reason.

### Messaging privacy rules, as code

- A message row has a sender and a recipient pseud. There is no "read by admin".
- An administrator tool that lists messages must be audited and must show
  metadata only (who, when, how many) — never bodies.
- Blocking applies to messages **silently**: the sender is not told.
- Deleting a message deletes it for both parties; there is no "delete for me"
  state that the other party can still quote from the database.

### Notifications from the outbox

```text
outbox event "comment.delivered" → notification rows for subscribers
```

Because the event is written in the same transaction as the comment, a
notification cannot exist for a comment that rolled back. Delivery to email or
push is a job (Part 5) with retries, and each channel's failures are visible to
the person who configured it.

### Events and challenges

```text
state: draft → open → closed → archived
entries are appended while open; after closed, no writes are accepted
```

The rule that matters: after a challenge closes, the entry list is frozen.
Allowing late edits makes every result disputable.

## 7. Tests

`milestone_12.rs` (community):

- a paragraph anchor without a chapter, and a timestamp anchor with one, are both
  400s;
- an anchored comment appears in the chapter's comment list in offset order;
- a comment whose anchor no longer resolves still appears, flagged, and is not
  silently moved;
- a blocked pseud cannot comment on the blocker's work, and the refusal does not
  reveal the block;
- a group's private content is 404 to a non-member;
- a message body never appears in any notification, webhook or admin payload.

`milestone_13.rs` (events):

- an entry submitted after a challenge closes is refused;
- closing is idempotent;
- a collection's ordering is stable across reads.

## 8. Expected UI behaviour

- Commenting on a paragraph highlights the paragraph while you type.
- Forum topics show their last activity honestly, and nothing is "pinned" without
  a moderator action.
- Notifications clear individually and in bulk, and a cleared one stays cleared.
- A direct message thread loads in order and marks read once.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Anchor drifts after editing a chapter | anchoring by page position, not a paragraph index | anchor on the chapter's paragraph model |
| Comment appears twice | client retry without an idempotency key | accept a client token, unique per (comment, token) |
| Notification for a deleted comment | notifications built after commit from a query | derive from the outbox event |
| Forum topic list is slow | unindexed `ORDER BY last_post_at` | index on `(board_id, last_post_at)` |
| A block leaks through messaging | the send door checks the sender's blocks only | check both directions |

## 10. Consequences

- **You now hold private conversations.** Encryption at rest, retention limits,
  and an explicit policy for law-enforcement requests are the cost of this part.
  If you are not prepared to write that policy, do not ship messaging.
- **Presence is location data.** "Online now" tells anyone when a person is at
  their keyboard. Default it off, and never show it on a profile the user did not
  enable.
- **Every social surface multiplies the filter's importance.** Part 6 protects a
  writer from a comment section; a group chat needs the same protection, plus
  moderators who are accountable (Part 11).

## 11. Checkpoint

```bash
git tag v0.13-community
```

Verified by `milestone_12.rs` and `milestone_13.rs` plus a browser pass with two
accounts: comment with an anchor, edit the chapter, reload, and confirm the
comment is flagged rather than moved.

# Part 11 — Trust, governance, credits and fair queues

Checkpoint: `v0.13-community`

Moderation is a power. This part builds it with the two properties that make
power survivable: **every action is visible to the person it was taken against**,
and **every action has a way to be questioned**.

## 1. Checkpoint

```bash
git checkout v0.13-community
```

## 2. What will work by the end

```bash
curl -X POST localhost:8080/api/v1/reports -d '{"subject":"…","reason":"harassment"}'
curl localhost:8080/api/v1/reports/$REPORT             # state, quorum, votes
curl -X POST localhost:8080/api/v1/sanctions/$S/appeal -d '{"reason":"…"}'

curl localhost:8080/api/v1/me/credits                  # balance and the ledger behind it
curl -X POST localhost:8080/api/v1/bounties -d '{"work":"…","amount":500}'
```

## 3. Concepts

- **A report is a case, not a flag.** It has a state, a quorum, votes, and an
  outcome that the reporter and the reported both receive.
- **Quorum, not a single judge.** Decisions made by a small panel with a written
  threshold are reproducible and contestable; one moderator's mood is neither.
- **A sanction names what it forbids, for how long, and why.** "Banned" is not a
  sanction; "cannot post comments for 7 days, because of these two reports" is.
- **Appeals are a first-class path**, with their own state machine, and they can
  succeed.
- **Credits are a ledger, never a mutable balance.** Every number is the sum of
  entries, and every entry has a reason.
- **A fair queue spends a budget, not a position.** "Your bounties get a share of
  the instance's attention proportional to your contribution, and no single
  contributor can starve the rest."

## 4. Commands

```bash
lorehaven migrate        # applies 0016_governance and 0017_economy
cargo test -p lorehaven-app --test milestone_14
cargo test -p lorehaven-app --test milestone_15
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0016_governance.sql` | reports, cases, votes, sanctions, appeals, process feedback |
| `migrations/sqlite/0017_economy.sql` | credit ledger, bounties, fair queues, billing |
| `crates/domain/src/governance.rs` | the case state machine, quorum maths, sanction terms |
| `crates/domain/src/economy.rs` | ledger rules, bounty escrow |
| `crates/domain/src/charging.rs` | what an action costs |
| `crates/domain/src/fairqueue.rs` | the fair-share ordering |
| `crates/db/src/governance.rs`, `db/economy.rs` | storage |
| `crates/app/src/routes/governance.rs`, `routes/economy.rs` | the doors |
| `crates/app/src/routes/admin.rs` | the operator's audited tools |

## 6. The code that matters

### The case state machine

```text
reported → triaged → under_review → decided → appealed → closed
                           │                      │
                           └────── dismissed ─────┘
```

Write this as one enum with one `transition` function, and make every write go
through it. State machines implemented as ad-hoc `UPDATE … SET state=…` calls in
four handlers are how a case ends up `decided` with no votes.

Every transition writes:

```text
actor (which account, which pseud), at, from, to, and the reason for the change
```

That audit row is what makes the next section possible.

### Quorum arithmetic, in the domain crate

```rust
// crates/domain/src/governance.rs — pure, unit-tested arithmetic
pub fn decide(votes: &[Vote], threshold: Threshold) -> Outcome
```

Put the arithmetic where it can be tested without a database: how many votes
count, what happens on a tie, what happens when a voter is the reported party or
the reporter (they may not vote — enforce it in the domain, not the handler),
what happens when the review window expires with too few votes (the case closes
`dismissed`, not "open forever").

### Sanctions that expire correctly

```sql
sanctions (id, subject_pseud_id, kind, scope, reason, case_id,
           starts_at, expires_at, lifted_at, lifted_by, lifted_reason)
```

Two rules learned the hard way in every moderation system:

- **Check the expiry on read**, not only with a sweep. A sweep that has not run
  yet must not keep someone silenced.
- **A lifted sanction stays in the table.** "This was applied and removed" is
  information the sanctioned person is entitled to see, and its absence makes
  appeals unanswerable.

### The credit ledger

```text
credit_entries (id, account_id, delta, reason, subject, at, idempotency_key)
balance = SUM(delta)   -- never a stored column
```

The idempotency key is what stops a retried payment webhook from doubling a
balance. And the balance is a sum, always: a stored `balance` column is a number
that will disagree with the ledger the first time a transaction half-fails.

Bounties escrow: the amount leaves the poster's balance when the bounty is
created, and is paid to the claimer when the work is accepted — or returned if
the bounty expires. Both legs are entries, and the escrow is a distinct account,
not a special case inside the code.

### Fair queues: a share, not a rank

```rust
// crates/domain/src/fairqueue.rs
// ordering by (contribution_share, last_served_at, submitted_at)
```

The property to test for is the one that matters: **one contributor submitting a
hundred jobs does not delay another contributor's first job indefinitely.** Write
a test that submits one hundred jobs from A and one from B, and asserts B's job
is served within a bounded number of turns.

## 7. Tests

`milestone_14.rs` (governance):

- a case cannot be decided without quorum;
- the reporter and the reported may not vote;
- a tie resolves to the documented outcome (whatever you chose — assert it);
- a sanction that has expired is not enforced even before the sweep runs;
- an appeal can overturn a decision, and the audit trail shows both;
- a report from a blocked account is refused without telling the reporter why;
- every governance action is visible to its subject: `GET /sanctions/me` shows
  the reason and the evidence summary.

`milestone_15.rs` (economy):

- a credit balance always equals the sum of its entries;
- a repeated payment callback with the same idempotency key credits once;
- an expired bounty returns the escrow exactly once;
- a fair queue gives a new contributor a turn within a bounded number of jobs.

## 8. Expected UI behaviour

- Reporting asks for a reason and shows what happens next, and who will see it.
- A case has a visible state and history.
- A sanction appears in the sanctioned account's own view with its reason and
  expiry, and a button to appeal.
- Credits show a ledger, not a mystery number.
- No operator tool lets anyone move credits or lift a sanction without leaving an
  audit row.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Balance disagrees with the ledger | a stored balance column | compute from entries; store nothing |
| Cases stuck `under_review` | no expiry path | a maintenance step that closes expired windows as `dismissed` |
| A lifted sanction still blocks | the check reads a cached flag | read the row, check `expires_at` and `lifted_at` together |
| Bounty paid twice | two acceptance paths | one transition, with an idempotency key |
| A moderator's action is invisible | the audit row was written only on some paths | write it in the single transition function |

## 10. Consequences

- **Moderation data is the most legally sensitive data you hold.** It names
  people, their alleged behaviour, and who judged them. Retention, access
  control and safe deletion all need deciding here — not after the first request
  for it.
- **A sanction is a promise of proportionality.** If the instance cannot explain
  a sanction to its subject, it should not apply it.
- **Credits with real money attached become a payment system** — with the
  accounting, tax and reporting obligations that follow. Keep the ledger clean
  enough that an accountant can read it, and separate "credits" from "purchases"
  in the same way you separated account from pseud.

## 11. Checkpoint

```bash
git tag v0.15-governance
```

Verified by `milestone_14.rs` and `milestone_15.rs` on both dialects, plus a walk
through a full case by hand with three accounts: report, triage, vote, decide,
sanction, appeal, overturn.

# Part 12 — Marketplace, extensions, webhooks and themes

Checkpoint: `v0.15-governance`

Extensions are the only part of this project that runs someone else's code
against your data. Everything else can be reasoned about; this part needs
mechanisms, not intentions.

## 1. Checkpoint

```bash
git checkout v0.15-governance
```

## 2. What will work by the end

```bash
curl localhost:8080/api/v1/extensions                       # the gallery, with versions
curl -X POST localhost:8080/api/v1/extensions/$SLUG/grant -d '{"version":"1.2.0","capabilities":["read:works"]}'
curl localhost:8080/api/v1/me/extension-grants               # what I have installed, at which version
curl -X POST localhost:8080/api/v1/extensions/$SLUG/revoke
curl -X POST localhost:8080/api/v1/me/webhooks -d '{"url":"https://…","events":["work.published"]}'
```

## 3. Concepts

- **A manifest declares; a grant confines.** The extension asks for capabilities;
  the user grants a subset; the host enforces the granted set on every call.
- **Capabilities are the entire security model.** If a capability cannot be
  checked mechanically at the boundary, it is not a capability — it is a comment.
- **Installation is per account, one per package, version-pinned.** Reinstalling
  the same version is idempotent; changing the version is a decision the user
  makes.
- **Forking does not bypass review and does not copy grants.** A fork is a new
  package with its own review and its own permission prompts.
- **Webhooks are outbound data flow**, and everything privacy-classified is
  therefore a webhook payload question.
- **Themes are CSS with no network and no ability to hide safety controls.**

## 4. Commands

```bash
lorehaven migrate        # applies 0018_marketplace
cargo test -p lorehaven-app --test milestone_16
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0018_marketplace.sql` | listings, commissions, manifests, grants, webhooks, gallery items |
| `crates/domain/src/extension.rs` | `Capability`, manifest shape, `grant_is_subset` |
| `crates/domain/src/caps.rs` | capability configuration and the nesting rules |
| `crates/domain/src/marketplace.rs` | listing and commission rules |
| `crates/domain/src/webhook.rs` | signing, retry policy, event vocabulary |
| `crates/db/src/marketplace.rs` | manifests, grants, listings, gallery |
| `crates/app/src/routes/marketplace.rs` | the doors |
| `crates/app/src/worker.rs` | the webhook delivery handler |

## 6. The code that matters

### Capabilities, and the subset rule

```rust
// crates/domain/src/extension.rs
pub fn grant_is_subset(manifest_caps: &[Capability], grant_caps: &[Capability]) -> bool
```

The grant must be a subset of what the manifest declared. That one function is
what stops an upgrade from silently gaining a permission: if version 1.2 declares
`read:drafts` and the user granted only `read:works` in 1.1, the new grant is not
a subset, so the extension keeps running at the old version and the user is asked.

Enforcement happens **at the boundary**, not inside the extension: the host
answers the extension's API calls and checks the granted set on each one. An
extension that never calls the host cannot reach data by any other route,
because it has no other route — it has no filesystem, no network, no database
handle. If your extension mechanism gives it any of those, the capability model
is decoration.

### Installation: one row per (account, package), version pinned

```sql
extension_grants (account, manifest_id, version, capabilities, granted_at, revoked_at,
                  PRIMARY KEY (account, manifest_id))
```

The primary key *is* the "one active installation per package per account" rule;
you get it from the schema rather than from a check. The interesting parts:

- **Idempotent install**: installing 1.2.0 twice leaves one row, unchanged.
- **Version pinning**: an auto-update policy is a column (`pin` | `latest` |
  `latest_minor`), and a pinned installation does not move when a new version is
  approved.
- **Revocation stops execution immediately**: the grant row's `revoked_at` is
  checked on every call. "Revoked packages stop running" is an acceptance
  criterion, and it means *now*, not on the next restart.
- **A revoke is not a delete**: the row stays, so the user can see what was
  installed and when it stopped.

### Install counts with documented semantics

```text
installs = COUNT(DISTINCT account) WHERE revoked_at IS NULL
```

Say in the UI what that means. Reinstalling must not inflate it (the primary key
makes that impossible), and uninstalls must decrement it (the revocation
timestamp does that). Never expose who installed what — an install list is a
behavioural profile of every user of the gallery.

### The review workflow

```text
submitted → pending → approved | rejected | revoked
```

- A manifest is stored **versioned**, so an approved 1.1 does not silently become
  1.2.
- The document is hashed, and the hash is what reviewers approved. A change to
  the stored document invalidates the approval.
- Revocation is retroactive for future calls and never rewrites history: the
  version that was approved at the time it was approved stays visible.

### Webhooks: signing, retry, and the privacy rule

```text
signature = HMAC-SHA256(secret, timestamp + "." + body)   # timestamp in the signed content
```

- **Rotating secrets**: two secrets valid at once during rotation, so a receiver
  is never broken by the change.
- **Retry with backoff, dead-letter after bounded attempts**, and auto-disable a
  subscription after repeated failures — with an email or an in-app notice to the
  owner, because a silently disabled integration is worse than a broken one.
- **The privacy rule is absolute**: a payload carries the same privacy
  classification as the API response for that data. No destructive comment
  content, no source credentials, no pseud linkage, no reading history. Write
  the event vocabulary as a table with the classification beside each event, and
  review it whenever an event is added.
- **Rate limits per subscription**, keyed by the subscriber, so one busy
  integration cannot starve the others.

### Themes: sandboxed CSS

```text
allowed:  inline resources, data: URIs, tokens and layout slots
refused:  url() to external origins
          @import from untrusted sources
          attribute selectors combined with external URLs  (exfiltration)
          javascript: in any declaration, expression(), -moz-binding
          anything that hides the safety controls, the filter status,
          the feedback mechanism or the extension manager
```

The last line is not a style rule, it is a security rule: a theme that can hide
"this instance filters feedback" or the "revoke extension" button has escalated
itself. Mark those elements with a data attribute the theme engine refuses to
style-away, and validate submitted CSS against the rule set before storing it.

### Gallery items

```text
gallery_items (id, work_id, owner, media_type, storage_key)
```

Serve them **only** through presigned, expiring URLs scoped to the object, never
by a filesystem path. The gallery is where user-uploaded media meets the public
internet; treat every file as hostile (Part 14 covers the scanning).

## 7. Tests

`milestone_16.rs` asserts:

- a grant that is not a subset of the manifest is refused;
- installing the same version twice leaves exactly one grant;
- a pinned installation does not move when a newer version is approved;
- a revoked grant stops calls immediately — the next call fails, with no restart;
- an extension cannot call a capability it was not granted, and the refusal names
  the capability;
- a fork of a package has no grants of its own and cannot read the original's
  users;
- a webhook signature verifies with the rotating pair, and an old timestamp is
  refused;
- a webhook payload for a private event contains no private field;
- a subscription that fails repeatedly is disabled and its owner is told.

## 8. Expected UI behaviour

- Installing shows exactly which capabilities are being granted, in plain words.
- An upgrade that needs a new permission asks, and does not install silently.
- Revoking removes the extension's effect immediately.
- The gallery shows an install count with its meaning, never a list of users.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| An extension keeps working after revocation | grants cached in memory | check the grant on every call, or version the cache and invalidate on revoke |
| Install count rises on every visit | counting install events, not distinct accounts | `COUNT(DISTINCT account)` over live grants |
| Webhook delivers twice | retry without an idempotency key | include the event id; receivers dedupe on it |
| A theme hides the safety notice | the protected elements are styled by class | protect by attribute, and refuse CSS that targets it |
| A fork inherits permissions | grants copied at fork time | a fork gets none; it asks again |

## 10. Consequences

- **The gallery is a distribution channel for code you did not write.** Take the
  review workflow seriously, and be prepared to revoke in a hurry — which means
  revocation must be immediate and auditable.
- **Webhooks are data leaving your instance.** Once a payload is delivered, you
  cannot recall it. Classify every field, and test the classification.
- **Themes are a phishing surface.** A convincing theme can imitate your login
  form. Refuse external resources and, if you allow custom markup, never allow a
  form.

## 11. Checkpoint

```bash
git tag v0.17-marketplace
```

Verified by `milestone_16.rs` plus a manual pass: install an extension, revoke it,
watch a call fail immediately, and deliver a webhook to a receiver that verifies
the signature.

# Part 13 — The generalized media platform: editions, derivatives, lending and narration

Checkpoint: `v0.17-marketplace`

Lorehaven is not a text site with attachments. A work can have editions in
several media; media can be derived from each other; a library can lend; a story
can be read aloud. This part is where that generalization is paid for, and it is
the longest part of the book for a reason: it is where the interesting bugs live.

## 1. Checkpoint

```bash
git checkout v0.17-marketplace
```

## 2. What will work by the end

```bash
curl localhost:8080/api/v1/media/$EDITION                # an edition, anonymous if it is public
curl 'localhost:8080/api/v1/canons/$CANON/media?limit=20&cursor=…'
curl -X POST localhost:8080/api/v1/editions/$EDITION/derivatives -d '{"kind":"ocr_text"}'
curl -X POST localhost:8080/api/v1/works/$WORK/loans -d '{"account":"…"}'
curl localhost:8080/api/v1/me/loans                       # active | expired | revoked
curl -X POST localhost:8080/api/v1/editions/$EDITION/narration
curl -X POST localhost:8080/api/v1/editions/$EDITION/narration/approve
```

## 3. Concepts

- **Work → edition → file.** A work is the idea; an edition is a rendering
  (text, audio, a translated text, a print layout); a file is bytes with a
  checksum.
- **A derivative is machine-made and says so.** It carries its kind, its parent
  checksum, the program that made it and the failure classification if it failed.
- **Nothing derived is published by the machine.** A derivative lands as a draft
  for a human to approve.
- **Lending is a window, not a transfer.** A loan has a window, an expiry and a
  state that is computed from those, not stored as "lent".
- **Narration is an edition, built by a worker, approved by a person.**
- **Every door applies the adult gate.** No exceptions, and anonymous callers get
  404 for anything they cannot read.

## 4. Commands

```bash
lorehaven migrate        # applies 0024, 0025, 0026, 0029, 0030, 0031, 0032, 0033
cargo test -p lorehaven-app --test milestone_22
cargo test -p lorehaven-app --test milestone_25
cargo test -p lorehaven-app --test milestone_26
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0024_media_generalization.sql` | media editions and their kinds |
| `migrations/sqlite/0025_media_files.sql` | files, checksums, media types |
| `migrations/sqlite/0026_canon_space.sql` | canon and space media, with positions |
| `migrations/sqlite/0029_lending.sql` | loans |
| `migrations/sqlite/0030_derivatives.sql` | derivatives, kinds, jobs |
| `migrations/sqlite/0031_media_edition_creators.sql` | machine-producer credits |
| `migrations/sqlite/0032_media_editions_audio_checksum.sql` | narration audio |
| `migrations/sqlite/0033_loan_expiry.sql` | `work_loans.expired_at` |
| `crates/domain/src/media.rs` | edition kinds, media type rules |
| `crates/domain/src/derivative.rs` | `DerivativeKind`, `is_machine_produced` |
| `crates/domain/src/lending.rs` | loan windows and liveness |
| `crates/db/src/media.rs` | editions and scoped media, with cursors |
| `crates/db/src/derivative.rs`, `db/lending.rs`, `db/narration.rs` | storage |
| `crates/app/src/derivative.rs` | the OCR and transcode worker |
| `crates/app/src/narration.rs` | the narration worker |
| `crates/app/src/tts.rs` | the engine trait and its implementations |
| `crates/app/src/routes/media.rs`, `routes/derivative.rs`, `routes/lending.rs`, `routes/narration.rs` | the doors |
| `frontend/src/routes/Media.svelte` | the media page |

## 6. The code that matters

### Editions, and the machine-producer credit

```rust
// crates/domain/src/derivative.rs
impl DerivativeKind {
    pub fn is_machine_produced(&self) -> bool   // OCR, transcode: not the author's text
    pub fn program(&self) -> &'static str       // "tesseract", "ffmpeg"
    pub fn output_media_type(&self) -> &'static str
    pub fn remedy(&self) -> &'static str        // what to install, in the doctor's words
}
```

Declaring the program, the media type, the remedy and the machine-producer flag
**on the kind** means the door, the worker and `doctor` cannot disagree. When an
instance is missing `tesseract`, the answer is not a generic 500: it is
`CONVERTER_UNAVAILABLE` naming the program to install, produced from the same
declaration `doctor` prints.

Two rules follow from `is_machine_produced`:

- A machine-produced edition is credited to the machine, plus the account that
  asked for it. It never appears as the author's own work.
- A machine-produced edition is never auto-published. It is a draft until a human
  approves it.

### Derivatives: the pipeline

```text
1. the door validates the kind, checks the parent blob exists (checksum), enqueues a job
2. the worker: temp dir (a Drop guard), run tesseract or ffmpeg, capture stderr
3. store the result as a blob; record checksum, size, media type on the derivative
4. classify failure: fatal (bad parent, unsupported kind, program missing)
                    | transient (timeout, disk pressure)
5. mark the derivative stale when its parent changes
```

Real details that matter:

- **Temp directories need a Drop guard.** Cleanup in the success path only is a
  disk-filling bug: the failing path is exactly the one that leaves files behind.
- **Transcode target**: MP4, H.264 video with AAC audio and `+faststart` so the
  file plays while it downloads. An audio-only derivative is still an MP4
  container if you want one code path.
- **OCR output is text/plain**, stored as a blob like anything else, so the
  reader can show it and the export can include it.
- **A failed build is recorded on the row** with its classification. A derivative
  stuck at `queued` forever is indistinguishable from one nobody asked for.
- **Staleness is not deletion.** When the parent changes, the derivative is marked
  stale and stays readable with a notice. Deleting a derivative because its parent
  moved is data loss the user did not ask for.

### Lending: a window with a state derived from it

```sql
work_loans (work_id, borrower_account_id, granted_at, expires_at, expired_at, revoked_at)
```

Two bugs worth building the tests for before you write the code:

1. **The unique constraint bites on re-borrow.** A reader whose loan expired
   still has a row; an unconditional insert fails with a unique violation and the
   reader gets a 500 on every attempt to borrow the same book again. The grant is
   an upsert that re-grants the row.
2. **`is_active()` must consider expiry.** A loan whose window closed is not
   active, no matter what state column says. Derive liveness from the timestamps
   and treat any stored status as a cache at best.

And an operational detail: the maintenance pass stamps `expired_at` when the
window closes, and the API reports `active|expired|revoked`. The stamp is for
reporting and analytics — **every read checks the timestamps**, because a sweep
that has not run yet must not hand someone a book they may no longer borrow.

### Narration: an engine behind a trait

```rust
pub trait TtsEngine {
    fn name(&self) -> &'static str;
    fn is_available(&self) -> bool;
    fn health(&self) -> EngineHealth;      // names what is missing, and the remedy
    fn synthesize(&self, text: &str, voice: &str) -> Result<Audio>;
}
```

Three implementations, and the third is the interesting one:

- `PiperEngine` — a local binary, invoked with **argv only, never a shell**, and
  a temporary file that is cleaned up.
- `SilentEngine` — a valid WAV whose duration follows the text length. It exists
  so the whole pipeline can be exercised on a machine with no synthesizer, and in
  CI. A test fixture that is a real file is worth ten mocks.
- `MissingEngine` — `health()` names the missing program and the fix, so the
  request door can refuse up front instead of queueing a job that cannot succeed.

Splicing audio is where the subtle bug is:

```rust
// concat_audio parses the RIFF chunks and rewrites the sizes.
// [a, b].concat() is not a playable file — the header still describes a.
```

Chunking the text is the other one: split on sentence boundaries first, and never
in the middle of a multi-byte character. A Chinese novel narrated with mangled
boundaries is a bug report you will not enjoy.

Narration lands as a draft edition with its `media_editions.audio_checksum` set,
and `approve_narration_edition` refuses an edition with no audio — so you cannot
publish a silent chapter by accident.

### Scoped media, and the pagination that has to be right

```text
GET /api/v1/canons/{id}/media?limit=&cursor=
cursor = "<position>|<created_at>|<id>"        limit ≤ 200
```

Same rule as Part 8: the cursor carries the whole ordering key, and
`next_cursor` is returned only for a full page. A canon is a curated, ordered
list — the ordering matters and the position is what users reorder.

### The adult gate on every door, including the new ones

```text
any door that can return media, editions, derivatives, narration
  → MaybeSession, then the visibility rule
  → explicit content: 404 for anyone not eligible
```

This is the part of the project where new doors are added fastest, and therefore
where an ungated door is most likely. Make it a checklist item in every commit:
*does this new door have a `MaybeSession` extractor and a visibility check?*
Two projects' worth of incidents fit in that question. The right pattern for a
new door is to call the same helper the work page uses — never to re-implement
the check.

### The author's own numbers

`GET /api/v1/me/dashboard` returns the acting pseud's own works and what readers
did with them. The design rules are the point of this section:

```json
{
  "works":   { "total": 3, "published": 2, "unpublished": 1, "chapters": 12, "words": 34000 },
  "readers": { "bookmarks": 8, "ratings": "fewer_than_5", "reviews": 0 },
  "privacy": { "floor": 5, "note": "reader-facing counts below the floor are reported as a band" }
}
```

- The author's **own inventory is exact**: how many works they wrote is a fact
  about them.
- **Reader-facing counts are banded** at a floor (5 in this implementation): a
  count below it is a string, never a number a client would render as an exact
  figure. One bookmark is a specific person's act; "fewer than five" is not.
- **No "held by the filter" counter.** The author's view is framed as what
  arrived. A scoreboard of what was withheld turns the filter into a grievance
  machine.
- **No reader identity** appears anywhere in the payload, and the SQL is written
  so it cannot: aggregates only, over the caller's own works.

## 7. Tests

`milestone_22.rs` (scoped media):

- a two-page cursor walk at `limit=2` returns three pages, each item once, in
  position order;
- `limit=0` and a malformed cursor are refused;
- an anonymous caller gets 404 for media on a work they cannot read.

`milestone_25.rs` (derivatives, lending, archive mode, Dublin Core):

- requesting a derivative whose program is absent is refused with the actionable
  code, and enqueues nothing;
- an OCR result is stored with its checksum and media type, and is a draft;
- a parent change marks the derivative stale without deleting it;
- re-borrowing after expiry succeeds (the upsert), and the loan list reports
  `expired` for the old one;
- a loan whose window closed is not active even before the sweep runs.

`milestone_26.rs` (narration and the adult gates):

- the narration request refuses when no engine is usable, naming the engine;
- the chunker never splits a multi-byte character, and the spliced WAV is valid
  (RIFF sizes rewritten);
- approving an edition with no audio is refused;
- an anonymous reader gets 404 — not 401 — for a narration of a work they cannot
  read;
- an age-ineligible reader gets 404 for explicit media on every door, including
  the media list, the derivative list and the audio endpoint.

## 8. Expected UI behaviour

- A work page lists its editions with their kind and language.
- Requesting OCR shows a job and, when it finishes, a draft edition with a
  "machine produced" credit.
- A borrowed work shows its remaining window, and the list keeps expired loans
  with their dates instead of hiding them.
- Narration plays in the browser once approved; before approval, only the author
  sees it.
- Media that the reader may not see is simply absent — no lock icon, no teaser.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Re-borrowing a book is a 500 | unique (work, borrower) violated by an expired row | upsert the grant |
| An expired loan still reads as active | liveness read from a stored state | derive from timestamps |
| Narration audio does not play | spliced WAV header still describes the first chunk | rewrite RIFF and `data` sizes |
| Narration fails on non-Latin text | the chunker split inside a character | split on char boundaries, sentence-first |
| Derivative stuck at `queued` | the failure was recorded nowhere | record the failure and its classification on the row |
| Temp files fill the disk | cleanup only on success | a Drop guard around the temp dir |
| A new media door leaks explicit content | the extractor was `RequireSession`, or no visibility check | `MaybeSession` + the shared helper, in the same commit as the door |

## 10. Consequences

- **Derived content is still someone's work.** OCR text and a transcode carry the
  author's rights; credit them, and keep the machine-producer marker visible.
- **Lending has a legal shape** in some jurisdictions. Decide what your instance
  claims before you enable it, and make it configurable.
- **Narration voices are people.** Some engines clone voices; if yours can, the
  consent question is yours, not the user's.
- **Banded counts protect readers, not the author's vanity.** Do not remove the
  floor to make a dashboard look fuller.

## 11. Checkpoint

```bash
git tag v0.22-media
```

Verified by `milestone_22.rs`, `milestone_25.rs` and `milestone_26.rs` on both
dialects, plus a hand pass with `ffmpeg` and `tesseract` installed: an OCR of a
scanned page, a transcode, a scratch loan cycle and a narration rendered by the
silent engine with the pipeline end to end.

# Part 14 — Integrations: translation, public API, feeds, push and federation

Checkpoint: `v0.22-media`

Everything in this part is a door between your instance and something else: a
translator, a bot, a feed reader, a phone, another instance. Each one is a place
where the privacy rules you have been applying internally have to be applied
again, to a new audience.

## 1. Checkpoint

```bash
git checkout v0.22-media
```

## 2. What will work by the end

```bash
# A translation edition, built as a job and approved by a human.
curl -X POST localhost:8080/api/v1/works/$WORK/translations -d '{"language":"fr"}'

# A scoped API token for a bot, and the bot using it.
curl -X POST localhost:8080/api/v1/me/tokens -d '{"scopes":["read:works"],"name":"my bot"}'
curl -H 'authorization: Bearer …' localhost:8080/api/v1/works/$WORK

# Feeds a reader can subscribe to.
curl localhost:8080/api/v1/feeds/pseud/ada.atom
curl localhost:8080/feeds/site.rss

# Web push, for a reader who asked for it.
curl -X POST localhost:8080/api/v1/me/push-subscriptions -d '{"endpoint":"…","keys":{…}}'
```

## 3. Concepts

- **A public API is a contract with strangers.** Version it, document it, and
  never break it in a patch release.
- **Scopes are capabilities** (Part 12's model) applied to tokens. A token that
  can read one pseud's drafts is a different object from one that can read public
  works.
- **Feeds are an export with a URL.** Everything you decided about exports
  applies: what is public, what is attributed, what is never included.
- **Push notifications carry a payload over someone else's infrastructure.** The
  payload is on a lock screen: treat every field as public.
- **Federation is a promise to a machine you do not control.** Sign what you
  send, verify what you receive, and rate-limit by origin.
- **AI features are the same privacy question with a new verb.** If text leaves
  your instance for a model, that is a disclosure — name it, scope it, and make
  it opt-in.

## 4. Commands

```bash
lorehaven migrate        # applies 0019_translation, 0020_external, 0022_spec_revision, 0023_notifications
cargo test -p lorehaven-app --test milestone_17
cargo test -p lorehaven-app --test milestone_18
cargo test -p lorehaven-app --test milestone_19
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0019_translation.sql` | translation sets, editions, providers |
| `migrations/sqlite/0020_external.sql` | tokens, scopes, feed cursors, push subscriptions, federation peers |
| `migrations/sqlite/0022_spec_revision.sql` | the spec-revision ledger |
| `migrations/sqlite/0023_notifications.sql` | notifications and their channels |
| `crates/domain/src/translation.rs` | translation state, attribution rules |
| `crates/domain/src/api_scopes.rs` | the scope vocabulary and its enforcement |
| `crates/domain/src/feeds.rs` | feed building and what may appear in one |
| `crates/db/src/translation.rs`, `db/external.rs` | storage |
| `crates/app/src/routes/translation.rs`, `routes/external.rs` | the doors |

## 6. The code that matters

### Translation is an edition, not an edit

```text
work → translation set (source language → target language)
     → translated chapters, each a new edition with its own revision
```

Rules that keep translators and authors out of conflict:

- A translation never modifies the source. It is a sibling edition.
- A translator is credited on the edition, not on the work — unless the author
  invites them as a co-author (Part 3's collaborators).
- A machine translation is **machine-produced** (Part 13's marker): visible as
  such, and not publishable without a human approving it.
- The source revision is recorded, so a translation can be marked stale when the
  original changes — the same pattern as derivatives.

### Public API tokens

```text
tokens (id, account_id, name, hashed_secret, scopes, created_at, last_used_at,
        expires_at, revoked_at)
```

- Store a **hash** of the token, never the token. It is a password.
- Scopes are checked at the boundary on every request, and the refusal names the
  missing scope so a bot developer can fix it.
- A token acts as an account; if the API needs a pseud, the caller names it
  explicitly and it is checked like any other actor.
- `last_used_at` exists so a user can see a token in use and revoke it. Show it.
- **Rate limits are per token**, not per IP — a bot on a shared host must not be
  limited by its neighbours, and must not be able to escape its own limit.

### Feeds: everything from Part 9, with a URL

```text
/feeds/site.rss                  published works, site-wide
/api/v1/feeds/pseud/{handle}.atom  a pseud's published works
```

A feed is generated from the same query the discovery page uses, filtered the
same way. Two rules:

- **Never include a draft, a private rating, or a reading event.** A feed reader
  caches; a leak in a feed is a leak you cannot withdraw.
- **Include the full content or a teaser, deliberately.** A feed that includes
  full chapter text is an export; decide whether the author wants that, and
  default to a teaser.

### Push notifications

```text
push_subscriptions (account_id, endpoint, keys, created_at, last_success_at, failures)
```

- The payload is on a lock screen. **Never put a comment body, a message, or a
  work title the reader has hidden in it.** "New comment on Salt and Iron" is
  acceptable if the reader chose it; the comment's text is not.
- Expired subscriptions (404/410 from the push service) are deleted, not retried
  forever.
- Every channel is independently switchable, and the notification centre is the
  source of truth: push is a hint, never the record.

### Federation, if you build it

```text
verify signature → check the origin's rate limit → check the actor is not blocked
→ store with its origin → never trust a remote id as a local one
```

Three hard rules: sign what you send; verify what you receive before you act on
it; and treat a remote identity as a *claim* that must line up with the actor's
key every time it is used, not as a string you store once and trust.

### AI features

If your instance can call a model — to summarize, to suggest tags, to classify —
then:

- the text sent leaves your instance: say so at the point of use, per request,
  not in a policy nobody reads;
- an author's draft is never sent without an explicit action;
- a model's output that affects what other people see (tags, translations,
  classifications) is **marked as model-produced** and reviewable;
- and the classification outcomes from Part 6 are never rewritten by a model
  without a human.

## 7. Tests

`milestone_17.rs` (translation):

- translating a work creates a separate edition and leaves the source untouched;
- a machine translation is marked machine-produced and cannot be published
  without approval;
- changing the source marks the translation stale without deleting it;
- a translator without a collaboration grant cannot edit the source work.

`milestone_18.rs` / `milestone_19.rs` (external surfaces):

- a token without the required scope is refused with the scope named;
- a revoked or expired token is refused, and `last_used_at` stops moving;
- a feed contains only published works, and never a private rating or a reading
  event;
- a feed for a blocked party returns nothing to the blocker;
- a push payload contains no message body and no comment text;
- an unsigned federated delivery is refused, and a replayed one is refused too.

## 8. Expected UI behaviour

- Token creation shows the scopes in plain words, and the token exactly once.
- Settings show each channel (in-app, email, push) separately.
- A pseud's feed URL is discoverable from their profile page.
- A machine translation is labelled as one, everywhere it appears.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| A feed leaks a draft | the feed query did not reuse the visibility rule | one query, one rule |
| Push retried forever | expired endpoints treated as transient | delete on 404/410 |
| A bot is rate-limited by another bot | limits keyed by IP | key by token |
| A translation edit changed the source | the translator edited the wrong edition | separate editions, separate permission checks |
| Model output published silently | the machine marker was applied to media only | apply it to any model-produced text |

## 10. Consequences

- **Every surface here is a permanent disclosure.** Feeds are cached in clients
  you cannot reach; pushes sit on lock screens; federated copies are copies.
  Classify before you ship, not after.
- **Tokens outlive sessions.** Provide a list with last use and a revoke, or
  users will have no way to close a door they opened.
- **Federation makes you part of a network's moderation problem.** Decide your
  block list and your defederation policy before you federate, not during the
  first incident.

## 11. Checkpoint

```bash
git tag v0.19-integrations-ai-search
```

Verified by `milestone_17.rs`, `milestone_18.rs` and `milestone_19.rs`, plus a
hand pass: subscribe to a feed in a real reader, install a push subscription in
the browser, and call the API with two tokens that have different scopes.

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

# Appendix — The patterns you will use in every part

This appendix is the part of the book to keep open while you work. Each pattern
here was learned by making the mistake once; they are collected so you do not
have to.

---

## A. Two dialects, one codebase

Every migration exists twice:

```text
migrations/sqlite/0033_loan_expiry.sql
migrations/postgres/0033_loan_expiry.sql
```

The same number, the same intent, dialect-appropriate SQL. The runner applies
whichever directory matches the connected database, and the ledger records how
many have been applied so `doctor` can tell you what is pending.

Rules:

- **Never add a migration to one directory only.** The failure mode is silent:
  SQLite works, PostgreSQL breaks in production, and the person who finds out is
  not you.
- **Queries carry both forms when they differ**, and the difference is explicit:

```rust
let row = match db.backend() {
    Backend::Sqlite   => sqlx::query(SQLITE_SQL).bind(&id).fetch_one(pool).await?,
    Backend::Postgres => sqlx::query(POSTGRES_SQL).bind(&id).fetch_one(pool).await?,
};
```

- **The differences you will actually meet**:

| Thing | SQLite | PostgreSQL |
|---|---|---|
| UUID comparison with a bound string | `?` | `?::uuid` |
| Booleans | `INTEGER` 0/1 | `BOOLEAN` |
| Timestamps | `TEXT` (RFC 3339) | `TIMESTAMPTZ` — bind strings as `?::timestamptz` |
| `RETURNING` | supported | supported |
| `FOR UPDATE SKIP LOCKED` | not available | use it for queue claims |
| Auto-increment | `INTEGER PRIMARY KEY` | `GENERATED … AS IDENTITY` |

- **Test on both, or the second one does not exist.** Set
  `LOREHAVEN_TEST_PG_URL` and the same suite runs against PostgreSQL:

```bash
cargo test -p lorehaven-app --test milestone_25                       # SQLite
LOREHAVEN_TEST_PG_URL=postgres://user:pw@127.0.0.1:5432/postgres \
  cargo test -p lorehaven-app --test milestone_25                     # PostgreSQL
```

---

## B. The test harness

Every integration test builds the real router and sends real requests:

```rust
let dir = scratch_dir("shelf-import");
let tdb = test_support::TestDb::connect_with_dir("shelf-import", &dir).await;
let app = server::build_router(AppState::new(config_for(&dir), tdb.db().clone()));
let mut client = Client::new(app);
register(&mut client, "reader@example.com", "reader").await;
```

Four rules:

1. **Nothing is mocked.** The point of the suite is to prove the vertical slice
   works end to end: real router, real database file, real request objects.
2. **One scratch directory per test**, named by process and thread, so parallel
   tests never share a database.
3. **Guard process-global state.** Tests that change `PATH` (to exercise the
   converter discovery with and without a program present) must serialize on an
   async `Mutex`. A `PATH` race produces a failure that appears only when two
   tests happen to run at once.
4. **Name the file for the milestone** (`milestone_25.rs`), so the ledger in
   `docs/requirements.csv` can point at a real test by name.

Filter the noise when you run them:

```bash
unset LOREHAVEN_TEST_PG_URL
CARGO_TARGET_DIR=~/.cargo-target/lorehaven cargo test -p lorehaven-app --test milestone_26
```

---

## C. The door checklist

Every new route answers five questions before you write it:

| Question | Answer lives in |
|---|---|
| Who may call it? | `RequireSession` / `RequirePseud` / `MaybeSession` |
| May they read *this object*? | the shared visibility helper (never a new check) |
| What does it return when they may not? | **404** if they may not know it exists; 403 only if they may know and may not act |
| Does it need a CSRF token? | any state-changing cookie-authenticated route: yes |
| Is it rate limited? | anything that costs money, sends messages, or can be spammed: yes |

The two rules that catch the most real bugs:

- **Public objects must be reachable without an account** (`MaybeSession`). A
  door that requires a session for public content breaks every shared link.
- **Explicit content is 404 on every door**, computed from the caller's age
  state, in the query as well as the handler. "I added the gate to the detail
  page" is how a list page leaks a title.

---

## D. The error taxonomy

One type, one place, stable codes:

```rust
pub enum AppError {
    Validation { message, field_errors },   // 422 — the caller can fix it
    NotFound,                               // 404 — may not know it exists
    Unauthorized,                           // 401 — no session
    Forbidden,                              // 403 — session, but not allowed
    Conflict { … },                         // 409 — stale revision, duplicate key
    ConverterUnavailable { program },       // 422 — the instance cannot do this
    Internal(anyhow::Error),                // 500 — masked in the response, full in the log
    …
}
```

- The response body is always `{"error":{code,message,field_errors,request_id}}`.
- `is_fault()` splits logging: refusals at `debug`, faults at `error`.
- `Display` is masked; only `Debug` reaches the log, so internal detail never
  leaks to a client.
- **A new failure gets a code, not a string.** Codes are what clients switch on,
  and what your tests assert.

---

## E. Pagination, once and for all

```rust
// every list endpoint
let limit = parse_limit(raw_limit, DEFAULT, MAX)?;    // clamp, never fail
let cursor = decode(raw_cursor)?;                     // error on malformed
// ORDER BY <full ordering key>, e.g. (position, created_at, id)
let items = query(...limit + 1...).await?;            // fetch one extra
let next = if items.len() > limit { Some(cursor_of(&items[limit - 1])) } else { None };
```

- The cursor carries **the whole ordering key**, exactly what `ORDER BY` compares.
- The final `id` tiebreaker is not optional.
- `next_cursor` only for a full page, or every client makes one extra request
  forever.
- The test is a **multi-page walk**, asserting each row appears exactly once, in
  order.

---

## F. The discipline of honest status

Three documents, each with one job:

| Document | Answers |
|---|---|
| `docs/spec.md` | what the site is supposed to be |
| `docs/requirements.csv` | every requirement, its status, and the evidence |
| `docs/verification.md` | literal output proving the claims, newest first |

Statuses are not decoration. Use the honest set: `implemented-locally-tested`,
`partially-implemented`, `skeleton`, `not-implemented`, `verified`. "Done" is not
a status, because it does not say how you know.

When you finish a piece of work, paste the actual output:

```text
cargo test -p lorehaven-app --test milestone_24
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.11s
```

Not "tests pass". Six months later, the literal line tells you which suite, how
many, and whether it was the same suite you meant.

---

## G. Commit and gate discipline

- **Run the gates once, at the end of a change**: `cargo fmt`, `cargo clippy
  --workspace --all-targets -- -D warnings`, then the workspace suite. Running
  them after every file wastes your afternoon and teaches you nothing new.
- **Narrow checks while you work**: one test binary, one crate. Fast feedback for
  the thing you are actually editing.
- **Clear the warnings that were already there.** A red gate that predates your
  work is still in the tree you are touching; leaving it means the next person
  cannot tell your failures from the old ones.
- **Capture the stream the tool writes to, or the gate is theatre.** rustc,
  clippy and `cargo test` report warnings and errors on **stderr**. A check
  written as `cargo clippy ... 2>/dev/null | grep -c warning` returns a confident
  zero from an empty stdout while the tree is full of warnings — and it will keep
  returning zero, which is worse than a red gate, because it looks like evidence.
  Redirect to a file and read the file: `... > /tmp/gate.log 2>&1`. The corollary
  is the rule that actually saves you: **if a gate has never once failed, suspect
  the gate before you trust it.** A check that has been green for every session is
  not proof of a clean tree; it is the shape a broken check takes.
- **An E2E test that asserts an empty state or a global count is testing the
  fixture, not the feature.** Shared test accounts accumulate state between
  tests, so "the list is now empty" can never hold on the second run. Assert the
  specific thing you changed: capture a stable per-row handle (an id in an href,
  an `aria-label`) before acting, then assert that handle is gone and the count
  fell by exactly one. And match rows on ids, not on user-facing text — UI
  labels often fall back to a generic value.
- **Commit per coherent piece**, with a message that says what changed and why.
  The history is part of the documentation, and it is the only part nobody
  maintains — so write it as if it will be read, because it will be.

---

## H. Debugging order

When something does not work, in this order:

1. `lorehaven doctor` — the environment is the most common cause and the
   cheapest to check.
2. The literal response: `curl -i` the door, read the code and the request id.
3. The log line for that request id — every request carries one.
4. The database, by hand. The schema is the truth; the code is an opinion about
   it.
5. A test that reproduces it over the real router. If you cannot write it, you
   do not yet understand the bug.

And when a fix works but you do not know why: keep looking. A fix you cannot
explain is a bug that has moved.

---

## I. Where the time actually goes

In rough order, from this project's experience:

1. **Dialect parity** — a query that works on SQLite and not PostgreSQL.
2. **Visibility and gates** — a door that forgot one, most often a list.
3. **Expiry and liveness** — a stored flag that disagrees with a timestamp.
4. **Pagination** — the silent `LIMIT`, the non-unique cursor.
5. **Idempotency** — a retry that doubles something, in jobs, credits, webhooks,
   imports.
6. **Boundaries in text** — chunking, escaping, anchoring into an edited
   document.

None of these are exotic. All of them are cheap to prevent with one rule and one
test, and expensive to find in production. That is the whole argument for the way
this book is written: build the rule with the feature, and prove it with a test
that sends a real request.
