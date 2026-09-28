# lorebot — build plan

Implements `docs/plans/lorebot-spec.md`. Read the spec first; this file is the
order of work and the proof.

Each step names the files it creates, the exact verification command, and what
that command must print. A step whose verification passes for the wrong reason
is not done — the notes say which wrong reasons were hit.

New repo at `~/code-local/rust/lorebot`. No path dependency on Lorehaven.

---

## Step 0 — the workspace

```
mkdir -p ~/code-local/rust/lorebot/{crates/{lorebot-core,lorebot-adapter,lorebot-discord,lorebot-telegram,lorebot-matrix},scripts,tests}
cd ~/code-local/rust/lorebot
git init
```

`Cargo.toml` at the root:

```toml
[workspace]
resolver = "2"
members = ["crates/*"]
```

Deliberately `members = ["crates/*"]` so adding an adapter is a directory, not a
root edit.

`rust-toolchain.toml`, pinned the same way Lorehaven's is, so a toolchain bump
is one commit rather than a surprise on someone else's machine.

**Verify**

```
cd ~/code-local/rust/lorebot && cargo metadata --no-deps --format-version 1 >/dev/null && echo OK
```

Must print `OK`. An empty workspace with no members fails here, which is the
point — a workspace that silently resolves to nothing builds an empty binary.

---

## Step 1 — the IR, `lorebot-core::core`

Ported from `~/code/rust/fanfic-archivist-bot/crates/archivist-core/src/core.rs`
essentially unchanged: `Button`, `ButtonStyle`, `SelectOption`, `ActionRow`,
`RichItem`, `PlatformMessage`, and the `Ephemeral` variant.

Two additions the source does not have, both because §23.2 requires them:

* `RichItem::notice` — a `RichItem` that is explicitly a notice (a disclosure,
  a rate-limit warning) so an adapter can style it apart from a result. The
  §23.2 disclosure requirement is otherwise indistinguishable from ordinary
  output, and a disclosure nobody can see is not a disclosure.
* `PlatformMessage::Ephemeral` is **not** an addition — it is in the source and
  is load-bearing. §23.2's "no private library results in public channels" is
  enforced by it.

**Verify**

```
cd ~/code-local/rust/lorebot && cargo test -p lorebot-core core:: -- --nocapture
```

Must show the IR round-trips through serde, that `Ephemeral` nests, and that
`RichItem` builders compose. Serialization is tested because the Discord and
Telegram adapters both send the IR over a socket, so a shape that does not
round-trip is a wire bug found at the adapter rather than here.

---

## Step 2 — the error type, `lorebot-core::error`

Port `BotError` with `thiserror`. Additions:

* `ScopeDenied { needed, granted }` — a 403 from Lorehaven. Distinct from
  `Api` because the fix is different: not "retry", but "this token cannot do
  this", which the user needs to be told plainly.
* `Revoked` — a 401 after a token that previously worked. Worth its own variant
  so the message can say *re-link* rather than *something went wrong*. The
  revocation requirement (§23.2) is exactly this path.
* `NotLinked`, `Config`, `Command`, `RateLimited`, `Other` as in the source.
* **No `Redis`** — that is the one variant the source has and lorebot does not.
  The in-process cache means there is nothing to fail.

**Verify**

```
cd ~/code-local/rust/lorebot && cargo test -p lorebot-core error::
```

Must show a 403 mapping to `ScopeDenied` and a 401 mapping to `Revoked`, and
that the two are distinguishable in their `Display` output. The test asserts on
the *message*, not the variant, because the message is what a user reads.

---

## Step 3 — the config, `lorebot-core::config`

`BotConfig` from the source, environment-driven: instance URL, per-platform
tokens, the store path and its key, per-platform rate limits.

Validation that **fails at startup, not at first use**:

* the instance URL is absolute and ends in `/api/v1`;
* the store key is present and at least 32 bytes;
* at least one platform is enabled.

