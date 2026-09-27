# M54 — the bot core and its link flow

Status: **the build plan for the two M54 rows**, written 2026-09-27 immediately
before starting, per the standing workflow (spec + plan before code). Authority:
spec §23.2 (amended, ADR 0024), `docs/goal.md` order-of-work step 2, and the
two rows M54-01/M54-02 in `docs/requirements.csv`.

`docs/plans/bots-extension.md` says this is "proposed, not adopted" and to
confirm with the user first. That referred to the extension and to the
desktop app; the bots themselves were **already adopted into the spec** by
ADR 0024, which `docs/spec.md` §23.2 carries verbatim. This plan covers only
what the spec adopted. The extension stays unspecced and unbuilt.

## What M54 is, and what it is not

M54-01: port the platform-neutral core of `~/code/rust/fanfic-archivist-bot`
against `/api/v1`. M54-02: the link flow — short-lived challenge, Lorehaven-side
pseud and scope confirmation, revocable tokens.

Three constraints are not negotiable, and each has a specific way this build
can silently violate it:

1. **The bot never touches the database.** It is a separate process in a
   separate repo. It speaks HTTP or it does not exist. *How it can silently
   violate this:* by adding a `lorehaven-db` path dependency, or by importing
   the workspace. The gate is a `cargo tree` assertion, not a code review.
2. **Bots never receive a password.** *How it can silently violate this:* a
   link flow that accepts credentials at the bot end, or a challenge that is
   long-lived enough to be a bearer secret. The gate is a test that sends a
   password field to the exchange endpoint and reads the refusal.
3. **Tokens are revocable and scoped.** *How it can silently violate this:*
   `expires_at` exists on `api_tokens` and **nothing writes it** — see the
   defects below. A token with no expiry is a permanent bearer credential, and
   "revocable" on paper is the whole claim.

## The defects found while planning, which M54-02's exit condition depends on

These are not M54 features that are missing. They are the existing token
machinery being wrong in ways §23.2's link flow cannot be built on. Found by
reading the code, not by a test failing, which is the point: nothing here is
covered by a test today.

### D1 — `POST /api/v1/me/tokens/{id}` revokes any token, for anyone

`crates/app/src/routes/external.rs:163` takes `MaybeSession(_user)` and passes
the path id straight to `revoke_token`, which is `UPDATE api_tokens SET
revoked_at = ? WHERE id = ?` with **no account predicate**. Two consequences,
both real:

- An **anonymous** request revokes a token. The session is extracted and
  discarded.
- A **signed-in** request revokes a token belonging to a *different* account.

So the endpoint that §23.2 leans on for "revocation and unlinking" is an
unauthenticated cross-account denial of service. Fixing it is a prerequisite,
not a nicety: M54's exit condition is "revoking a token takes effect on the
next call", and an unauthenticated revoke makes that condition true for the
wrong reason.

### D2 — `api_tokens.expires_at` is never written, so no token ever expires

`issue_token` (`crates/db/src/external.rs:14`) inserts `(id, account_id, name,
token_hash, scopes, created_at)` and never mentions `expires_at`. Nothing else
writes it. `list_tokens` *reads* it and reports it. So every token issued on
every instance is permanent — which contradicts §23.2's "revocable limited
authorization" and makes D1's blast radius unbounded in time.

### D3 — `api_tokens.last_used_at` is never written either

Same shape, lower stakes: the column exists, is read by `list_tokens`, is never
maintained. A token's age is not observable, so an operator cannot tell a live
credential from an abandoned one.

### D4 — `resolve_token` does not check `expires_at`

