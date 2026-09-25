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