**Verify**

```
cd ~/code-local/rust/lorebot && cargo test -p lorebot-core config::
```

Must show that a config missing the key is rejected, and — the one worth
writing — that a config whose store key is present but *short* is rejected with
a message naming the minimum. A store that starts with a weak key and only
discovers it on the first read is worse than one that refuses to start.

---

## Step 4 — the API client, `lorebot-api`

The largest change from the source: FicHub's client becomes Lorehaven's.

A `LorehavenClient` with a `reqwest::Client`, an instance base URL, and an
optional bearer token. **One method per §23.2 action**, each declaring the
scope it needs:

```rust
pub struct SearchWorks { /* query, limit, ... */ }
impl LorehavenClient {
    pub async fn search_public_works(&self, q: &SearchWorks) -> Result<SearchPage>;      // no scope
    pub async fn public_work(&self, id: &str) -> Result<WorkView>;                        // no scope
    pub async fn start_import(&self, req: &StartImport) -> Result<ImportView>;           // content.write
    pub async fn job_status(&self) -> Result<JobList>;                                   // content.write
    pub async fn request_export(&self, req: &ExportRequest) -> Result<ExportView>;       // library.read
    pub async fn save_bookmark(&self, req: &BookmarkRequest) -> Result<BookmarkView>;    // library.read
    pub async fn send_kudos(&self, work_id: &str) -> Result<KudosView>;                  // content.read
    pub async fn link_exchange(&self, code: &str) -> Result<LinkToken>;                  // none
    pub async fn list_tokens(&self) -> Result<Vec<TokenView>>;                           // none
    pub async fn revoke_token(&self, id: &str) -> Result<()>;                            // none
}
```

`ScopeRequirement` is a field on the request structs, not a comment, and
`assert_scope` is called before the request goes out. This is the client-side
half of §23.1's contract: a method cannot be called with a token that will be
refused.

`LinkToken` deliberately has **no password field**. §23.2 says bots never
receive a password, and a type that cannot hold one is stronger than a comment
saying it must not.

**Verify**

```
cd ~/code-local/rust/lorebot && cargo test -p lorebot-api -- --nocapture
```

Uses `wiremock`. Must show: a 403 body becomes `ScopeDenied` naming the scope;
a 401 becomes `Revoked`; a 200 with a token payload yields no password field;
and each method sends `Authorization: Bearer` only when a token is set. That
last one is the one people get wrong — a public search that sends a bearer
token unnecessarily is a credential leak to a log line.

---

## Step 5 — the store, `lorebot-core::store`

The token store. **Not a database.** An encrypted JSON file:

```
{ "<platform>:<user_id>": { token, acting_pseud_id, scopes[], created_at } }
```

* AES-256-GCM, key from `LOREBOT_STORE_KEY` (32 bytes, base64 or hex).
* The nonce is stored per record. A fresh nonce per write, and a nonce reused
  with the same key is a catastrophic failure mode, so the test asserts
  nonces differ between two writes of the same record.
* `revoke(platform, user)` removes the record and **does not** call the API.
  The server is the authority; the store just stops offering the token. The
  conformance suite pairs this with an integration test that the server refuses
  it.
* A `NoStore` implementation for a deployment that keeps tokens in memory only,
  so the trait has two real users.

**Verify**

```
cd ~/code-local/rust/lorebot && cargo test -p lorebot-core store:: -- --nocapture
```

Must show: the file on disk does not contain the plaintext token (asserted by
reading the bytes and searching for it); two writes of one record produce
different nonces; a wrong key fails to decrypt rather than returning garbage;
and `revoke` removes the record.

The wrong-key test is the one that catches a real bug: GCM authentication
failure must be an error, not an empty string that then reads as "no token" and
silently makes the bot look unlinked.

---

## Step 6 — the in-process cache, `lorebot-core::cache`

