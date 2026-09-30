# M53-03 / M53-04 — gated adapters and blocked-here verification status

**Rows:** `M53-03`, `M53-04` (the only non-M45 `planned` rows; 45 remain in M45).

**Status of the parent milestone.** `M53-01` (batch 1 adapters ported against the
safe-fetcher `SourceAdapter` trait) and `M53-02` (frozen HTML fixtures) are
`implemented-fully-tested`: twelve adapters in `crates/scrapers/src/sites/`, fixtures
under `crates/scrapers/tests/fixtures/<source>/`. Both rows carry `--` for evidence,
which is a gap noted below but not a defect in the code.

**Spec basis.** `docs/spec.md` §11.6 (credential vault), §11.8 (runtime source
health), and the porting rules at `docs/spec.md:1993-1997`:

> - Login-requiring and adult-gated adapters port only behind the credential vault
>   (§11.6) and the age gate, and record that in their fixture suite.
> - Where this host cannot reach a source (Cloudflare-walled, geo-blocked), the
>   adapter records `verification_status = blocked-here` and is excluded from
>   support counts until live verification is possible; the user-supplied cookie
>   ingestion path is the documented workaround.

---

## What already exists (do not rebuild it)

| Thing | Where |
|---|---|
| `SourceAdapter` trait, `SourceKey`, `Wall` | `crates/scrapers/src/lib.rs:733+` |
| `SourceCapabilities` incl. `authentication: AuthKind` | `crates/scrapers/src/lib.rs:152` |
| `AuthKind { None, Token, Password, SessionCookie }` | `crates/scrapers/src/lib.rs:119` |
| `Registry`, `default_registry`, `can_handle` | `crates/scrapers/src/registry.rs` |
| `sources.health` column + `set_source_health` | `crates/db/src/imports.rs:386` |
| `recompute_source_health` (rolling window, §11.8) | `crates/db/src/imports.rs:437` |
| `settle_source_health` called from the worker | `crates/app/src/imports.rs:219` |
| age-gate machinery | exists (M-stage prior); `AuthKind::Password` docs already demand §11.6 consent |

**Verified absent** (`grep -rln 'credential_vault\|CredentialVault'` and
`grep -rln 'verification_status'` over `crates/` both return nothing):

- No credential-vault table, store function, or route.
- No `verification_status` column, constant, or type.

