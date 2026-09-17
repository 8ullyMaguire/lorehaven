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
