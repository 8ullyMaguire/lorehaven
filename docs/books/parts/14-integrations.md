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