So both rows are genuinely `planned`, not stale labels. **This is the part that
makes them two rows and not one:** §11.6's vault is a store-and-route concern
(M53-03's precondition), while `blocked-here` is an adapter-and-counting concern
that must work for adapters that will *never* have credentials, because nobody can
reach the site from this host at all.

---

## M53-03 — gated adapters port only behind the credential vault and age gate

### Step 1 — migration `0096_source_credentials.sql` (both dialects)

`migrations/sqlite/0096_source_credentials.sql` and `migrations/postgres/0096_source_credentials.sql`:

```sql
-- Per-source credential vault (spec §11.6). Pseud-scoped, encrypted at rest,
-- never returned in plaintext by any endpoint.
CREATE TABLE source_credentials (
    id                TEXT PRIMARY KEY,
    pseud_id          TEXT NOT NULL REFERENCES pseudonyms(id) ON DELETE CASCADE,
    source_key        TEXT NOT NULL,
    kind              TEXT NOT NULL,          -- token | password | session_cookie
    origin_host       TEXT NOT NULL,          -- origin-bound credential use
    ciphertext        BLOB NOT NULL,          -- encrypted at rest
    created_at        TEXT NOT NULL,
    expires_at        TEXT NOT NULL,          -- default 30 days or the source's earlier expiry
    revoked_at        TEXT,                   -- immediate revocation
    last_used_at      TEXT,
    consent_at        TEXT NOT NULL,          -- explicit consent, required for password/cookie
    version           INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX source_credentials_pseud ON source_credentials (pseud_id);
-- No automatic copying across pseuds: one row belongs to one pseud, and the
-- absence of any (pseud, source) uniqueness would invite a "copy to all" write.
CREATE UNIQUE INDEX source_credentials_pseud_source ON source_credentials (pseud_id, source_key);
CREATE INDEX source_credentials_expiry ON source_credentials (expires_at);
```

**The unique index is load-bearing, not decoration.** Without it a
`POST /source-credentials` for a source a pseud already holds could create a second
row, and "no automatic copying across pseuds" becomes a convention rather than a
constraint. Test that a second insert fails.

Postgres variant: `BYTEA` for `ciphertext`, `UUID` for `pseud_id` if that is the
convention in `0093_preservation.sql` — **check before writing**, and mirror
`IS NOT DISTINCT FROM` for the nullable comparisons elsewhere in this repo.

### Step 2 — the gate itself, in the scrapers crate

`crates/scrapers/src/sites/mod.rs`: extend the registration API so an adapter
declares *why* it may only be used with a credential, and `default_registry()`
refuses to register one without the gate.

- Add `SourceCapabilities::requires_credential(&self) -> bool` — true when
  `authentication != AuthKind::None`.
- Add `Registry::register_gated(Box<dyn SourceAdapter>)`, which records the adapter
  and marks it gated.
- `Registry::parse(url)` on a gated adapter's host returns
  `SourceError::Unsupported` **naming the fix** ("<source> requires a stored
  credential; add one at POST /api/v1/source-credentials"), matching the existing
  refusal style at `registry.rs:83-96`.
- **An adapter with `authentication == Password` or `SessionCookie` must be
  `age`-gated to be registrable at all.** The porting rule says "only behind the
  credential vault *and the age gate*", so a build with the age gate off must not
  ship a cookie-reading adapter — the check belongs at registration, not in a doc
  comment.

### Step 3 — the four §11.6 routes

`crates/app/src/routes/source_credentials.rs`, mounted in the router:

```
GET    /api/v1/source-credentials        -> metadata only, never the secret
POST   /api/v1/source-credentials        -> create; explicit consent required for password/cookie
DELETE /api/v1/source-credentials/:id    -> revoke; does NOT delete imported copies
POST   /api/v1/source-credentials/:id/test -> test the credential, audit without secrets
```

Each is a thin handler over `crates/db/src/source_credentials.rs`. Every response
shape carries `id, source_key, kind, origin_host, created_at, expires_at,
revoked_at, last_used_at` and **never** `ciphertext`. Test that assertion directly
against the serialised JSON rather than trusting the struct not to have a field.

### Step 4 — audit events without secret contents

`crates/db/src/source_credentials.rs`: `credential_audit_events`
(pseud_id, credential_id, event, at) with `event ∈
created|used|tested|failed|revoked|renewed|expired`. §11.6 says "audit events
without secret contents" and "no credentials in job payloads, exports, logs, traces,
or analytics" — so the audit row must not carry the ciphertext, and neither must the
job payload for a test.

### Step 5 — expiry pauses jobs with actionable status, and auth failures are not retried

`crates/app/src/imports.rs`: on `SourceError::CredentialExpired`, the import row
gets a status naming the fix (spec §11.6 verbatim: "Expired credentials pause
affected jobs with actionable status. Do not repeatedly retry authentication
failures."). A bounded-retry count on authentication failures is the observable
proof of the second sentence — test that N consecutive auth failures produce N
attempts and no more, not N+1.

### Step 6 — fixtures record the gate (the row's own test)

`crates/scrapers/tests/fixtures/<gated-source>/`: add a `gated.json` alongside the
HTML recording, per the porting rule, that this adapter requires a credential, is
age-gated, and was not live-verified. A fixture test asserts the three.

---

## M53-04 — `verification_status = blocked-here`, excluded from support counts

### Step 1 — migration `0097_source_verification.sql` (both dialects)

Add to `sources`:

```sql
-- Where this host could not reach a source at all (spec §11.8 + porting rules).
-- NULL means the default: verified or never claimed.
ALTER TABLE sources ADD COLUMN verification_status TEXT;   -- 'blocked-here' | NULL
ALTER TABLE sources ADD COLUMN verification_note TEXT;     -- why, in operator words
```

`verification_status` is on `sources` rather than a new table because it is a
property of *this build's relationship with* a source, exactly like `health`.

**The `NULL`-means-default choice is deliberate:** 92 existing migrations write
`sources` rows, and a NOT NULL column with a sentinel would either rewrite all of
them or force every reader to handle two spellings of "fine".

### Step 2 — the adapter-side declaration

`crates/scrapers/src/lib.rs`: `SourceCapabilities::verification(&self) ->
VerificationStatus` with `Verified | BlockedHere { reason: &'static str }`,
defaulting to `Verified` so the twelve existing adapters are unchanged and the
default is the honest one for an adapter whose fixtures came from a real site.

The adapters that should declare `BlockedHere` are the ones this host cannot reach.
**Do not guess which those are — record it.** The gate in Step 3 makes a wrong
claim fail loudly (it is `blocked-here` and yet the fixtures parse as though they
were fetched live ⇒ the note contradicts the evidence).

### Step 3 — support counts exclude blocked-here, and the count must be honest about what it counted

`crates/scrapers/src/registry.rs`:

```rust
pub fn support_counts(&self) -> SupportCounts
```

returning `supported`, `blocked_here`, `total`, where **`supported` counts only
`Verified` adapters**. §11.7 is explicit: "Adapter counts are an outcome of verified
implementation, never a marketing claim." A count that includes a
`BlockedHere` adapter is exactly the marketing claim the spec forbids.

The test must be behavioural, not a restatement: build a registry with one verified
and one blocked adapter, assert `supported == 1 && blocked_here == 1`, and assert
that removing the gate (counting everything) fails that assertion.

### Step 4 — `blocked-here` is not `unavailable`

The most likely way to get this wrong is to fold `blocked-here` into health's
`unavailable`. §11.8 says "Do not label a source unavailable because one user's
credentials expired", and the same reasoning applies harder here: `blocked-here` is a
statement about *this build's host*, not about the source, and a public health
endpoint must not publish it as though the source were down for everyone.

- `blocked-here` sources keep health `unknown`, and are excluded from
  `recompute_source_health`'s window (`crates/db/src/imports.rs:437`) — otherwise
  the first import attempt from a walled host rewrites a `verified` source's health
  to `unavailable` on a permanent, non-actionable basis.
- Test: set a source to `blocked-here`, run the recompute, assert `health` is still
  `unknown`. Asserting only that `verification_status` was written would pass
  whether or not health was corrupted.

### Step 5 — the cookie ingestion workaround is documented

`docs/plan/gated-and-blocked-sources.md` (this file) plus a `docs/notes/`
entry: the user-supplied cookie path is the documented workaround for a
`blocked-here` source, per `docs/spec.md:1996`. §11.6 origin-binds credentials, so
the cookie's host must match the source's `origin_host` — test that a cookie for a
different host is refused rather than stored.

---

## Verification per step

| Step | Command | Expected |
|---|---|---|
| 1 | `sqlite3 :memory: < migrations/sqlite/0096_source_credentials.sql` | no error |
| 1 | run both dialects' full migration suite | 0 failing; ledger 92 → 94 |
| 2 | `cargo test -p lorehaven-scrapers --test gated_registry` | all pass |
| 3 | `cargo test -p lorehaven-app --test m53_source_credentials` | all pass; no serialised response contains `ciphertext` |
| 4 | same | audit rows carry no secret bytes |
| 5 | `cargo test -p lorehaven-app --test m53_credential_expiry` | paused status is actionable; N auth failures ⇒ exactly N attempts |
| 6 | `cargo test -p lorehaven-scrapers --test fixtures` | every gated fixture asserts all three facts |
| S3 | `cargo test -p lorehaven-scrapers --test support_counts` | 1 verified + 1 blocked ⇒ `supported == 1` |
| S4 | `cargo test -p lorehaven-app --test blocked_here_health` | health stays `unknown` |

Then the whole workspace on **both** engines, and `cargo clippy --workspace
--all-targets` at 0 warnings and `cargo fmt --all` clean.

---

## Decisions taken, and why

1. **`verification_status` is `NULL` by default, not a sentinel string.** 92
   migrations write `sources`; a NOT NULL column would rewrite all of them or force
   every reader to handle two spellings of "fine". Decided without asking because it
   is reversible with a follow-up migration and the alternative is worse.
2. **`VerificationStatus` defaults to `Verified`.** The twelve existing adapters
   have real fixtures from real sites; defaulting to `BlockedHere` would misreport
   them, and a default that misreports is worse than no default.
3. **`register_gated` refuses to register a `Password`/`SessionCookie` adapter when
   the age gate is off.** The porting rule says "only behind the credential vault
   *and the age gate*"; putting the check at registration makes the rule
   enforceable rather than aspirational.
4. **`SupportCounts` reports `supported` and `blocked_here` separately**, because
   §11.7's "outcome of verified implementation" is only checkable if the excluded
   set is visible.
5. **Not guessing which adapters are `blocked-here`.** The right set depends on what
   this host can actually reach, and I cannot determine that without attempting it.
   Row M53-04 is therefore promoted only once an adapter has been *observed* to be
   unreachable, with the note saying which host and what the failure was — not
   because a list was plausible.

## Not doing, and why

- **Not implementing the §11.6 vault's encryption.** The age/encryption primitive
  already exists for the snapshot gate; wiring it into the vault is a separate row
  unless the migrations need it. Step 1 stores `ciphertext BLOB` and leaves the
  encryption call to the store layer, which is the honest split.
- **Not adding a public health endpoint change.** §11.8's "public source status
  omits private import volumes, credential use, and user identities" is satisfied by
  *not* putting `blocked-here` into health at all (Step 4). Changing the endpoint's
  shape is a different row and would be scope creep.