Ported from the source's Redis cache, minus Redis: bounded entries, TTL,
per-key pagination cursors, and a small fixed capacity.

Two things carried over from the source that are worth keeping because they
are load-bearing rather than incidental:

* a cursor is remembered so paging back does not re-run the search;
* the cache key includes the **token's acting pseud**, not the token, so one
  reader's results are never served to another.

**Verify**

```
cd ~/code-local/rust/lorebot && cargo test -p lorebot-core cache::
```

Must show: a TTL expiry evicts; the capacity bound holds under many inserts;
two different acting pseuds do not share an entry; and a cursor round-trips.

---

## Step 7 — the dispatch layer, `lorebot-core::dispatch`

The eight actions, each taking the IR out and the credentials in.

Every private action returns `PlatformMessage::Ephemeral(..)`. This is the
enforcement point for §23.2's privacy requirement, and the reason is worth
stating: the server cannot know where a message goes, so the *client* is the
only place this can be decided. A bookmark result is authorisation-bearing
information about a person.

`CoreCtx` holds the client, the store, the cache and the config. The
`/privacy` and `/link` commands are here, and `/privacy` is where the §23.2
disclosure text lives — that the chat provider receives submitted messages.

**Verify**

```
cd ~/code-local/rust/lorebot && cargo test -p lorebot-core dispatch:: -- --nocapture
```

Must show, with a mocked client: each of the eight actions returns a
`PlatformMessage`; the four private ones return `Ephemeral`; an unlinked user
gets `NotLinked` rather than a result; and the `/privacy` text mentions the
provider.

---

## Step 8 — the adapter trait and the conformance suite

```rust
pub trait Adapter {
    fn name(&self) -> &'static str;
    fn supports_ephemeral(&self) -> bool;
    fn supports_buttons(&self) -> bool;
    fn render(&self, msg: &PlatformMessage, dest: &Destination) -> Result<Rendered>;
}
```

`Rendered` is the adapter's native shape, so the trait does not force a
Discord embed onto Matrix.

**The conformance suite is the exit condition**, and it is written once:

```rust
fn conformance<A: Adapter>(a: A) { /* every test body */ }
```

`conformance(discord())`, `conformance(telegram())`, `conformance(matrix())`.
One body, three runs — not three copies that drift.

The suite must cover, for every adapter:

1. `Text` renders and contains the text.
2. `Rich` renders title, body and fields.
3. `Ephemeral` renders as a **private** message — and an adapter that cannot do
   private messages **fails**, rather than quietly rendering it publicly. This
   is the privacy requirement, and it is why `supports_ephemeral` exists.
4. Buttons render or are omitted per capability, and never invent a button.
5. The destination passed in is the destination used — no ambient channel.
6. A disclosure notice is visually distinct from an ordinary result.
7. Long text is truncated, not dropped.

**Verify**

```
cd ~/code-local/rust/lorebot && ./scripts/conformance.sh
```

Must print three `ok` lines — one per adapter — and exit 0. The script failing
on a single adapter is the point: it is what makes "all three pass the same
suite" a fact rather than a claim.

---

## Step 9 — the three adapters

Each is a translation from `PlatformMessage` to the platform's shape, plus a
gateway client. Thinnest possible, because the core owns the logic.

* **Discord** — `RichItem` → embed, `ActionRow::Buttons` → components,
  `Ephemeral` → ephemeral reply. The source's `fic-archivist` is the reference
  for the shape.
* **Telegram** — `RichItem` → an HTML message block, `Ephemeral` → a
  reply-to-you message. Telegram has no embed, so the field rows become a
  definition list in the body. Needs HTML escaping, which is a real correctness
  concern: a work title containing `<` must not become markup.
* **Matrix** — `RichItem` → an `m.room.message` with `formatted_body` (HTML),
  `Ephemeral` → a direct-to-one room event.

Each adapter is a thin binary crate; the gateway client is behind a trait so the
conformance suite can run without a network or a token.

