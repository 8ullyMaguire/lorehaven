# lorebot — spec

**Row:** M54-01. **Milestone:** M54. **Status:** in progress.

A chat bot for Lorehaven on Discord, Telegram and Matrix. It is a thin REST
client of the Lorehaven public API and **never touches the Lorehaven database**.
This is the whole design constraint, and it is stated first because everything
else follows from it.

## Why a separate repo

Lorehaven is a Rust/Axum + SvelteKit application with a database, a job
runner, a media pipeline and a front end. A bot that linked against its crates
would inherit all of that and could reach the database by accident — which is
exactly the failure §23.2 forbids. So:

* No path dependency on `lorehaven-*`. No workspace inheritance.
* No database driver in the dependency tree. This is **asserted by a test**, not
  by convention, because a convention is not a property.
* The only thing shared with Lorehaven is the wire contract: JSON shapes and
  HTTP status codes.

`lorebot` lives at `~/code-local/rust/lorebot`, beside `lorehaven` and
`lorebook`, rather than in the `~/code/rust` mirror tree. That is the
home-directory convention, not a technical requirement.

## The port

Source: `~/code/rust/fanfic-archivist-bot`, whose `archivist-core` is 12k lines
across `api`, `model`, `cache`, `store`, `config`, `error`, `util`, `core`
(the IR), `dispatch`, `intent`, `lemmy`, `ratelimit`, `debug`. Its Discord
adapter is 6.5k lines.

What is ported, and what is not:

| module | ported | note |
|---|---|---|
| `core` (the IR) | **yes, and it is load-bearing** | §23.2's per-platform render matrix only works if the core speaks one message shape and each adapter renders it |
| `error` | yes | `NotLinked` and the scope-refusal variants are new |
| `api` | rewritten | FicHub's client becomes Lorehaven's `/api/v1`; this is the largest single change |
| `model` | rewritten | FicHub payload types become Lorehaven's |
| `config` | yes | environment-driven, same shape |
| `util` | yes | URL detection, word formatting, truncation |
| `ratelimit` | yes | per-platform buckets, no Redis — a bot is one process, and a shared Redis would be a dependency on infrastructure the bot does not otherwise need |
| `cache` | **yes, in-process** | the source uses Redis; see below |
| `store` | **simplified** | the source's token store is Redis + pending-link codes. A single bot process does not need Redis, and §23.2's "tokens stored securely by the bot deployment" is better served by an encrypted file than by a network round trip. Still not a database. |
| `dispatch` | yes, trimmed | the eight §23.2 actions, not the source's twelve commands |
| `intent` | **no** | the source's optional-Ollama LLM classifier. Out of scope: it is not one of the eight actions, and it is the one part of the core that would want a network dependency the bot does not need |
| `lemmy` | **no** | polls a Lemmy/Piefed instance for community posts. Not a §23.2 action. |
| `debug`, `metrics` | no | diagnostics for a service with an ops story this bot does not have yet |

Two omissions are deliberate and recorded rather than silent: dropping
`intent` and `lemmy` removes ~2.2k lines of the 12k, and the reason is that
neither is one of the eight actions in the spec. A port that quietly included
them would be a port of a different product.

### Why the cache is in-process

The source shares a Redis with FicHub so several bot replicas see one another's
responses. lorebot is one process with one token per linked user, so a Redis
would buy nothing and cost an operational dependency. The cache keeps the part
that matters: bounded size, TTL, and pagination cursor memory, so paging back
does not re-run a search.

## The message IR

Ported essentially unchanged, because it is the seam the whole design rests on.
`PlatformMessage` is one of:

* `Text(String)` — plain, safe everywhere.
* `Rich { header, items, actions }` — `RichItem` is title, url, body, fields,
  footer, color; `ActionRow` is buttons or a select menu.
* `File { filename, data, caption }` — an export the user asked to download.
* `Ephemeral(Box<PlatformMessage>)` — private to the invoker.

`Ephemeral` exists for a specific §23.2 requirement: **no private library
results in public channels**. A bookmark, an export link or an import status is
authorisation-bearing information about a person, and rendering one into a
shared Discord channel would leak it. Every private action therefore returns
`Ephemeral`, and the conformance suite checks that each adapter honours it —
which is the half of the privacy requirement the server cannot check, because
the server never learns where a message went.

## The eight actions

From §23.2, with the door each one calls. Every door is verified to exist and to
be token-reachable (Lorehaven M54 Part A).

