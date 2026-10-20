# 20. The API conventions

**Status:** reserved
**Milestone:** M19
**Source:** the corresponding section of the Darkhold specification

## 20.1 Shape

- Prefix `/api/v1`.
- Collections: `{"items": [...], "next_cursor": null}` — **cursor** pagination, never offset.
- Errors: `{"error": {"code", "message", "field_errors", "request_id"}}`.
- **Stable machine codes**, never renamed without a version bump. The initial set: `AUTH_REQUIRED` ·
  `ACCESS_DENIED` · `NOT_FOUND` · `VALIDATION_FAILED` · `REVISION_CONFLICT` · `RATE_LIMITED` ·
  `QUOTA_EXCEEDED` · `CONTENT_RESTRICTED` · `SOURCE_UNSUPPORTED` · `SOURCE_UNAVAILABLE` ·
  `SOURCE_AUTH_REQUIRED` · `SOURCE_CREDENTIAL_EXPIRED` · `IMPORT_REVIEW_REQUIRED` · `QUERY_INVALID` ·
  `QUERY_TOO_COMPLEX` · `JOB_FAILED` · `CONVERTER_UNAVAILABLE` · `INSUFFICIENT_CREDITS` ·
  `EXTENSION_PERMISSION_DENIED` · `EXTENSION_QUOTA_EXCEEDED` · `COMMENT_HELD_FOR_REVIEW` ·
  `TRANSLATION_UNAVAILABLE` · `TRANSLATION_PERMISSION_REQUIRED` · `QUORUM_INSUFFICIENT` ·
  `DMCA_TAKEDOWN_PENDING` · `TRUST_LEVEL_INSUFFICIENT` · `CONSENT_REQUIRED` · `GUEST_CAP_REACHED` ·
  `TIER_REQUIRED` · `MOOD_COLD` · `INTERNAL` · `NOT_IMPLEMENTED`.
- **Status mapping:** 401 auth · 403 access/content/extension/trust/consent · 404 not found ·
  **409** revision conflict · 402 insufficient credits · 422 validation / source-unsupported /
  converter-unavailable · 429 rate-limited / quota · 502 source-unavailable · 500 job-failed /
  internal · 501 not-implemented.
- **404, not 403**, for an inaccessible private object *and* for a malformed id *and* for an
  operator surface. "403 answers 'yes, and you may not'."
- **`Internal` errors are masked**: "Something went wrong on our side. The request id below
  identifies it." The detail is logged, never returned.

## 20.2 Auth

- Opaque, server-managed sessions in secure cookies (`HttpOnly`, `Secure` in production, explicit
  `SameSite`, narrow path and domain, expiry and revocation).
- **CSRF** for state-changing cookie-authenticated requests. Turning it off in production is a
  startup error.
- Separately-issued, **scoped** API tokens with hashed storage.
- The authorisation pipeline: `authenticate → establish actor (anonymous / guest / pseud) → resolve
  the active pseud → load the resource → evaluate the policy → perform → record the audit event`.
- **Policy lives in policy functions.** The frontend's visibility is convenience, not security.
- **`Actor` says which it is asking about** — `kind` (anonymous / guest / pseud),
  `account_id`, `pseud_id`, `age_state`, `trust_level`, `tier`.
- **A token can never exceed its account, and it stops working when the account stops.** The
  scopes a token carries are chosen by the account that issues it and by nothing else, so the
  question "may this account hold this scope at all" is answered **at the mint** and not at the
  door — a door check asks "does this token have the scope", and a token that was issued with
  `admin.write` does.
- **A token is a credential an account holds**, so it stops working when the account does:
  suspended, closed, or **folded** (the fold leaves a tombstone, and a tombstone must not act).
  The resolver therefore joins the account, and the filter is `status = 'active'` rather than
  `status <> 'folded'` — **the default has to be the closed one**, so a status nobody has
  thought about is not treated as active.
- **The tier and its reason come from one evaluation**, so they cannot disagree. The three T2
  gates are evaluated in a fixed order — supporter, then trust, then resonance — and exactly one
  wins, because three overlapping "or" conditions produce an ambiguous tier and an ambiguous tier
  produces an ambiguous explanation.
- **A guest is a row in `accounts` with an empty credential and `is_guest = true`** (D-6), and a
  guest is an *account*: it has a trust level, it can never extend it, and a token it mints
  cannot exceed it either.
- **The age gate is separate from the consent gate.** The consent gate refuses a *mutating
  request* without consent; the age gate decides what an account may *do* once it exists.
  `consent.age_gate` is `"required"` by default, because D-9 makes the default instance an
  adult-content host, so **required is the safe default and an instance has to opt out of it
  deliberately.**
- **A minor's account requires a guardian authorisation**, and the grant **moves the account's
  age state** — a recorded authorisation that does not change the state is a row nobody reads.

## 20.3 Rate limiting

Layered: account/token · anonymous first-party client id · IP/network ceiling · endpoint and
resource class · source domain · concurrent job count · an expensive-operation budget.

An anonymous client id is **untrusted** and does not bypass the network ceiling. Forwarded IP
headers are trusted only from configured proxies. The `address_multiplier` (default 4) scales the
address bucket only, so shared NAT absorbs more; zero is rejected at startup.

## 20.4 Public API and the versioned surface

- Every work, collection, rating, tag, comment, theme, channel and statistic readable in the UI is
  readable by a machine.
- The full-text search, the recommendation endpoints, the arena, the exchange, the catalogue and
  the OPDS feeds are all public API.
- **The OpenAPI document is published** and the route inventory is asserted against it, so the
  document cannot drift.

**And the document is GENERATED from the route table, not written out by hand.** The hand-written
one described **four** paths while the instance served **a hundred and fifty** public ones, and
the only thing that noticed was a client generator producing a client that did not work. **A
document that is wrong in the direction of "less" is the one nobody checks**, because a client
that only calls what the document mentions works perfectly — against an instance that has far
more.

So the route table lives in the **crate** (`crates/app/src/routes/inventory.rs`), not in a test,
because two things need it: the route-inventory test asserts every registered route has an entry
with the correct audience, and the OpenAPI test asserts every public route is in the document and
every path in the document resolves to a registered route. **A table that exists in two places is
a table that can be in one and not the other.**

**The `paths` are relative to the `servers` entry, and that is not a detail.** OpenAPI 3.1 resolves
`paths` against the server URL, and the document's `servers` entry carries the `/api/v1` prefix —
so a path that carried the prefix itself would be `/api/v1/api/v1/works` for a client that
resolves it. **The first generated document did exactly that**, and an existing test in
`milestone_45.rs` caught it. The test was not written for this change and did not know the change
was coming; **that is what a test that predates the change is for.**

---