Even once D2 is fixed, `resolve_token`'s predicate is `token_hash = ? AND
revoked_at IS NULL`. An expired token would still authenticate. This is the
same class as the `rec.mode` defect named in `docs/goal.md`: the machinery is
present and the wiring is absent, and a green suite would not see it.

### D5 — `bot_registrations.token_id` has no foreign key, and `owner` is an
account id stored in a column named for a person

`migrations/postgres/0020_external.sql:4` declares `token_id TEXT NOT NULL`
with no `REFERENCES`, and `owner TEXT NOT NULL` holding what the caller passes
as `account`. A deleted account leaves an orphan registration pointing at a
revoked token. The column names are the smaller half: `owner` reads as a
person's name to every future reader, and holds an account id.

### D6 — `MaybeToken` silently drops an unparseable scope

`crates/app/src/auth.rs:311` builds the scope list with `.filter_map(|s|
Scope::from_str(s).ok())`. A token row whose `scopes` JSON contains an unknown
scope yields that scope **dropped**, so a token granted `content.read` and
`admin.write` resolves as if it held only `content.read`. Narrowing, so not a
privilege escalation — but §0.4's rule is that an unrecognised value stops
startup, not that it silently disappears.

### D7 — the `/feeds/{handle}` handlers return a constant

`get_rss_feed` and `get_atom_feed` return `{"feed": "rss"}` and
`{"feed": "atom"}` for any handle, including one that does not exist. §23.5
requires public feeds and scoped revocable private feeds. This is exactly the
stub `docs/goal.md` forbids: "every branch returns a real value or refuses".

D7 is in §23.5, not §23.2, so it is **not** an M54 row. It is listed because it
is a stub on a surface the bot's "return a link to continue on Lorehaven" path
would want, and because leaving a known stub in place while claiming the build
is complete would be the failure mode this project has been bitten by before.

### D8 — a bot token cannot perform a single §23.2 action

This is the finding that changes M54-01's shape, so it is stated at the top.

§23.2 lists eight "supported initial actions". All eight have a real door in the
router — verified, not assumed:

| §23.2 action | door | module |
|---|---|---|
| search eligible public works | `GET /public/search` | external |
| fetch public metadata | `GET /public/works/{id}` | external |
| start an authorized private import | `POST /imports` | imports |
| check job status | `GET /jobs`, `GET /jobs/{id}` | jobs |
| request a permitted export | `POST /exports` | exports |
| save a bookmark | `POST /bookmarks` | library |
| return a link to continue on Lorehaven | — client-side, no door needed | — |
| send appreciation to authors | `POST /works/{id}/kudos` | works |

**But `MaybeToken` — the extractor that turns a bearer token into an identity —
appears in exactly one file in the whole tree: `crates/app/src/routes/media.rs`.**
`RequireToken` appears in **zero** handlers. The six write-shaped doors above
(`create_bookmark`, `toggle_kudos`, imports, jobs, exports) all take
`RequireSession`, which reads only a session cookie's pseud. So today a token
issued through `POST /me/tokens` authenticates `/media/search` and nothing else:
the eight actions are unreachable for a bot, and only the two public-read ones
work, and only because they need no identity at all.

The wiring that would have caught this is present and unused — `MaybeToken` and
`RequireToken` both exist in `crates/app/src/auth.rs`, and §23.1's scope check
is already implemented in `media.rs`. The *pattern* is there. It was applied to
one module.

One consequence makes this worse than a missing feature, and it is a security
consequence rather than a capability one. `verify_csrf`
(`crates/app/src/auth.rs:163`) skips the CSRF check when the request carries no
`SessionUser` in its extensions — which is exactly the case for a bearer-only
request. So the CSRF layer passes a token-authenticated `POST` untouched. That
is *correct* for CSRF (a bearer token is not ambient credentials) and it means
the only thing standing between a token and every write door is the per-handler
extractor that is missing. Widening `MaybeToken` to those handlers without
fixing D1 would hand a bot a valid credential for another account's tokens.

So the real Part A is larger than the token table: **six handlers must learn to
accept a token, each gated on the scope its action requires, each with the
acting pseud made explicit.**

## Build order

Dependency order, not appeal. Three examples of what breaking it costs:

- **Schema before routes before bot.** The link flow creates a challenge, so
  `link_challenges` must exist before `/api/v1/link/*`. Building the route
  first produces a handler that compiles against a table it invents.
- **D1–D4 before the bot.** The bot's exit condition is that revocation works.
  Shipping the bot against an unauthenticated revoke and a permanent token
  means the feature is demonstrably broken and the tests would pass anyway.
- **The bot core before the three adapters.** §23.2 requires Discord, Telegram
  and Matrix to pass *the same* core test suite. Writing an adapter first means
  writing the interface by its first implementation, which is how a Discord-only
  abstraction appears in a "platform-neutral" core.
- **D8 before the bot's API client, not after.** A client written against doors
  that refuse tokens produces a bot whose every non-read command fails at
  runtime. Discovering that after building three adapters means three adapters to
  re-test; discovering it first means one HTTP client contract. This is the
  clearest example of the order being forced rather than preferred.

## Part A — the token foundation (D1–D8), in the Lorehaven repo

Rows: M54-02 depends on all of Part A. Each step ends in a command and its
expected output.

### A1 — migration 0083, both dialects

New file, written twice. `migrations/sqlite/0083_token_expiry.sql` and
`migrations/postgres/0083_token_expiry.sql`.

The gate is `the_two_dialects_declare_the_same_columns_and_indexes` in
`crates/db/src/migrate.rs:601`, run by `cargo test -p lorehaven-db`. It
compares **declared table and index names, and their column sets**, across the
two files. That test is stricter than this plan first assumed, and two of its
rules shape the SQL below:

- an index must sit on **one line** — the parser scans line by line for ` ON `,
  so a wrapped index body is invisible and the index is read as undeclared;
- no `IF NOT EXISTS` prefix, because the parser takes the index name as the last
  whitespace token before ` ON` and reads `NOT` as the name.

The SQLite recreate precedent is
`migrations/sqlite/0075_fix_device_deliveries_export_job_fk.sql`: `PRAGMA
foreign_keys=OFF`, rename the table away, `CREATE` under the real name, copy,
`DROP`, `PRAGMA foreign_keys=ON`. That file is also the canonical worked example
of *why* the parity test is strict — the first attempt there declared a
`_fixed` table name the PostgreSQL twin never mentioned, and the test correctly
failed on it.

```sql
-- link_challenges (spec §23.2).
--
-- The challenge is issued by the BOT and redeemed by the BROWSER. It carries no
-- secret of the reader's: it is a public nonce whose single use is the security
-- property, and the reader's identity arrives with the confirmation, not here.
--
-- expires_at is short (default 600s, configured) precisely so a challenge
-- captured from a chat log cannot be replayed tomorrow. used_at is set exactly
-- once: the redemption is a single UPDATE ... WHERE used_at IS NULL, so two
-- browsers racing on the same challenge produce one token, not two.
--
-- `state` is a value, not an inference from `used_at`/`expires_at`, for the same
-- reason migration 0080 gave `claims` one: an expiry that nobody ever evaluated
-- is the rec.mode defect from docs/goal.md, and a column that can say
-- 'expired' while the predicate that would make it expired is never run is a
-- state no query can rely on.
CREATE TABLE link_challenges (
    code         TEXT PRIMARY KEY,     -- public nonce, high entropy
    state        TEXT NOT NULL,        -- pending|used|expired
    bot_id       TEXT NOT NULL,        -- the bot that asked for it
    requested_scopes TEXT NOT NULL,    -- what the bot asked for, to display
    pseud_id     TEXT,                 -- chosen at confirmation, NULL until then
    token_id     TEXT,                 -- the token minted, NULL until then
    created_at   TEXT NOT NULL,
    expires_at   TEXT NOT NULL,
    used_at      TEXT
);
CREATE INDEX link_challenges_pending ON link_challenges (state, expires_at);

-- api_tokens gains only what is genuinely absent.
--
-- acting_pseud_id is §23.1's "explicit acting pseud": a token belongs to an
-- ACCOUNT but acts as a PSEUD, and without this column every token call acts as
-- the account's default pseud, silently conflating two identities.
--
-- kind is read by issue_token's `_kind` parameter and then thrown away today,
-- so every token in the database is indistinguishable from a personal one
-- however it was issued.
--
-- expires_at and last_used_at are NOT added here. They already exist (0001,
-- both dialects) and are already read by list_tokens; they are simply never
-- written. Re-adding them here would have been the same defect a second time.
ALTER TABLE api_tokens ADD COLUMN acting_pseud_id TEXT;
ALTER TABLE api_tokens ADD COLUMN kind TEXT NOT NULL DEFAULT 'personal';
```

**Correction, made by reading both files before writing either:**
`expires_at` and `last_used_at` **already exist** on `api_tokens` in
`migrations/postgres/0001_identity.sql:162-163` and in the SQLite twin. So D2
and D3 are not a missing column — they are a column that is *read by
`list_tokens` and written by nothing*. The migration is correspondingly smaller
than the first draft said, and the fix for D2/D3 is in A2, not here.

Two things the parity test cannot see, verified against `declared_schema` in
`crates/db/src/migrate.rs:380`:

- it reads **`CREATE TABLE` only**, so `ALTER TABLE ... ADD COLUMN` is invisible
  to it. The new `api_tokens` columns therefore get no cross-dialect check from
  that test — the gate is the PG runtime pass instead, since a column declared
  in one dialect and not the other fails the first query that names it.
- it *does* read `CREATE TABLE` and index names, so `link_challenges` and its
  index must be declared identically in both files, with the index on one line
  and no `IF NOT EXISTS`.

`kind` is read by `issue_token`'s `_kind` parameter and then thrown away — the
parameter exists and is ignored. Moving it to a column is a one-line change to
the INSERT plus one to the signature.

On PostgreSQL, `acting_pseud_id` is a `TEXT` column holding what is a `UUID` in
`pseuds` — the same deliberate choice this schema already makes where a
TEXT-typed column references a uuid id. The gate is `RequireActor`'s lookup on
the PG pass, per the INT4-into-i64 blind spot in `docs/goal.md`.

D5 in the same migration: `bot_registrations` gets a real foreign key. On
PostgreSQL that means `ALTER TABLE bot_registrations ADD CONSTRAINT
bot_registrations_token_fk FOREIGN KEY (token_id) REFERENCES api_tokens (id)
ON DELETE CASCADE` — which the parity parser *does* read, via its
`FOREIGN KEY` / `ALTER TABLE` branch, so both files must declare it in that
form. On SQLite a bare `ALTER TABLE ADD CONSTRAINT` is not supported, so the
SQLite file **recreates** the table: `PRAGMA foreign_keys=OFF`, rename, `CREATE`
under the real name, copy, `DROP`, `PRAGMA foreign_keys=ON` — the shape of
`migrations/sqlite/0075_fix_device_deliveries_export_job_fk.sql`, and the
recreated table's columns must match PostgreSQL's exactly or the parity test
fails.

So the SQL that survives correction is: `link_challenges` plus its one-line
index, two `ALTER TABLE api_tokens ADD COLUMN` statements, and the
`bot_registrations` foreign key. The `expires_at` / `last_used_at` `ALTER`s the
first draft proposed are dropped, because those columns are already there.

### A2 — `crates/db/src/external.rs`

- `issue_token` gains `expires_at: Option<&str>` and writes it; gains `kind`
  written rather than discarded; returns the id (unchanged).
- `resolve_token` adds `AND (expires_at IS NULL OR expires_at > ?)` — D4. Note
  RFC 3339 text compares correctly as text on both backends, which is why every
  timestamp column in this schema is TEXT (ADR 0004).
- `touch_token(db, token_id)` writes `last_used_at` — D3. Called from
  `MaybeToken`, so the write happens on the path that resolves the token and
  nowhere else.
- `revoke_token(db, token_id, account: &str)` adds `AND account_id = ?` — D1.
  The handler supplies the caller's account; the predicate is in SQL so a
  caller cannot forget it.
- New: `create_link_challenge`, `redeem_link_challenge`,
  `confirm_link_challenge`, `expire_stale_challenges`.
  `redeem_link_challenge` is one `UPDATE ... SET state='used', used_at=?
  WHERE code = ? AND state='pending' AND expires_at > ?` and the caller checks
  rows-affected — the same make-it-structural-not-procedural reasoning as
  `exchange_signals`' content-hash primary key in migration 0081.
- New: `set_bot_state`, `bot_registration_for_token` — the `state` column is
  written once at insert and never again, so a suspended bot stays active.

### A3 — `crates/domain/src/api_scopes.rs`

D6. `Scope::from_str` already returns `Result`; the fix is at the call site in
`auth.rs`, not here. Add `parse_all(&[String]) -> Result<Vec<Scope>, String>` so
there is exactly one place that decides what a scope list means, and have
`MaybeToken` use it: an unparseable scope refuses the request rather than
narrowing it. Add a `all()` iterator for the link-flow UI.

### A4 — routes

`crates/app/src/routes/external.rs`:

- `revoke_token` — `RequireToken` **or** `RequireSession`, and pass the caller's
  account into `revoke_token` (D1). Refuse a token that is not the caller's
  with 404, not 403: a 403 confirms the token exists.
- `issue_token` — accept `expires_in_seconds`, refuse an unbounded value with
  the accepted range named (§0.4's rule, and the same reasoning as M52-09's
  engine choice). Write `expires_at` (D2).
- New: `POST /api/v1/link/challenge` (bot-authenticated, issues a challenge),
  `GET /api/v1/link/confirm?code=…` (session-required, shows the pseud and
  scope picker), `POST /api/v1/link/confirm` (session-required, mints the
  token, returns it **once** to the bot through the challenge redemption), and
  `POST /api/v1/link/challenge/{code}/redeem` (bot-authenticated, receives the
  token).

The redemption returns the secret exactly once, like `issue_token` does. A
challenge in `pending` returns a refusal naming that the challenge is
unredeemed; a used one returns a refusal, not the token again.

### A5 — tests

`crates/app/tests/m54_bot_link.rs`, both backends:

- an anonymous request to `/me/tokens/{id}` is refused and the token still
  resolves (D1) — this is the test that would have caught it.
- account A's token cannot be revoked by account B, and is still live (D1).
- a token with an expiry in the past does not resolve, and one with a future
  expiry does (D2, D4).
- `last_used_at` is null before use and non-null after one authenticated call
  (D3).
- the link flow end to end: bot issues a challenge, a reader confirms with a
  pseud and scopes, the bot redeems and receives a scoped token, and that token
  carries **only** the granted scopes.
- a challenge is single-use: a second redemption is refused.
- an expired challenge is refused.
- a challenge is refused if the reader tries to grant themselves
  `admin.write` without confirming it (the picker only offers the bot's
  requested scopes; the server re-checks against them).
- a password posted to any `/link/*` door is refused **by name** (constraint 2).

`crates/app/tests/milestone_18.rs` gains the D1 regression at the DB layer too,
so the predicate is pinned where it lives.

### A6 — token-aware identity for the six write doors (D8)

This is the step that makes §23.2's eight actions real, and it is the one the
existing code was already shaped for.

`crates/app/src/auth.rs` gains an extractor that resolves **either** identity —
session pseud or bearer token — into one actor, so a handler stops caring which
arrived:

- `RequireActor` — session pseud, or token account with the token's
  `acting_pseud_id` (D1's `acting_pseud_id` column from A1). Resolves the
  acting pseud *once*, and a token whose acting pseud has been deleted is a
  refusal rather than a silent fallback to the account's default pseud.
- `RequireActorScoped(Scope)` — as above, and refuses with 403 naming the
  missing scope, exactly as `media.rs` already does. One scope check, in the
  extractor, rather than a per-handler `if`.

Then six handlers switch from `RequireSession` to `RequireActorScoped` with
the scope §23.2's own security note implies:

| door | scope | reasoning |
|---|---|---|
| `POST /bookmarks` | `library.read` | a bookmark is a reader-side fact about the caller's own shelf; writing one changes nothing the author sees |
| `POST /works/{id}/kudos` | `content.read` | §0.3-class: kudos are "a statement about themselves" per the handler's own comment, and carry no content authorship |
| `POST /imports` | `content.write` | a private import is a write |
| `GET /jobs`, `GET /jobs/{id}` | `content.write` | job status is about the caller's own import/export |
| `POST /exports` | `library.read` | an export of the caller's own library; §23.2 says "a permitted export", and the permission check is already in the handler |
| `POST /imports/{id}/cancel` | `content.write` | with `POST /imports` |

Every one of those handlers must also confirm the row it is about belongs to the
actor, not merely that the actor holds the scope — a scope says what you may do,
not to what. `create_bookmark` and `toggle_kudos` are already account-scoped by
construction; the import/job/export handlers each need the ownership predicate
added, and each gets a test that account B cannot cancel A's import by id.

Verification:

```sh
cargo test -p lorehaven-app --test m54_bot_actions   # exit 0, both backends
```

Covering, per action: a bearer token with the right scope succeeds; with the
wrong scope, 403 naming it; with no token, 401; a session still works on every
one of them (the browser must not regress); and account B cannot act on account
A's import/export/job by id.

### A7 — frontend

`frontend/src/routes/Settings.svelte` gains a **Linked applications** section:
the existing `list_tokens` endpoint already backs it, so this is a real list,
a revoke button that calls the now-authorized revoke, the expiry, and the
scopes. `LinkConfirm.svelte` is the confirmation page for `GET /link/confirm`.

There is currently **no** UI for tokens at all — `grep me/tokens frontend/src`
returns nothing — which is why D1 shipped. A security control with no surface
is not one.

## Part B — the bot core (M54-01)

New repo: `~/code-local/rust/lorebot`. Separate workspace, no path dependency
into Lorehaven, no `lorehaven-db` reachable. It depends on the wire contract,
and on nothing else.

**This is the user's call to confirm before it starts**, because
`docs/goal.md` says the bot is a separate repo and a new repo is a new thing in
`~/code-local`. Recommended: a new repo at `~/code-local/rust/lorebot`, so it
sits beside `lorehaven` and `lorebook` rather than in the `~/code` mirror tree.

### B1 — the API client

`crates/lorebot-api`. Compiles against the wire types, not the server. The eight
§23.2 actions and the door each maps to are tabulated in D8 above, and Part A
makes all of them reachable with a token — so by the time this step starts,
every door the client needs is one the client can actually open.

The client is where the *scoping* decision gets tested from the other side: each
method declares the scope it needs, and the conformance suite asserts that a
token without it is refused. That is the half of §23.1's contract Lorehaven's
own tests cannot see, because they are testing the server and not a client that
depends on the refusal.

### B2 — the platform-neutral core

Port from `archivist-core`: `intent.rs` (intent classification), `core.rs`
(command dispatch), `cache.rs` (pagination cache), `store.rs` (token store),
`api.rs` (API client), `model.rs` + `lemmy.rs` (the message IR the per-platform
render matrix targets), `ratelimit.rs`, `config.rs`.

**The IR is the load-bearing port.** §23.2's "per-platform render matrix" only
works if the core speaks one message shape and each adapter renders it. The
port is to that shape, with Lorehaven's payload types replacing FicHub's.

### B3 — the three adapters

Discord, Telegram, Matrix. Each is a thin translation between the platform's
message model and the core IR, plus a gateway client. The port source has
`fanfic-archivist-{matrix,telegram}` and Discord in `fic-archivist`; the
Discord adapter's shape is the reference for the other two.

### B4 — the tests

The exit condition, verbatim: **a Discord, a Telegram and a Matrix adapter all
pass the same core test suite; revoking a token takes effect on the next call;
no code path reaches the DB.**

That means the core test suite is a trait-level conformance suite — one test
body, run three times, once per adapter — rather than three copies that drift.
The revocation test is an integration test against a real Lorehaven, because
the property under test is a server behaviour the bot merely observes.

## Definition of done

- `docs/requirements.csv`: M54-01 and M54-02 move off `planned` with evidence
  naming the test file, in the same commit as the code.
- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets` (read the
  log file, not a piped count), `cargo test --workspace --no-fail-fast` on
  **both** backends, `cargo test -p <crate> --doc` as its own command — all
  exit 0.
- The three adapters pass one conformance suite, and the DB-reachability
  assertion is a test rather than a review comment.
- `docs/handoff.md` records where the build stands, in the style of the
  existing entries, including the defects D1–D7 and what was decided.

## What this plan does not cover

- The Chromium extension — unspecced, and `docs/goal.md` puts it after the bot.
- Lorebook — a separate project that must not gain a dependency in either
  direction.
- §23.5's feeds beyond fixing the D7 stub, §23.6 ActivityPub, §23.7 AI.
- The M45 block, which `docs/goal.md` ranks after M53 and M54.