| action | door | scope | private? |
|---|---|---|---|
| search eligible public works | `GET /api/v1/public/search` | none | no |
| fetch public metadata | `GET /api/v1/public/works/{id}` | none | no |
| start an authorized private import | `POST /api/v1/imports` | `content.write` | **yes** |
| check job status | `GET /api/v1/jobs` | `content.write` | **yes** |
| request a permitted export | `POST /api/v1/exports` | `library.read` | **yes** |
| save a bookmark | `POST /api/v1/bookmarks` | `library.read` | **yes** |
| return a link to continue on Lorehaven | none — client-side URL | — | no |
| send appreciation to authors | `POST /api/v1/works/{id}/kudos` | `content.read` | no |

The scope column is the contract the client enforces from its side. Lorehaven's
own tests check the server refuses a token without the scope; lorebot's
conformance suite checks the client *declares* it, so a method cannot be called
with a token that will not work.

## The link flow

§23.2 spells this out and Part A built the server half:

```text
bot issues a short-lived challenge
→ user opens Lorehaven, signs in there and only there
→ selects a pseud and scopes
→ confirms
→ bot receives revocable limited authorization
```

**Bots never receive the user's password.** The bot never sees a Lorehaven
credential it did not mint: the user authorizes in Lorehaven's own UI, and the
bot only ever holds the token that comes back. The store is encrypted at rest
and the token is never logged, not at debug level.

A token is issued with an explicit `acting_pseud_id`, and a token without one
is refused by the server (Part A). The bot therefore cannot fall back to a
default face, which is the point: a bot posting under a face the reader never
chose is the failure the whole acting-pseud column exists to prevent.

## Security requirements, and how each is tested

§23.2's list is not a wish list. Each item below is a test, not a review
comment.

| requirement | where it is enforced | the test |
|---|---|---|
| no private library results in public channels | `Ephemeral` wrapper in the core | each adapter renders `Ephemeral` as a private message; the conformance suite runs it per adapter |
| private actions require private delivery or a link | every private action returns `Ephemeral`; an unlinked user gets `NotLinked` | conformance suite: an unlinked user cannot obtain a private result |
| destination/channel context checked before every response | `Adapter::send` takes an explicit destination | conformance suite: a reply is never delivered to a destination the caller did not pass |
| revocation and unlinking | `revoke()` in the store; the token stops working on the next call | integration test against a real Lorehaven |
| tokens stored securely | encrypted file, key from env | the store refuses to start without a key; the file on disk does not contain the plaintext token |
| explicit disclosure that the chat provider receives submitted messages | `/link` and `/privacy` in the core, rendered by every adapter | conformance suite: the disclosure text is present and mentions the provider |
| bots never receive the password | the link flow mints a token server-side | integration test: the exchange response contains a token and no password field |

## Exit condition

From the plan, verbatim: **a Discord, a Telegram and a Matrix adapter all pass
the same core test suite; revoking a token takes effect on the next call; no
code path reaches the DB.**

The first clause is the one that shapes the build. It is a **trait-level
conformance suite** — one test body, run three times, once per adapter — not
three copies that drift. Three hand-written suites pass on the day they are
written and disagree on the day the core changes.

The third clause is `assert_no_database_dependency`: parse this workspace's
`Cargo.lock` and fail if any of `sqlx`, `rusqlite`, `diesel`, `tokio-postgres`,
`postgres` or `surrealdb` appears. It is a test because the property is
structural and a `grep` in a review comment is not a property.

## Verification

* `cargo test --workspace` — unit tests, and the conformance suite per adapter.
* `cargo clippy --workspace --all-targets` — 0 warnings.
* `cargo fmt --all --check`.
* `scripts/conformance.sh` — the suite run three times, once per adapter, and it
  must print three greens.
* The integration test needs a running Lorehaven; it is `#[ignore]`d without
  `LOREBOT_LOREHAVEN_URL` and run explicitly, so the unit suite never depends on
  a server being up.

## Out of scope

Stated so the boundary is a decision rather than an omission:

* IRC, Slack and Fediverse adapters. §23.2 says they "follow the same core";
  the core and the conformance suite exist so adding one is a small file.
* Natural-language intent classification (the source's optional-Ollama path).
* Community monitoring (the source's Lemmy poller).
* Reading bot *content* from Lorehaven — quoting, body text, recommendation
  digests. The eight actions do not include it, and it would need doors that
  are not open to a token.