**Verify**

```
cd ~/code-local/rust/lorebot && cargo test --workspace && cargo clippy --workspace --all-targets
```

Clippy 0 warnings. Then the three conformance runs again, since an adapter
change can only break its own render.

---

## Step 10 — the DB-reachability assertion

```rust
#[test]
fn no_code_path_reaches_a_database() {
    let lock = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../Cargo.lock")).unwrap();
    for banned in ["sqlx", "rusqlite", "diesel", "tokio-postgres", "postgres", "surrealdb", "libsqlite3-sys"] {
        assert!(!lock.contains(&format!("name = \"{banned}\"")), "...");
    }
}
```

A test, not a review comment, because the property is structural. A convention
that says "no database" is not a property; a test that fails when one appears in
the lock file is.

It also greps the source for `DATABASE_URL` and `postgres://` and `sqlite://`,
which catches a hand-rolled connection the lock file would not have.

**Verify**

```
cd ~/code-local/rust/lorebot && cargo test -p lorebot-core no_code_path
```

Must pass. Then prove it can fail:

```
cd ~/code-local/rust/lorebot && sed -i 's|^\[dependencies\]|[dependencies]\nrusqlite = "0.32"|' crates/lorebot-core/Cargo.toml && cargo test -p lorebot-core no_code_path 2>&1 | tail -3
```

Must **fail**. A test that has never been seen to fail is not known to work;
this is deliberately run in the wrong state once.

---

## Step 11 — the integration test

Against a real Lorehaven, so the property under test is a server behaviour the
bot merely observes.

* **Revocation takes effect on the next call.** Link, act, revoke server-side,
  act again → `Revoked`. This is §23.2's "revocation and unlinking" and it is
  the only test that can be right: lorebot asserting that its own store
  forgets a token proves nothing about the server.
* **No password in the exchange.** The link exchange response contains a token
  and no password field.
* **A scope the token was not granted is refused**, and the client reports
  `ScopeDenied` naming it.

`#[ignore]`d unless `LOREBOT_LOREHAVEN_URL` is set, so the unit suite never
depends on a server.

**Verify**

```
cd ~/code-local/rust/lorebot && LOREBOT_LOREHAVEN_URL=http://127.0.0.1:8080 LOREBOT_LINK_CODE=... cargo test -p lorebot-core -- --ignored --nocapture
```

Must show revocation refused on the very next call, not on the second.

---

## Step 12 — docs and the Lorehaven side

* `lorebot/README.md` — what it is, how to link, how to configure.
* `lorebot/docs/ADAPTERS.md` — how to add a fourth adapter (the port source's
  own doc, kept for the shape).
* Lorehaven `docs/requirements.csv` — M54-01 to `implemented-verified-e2e` with
  the test file named.
* Lorehaven `docs/handoff.md` — the Part B entry, including the two omissions
  (`intent`, `lemmy`) and why.
* Lorehaven `docs/plans/m54-bot-core.md` — Part B marked done.

**Verify**

```
cd ~/code-local/rust/lorebot && cargo fmt --all --check && cargo clippy --workspace --all-targets -- -D warnings && cargo test --workspace && ./scripts/conformance.sh
```

All four must pass, from a clean tree.

---

## The definition of done

* `cargo fmt --all --check` — clean.
* `cargo clippy --workspace --all-targets -- -D warnings` — 0 warnings.
* `cargo test --workspace` — green, both `lorebot-api` and `lorebot-core`.
* `./scripts/conformance.sh` — three greens, one per adapter.
* `no_code_path_reaches_a_database` — passes, and has been seen to fail.
* The integration test passes against a running Lorehaven, and is `#[ignore]`d
  without one.

## What this plan does not cover

IRC, Slack and Fediverse adapters; natural-language intent; community
monitoring; reading bot content from Lorehaven. All four are named in the spec
with the reason, so the boundary is a decision and not a gap someone rediscovers.
