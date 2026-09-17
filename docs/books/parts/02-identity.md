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
