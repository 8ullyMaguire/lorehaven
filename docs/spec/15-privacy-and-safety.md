# 15. Privacy and safety

**Status:** reserved
**Milestone:** M22
**Source:** the corresponding section of the Darkhold specification

## 15.1 Data classification

`public` · `unlisted` · `restricted` · `pseud_private` · `account_security` · `source_secret` ·
`staff_confidential` · `aggregate_public` · `aggregate_internal`.

The classification propagates into cache keys, search indexes, analytics, notifications and
background jobs. A reader's data is `pseud_private` by default; a reader's *account* data is
`account_security`; a work's classification is the work's `visibility`.

**The instance theme is `public`. The reader's taste is `pseud_private`.** That split is the whole
of §12.1's privacy story.

## 15.2 The metadata exchange — a signal is a fact about a work

> A signal is a fact about a work, never a fact about a reader.

A signal MAY carry: source identifiers, title, author names, fandom, tags, characters,
relationships, content rating, word count, chapter count, completion, content hash, language,
source URL, extracted-at.

A signal MUST NOT carry, and the schema has no field for: reading history, progress, position,
reading status, ratings, notes, kudos, private library membership, pseud linkage, draft content,
source credentials, session identifiers, IP addresses, file paths.

**A non-conforming client is refused by name**, not quietly trimmed. The batch types carry
`deny_unknown_fields`.

- **Both sides opt in, independently.** A disabled exchange answers **404**.
- **Low bar to submit, high bar to curate.** Level 1 to submit; level 3 plus quorum to curate.
  Neither bar is configurable.
- **No credits for submission.** A reward attached to a contribution is how a contribution becomes
  influence, and it is a spam vector.
- **Received metadata is a suggestion**, applied through the existing tag-proposal queue.
- **An unknown entity named by a signal is created immediately and visibly unverified**, with a
  signal count that is **review priority only**.
- **The response never reveals who submitted a signal**, and never reveals how many accounts hold
  a work.
- **Latent demand:** a signal for a work the instance does not hold is the one form of demand that
  arrives from outside the instance's user base. It is a *distinct-work* signal, not a vote, and
  the count of submitters is never a count of readers.

**The launch corpus (§9.8) uses exactly this mechanism**, with the existing
`build_lorehaven_signals.py` builder, which reads the contract from `lore-metadata` and asserts
its payload against the allowed field set rather than assuming it.

## 15.3 The snapshot — anonymised, signed, peer-to-peer

- Readers with **fewer than K bookmarks** (default 20) are excluded, and the filter runs **before**
  the row count is computed.
- Each included reader becomes a **salted pseudonym** and a **tag-affinity vector**. No user id, no
  work id, no username. The published row has exactly two keys.
- The snapshot is **256 shards**, plain JSON (a `.json.zst` filename that is not compressed is a
  lie about the format; it was renamed rather than fixed).
- A **manifest** records the version, base version, build time, row counts, the k-anonymity
  threshold, the salt mode, the salt, and every shard's **SHA-256 content hash**.
- The manifest is **signed (ed25519) over the canonical bytes** — the bytes that were signed, not
  the pretty-printed file.
- **Deltas** list only the shards whose content hash changed. A full snapshot every 20 versions. A
  delta names its base, and the retention policy **refuses to delete a version that a retained
  delta names as its base**. Delta-on-delta chaining is refused.
- **The privacy check parses the format and compares whole values.** A check over an opaque byte
  stream proves nothing — hashes, floats and pseudonyms all contain digits — and a filter matching
  nothing reads as clean.

**Transport:** peers are reached over an onion service. The transport package has **no code path
that can resolve a name or open a socket to anything but a SOCKS proxy pointed at an onion host** —
no plain-dial fallback, no DNS, the redirect target re-checked on every hop.

## 15.4 AI crawler posture

- The instance's `robots.txt` is **generated**, not static, and disallows AI-training crawlers by
  default.
- **There are no bulk text endpoints** to disallow (N-3).
- A work carries an **author-set `ai_training` assertion** (allow / deny / unset), shown as
  metadata and exported, described as *"this author's stated preference, recorded"* — never as
  "protected".
- **The archive indexes and recommends AI-generated works** (D-4/N-4), labelled as a declared
  attribute and filterable. It does not detect AI-generated text and does not claim to.

## 15.5 The safe fetcher

Every outbound URL is validated, resolved, and checked against the forbidden ranges and against
local hostnames before the request. The resolved address is re-checked **after every redirect**. The
user agent is identifiable and the rate posture is per site. Credentials are scoped to one site.

## 15.6 DMCA and takedown

A notice removes the material and its local mirrors on request, and does not delete the
`MediaReference` — the record that something existed and was removed. A counter-notice escalates to
a **quorum** rather than to a person. The takedown is recorded in the modlog and is visible.

## 15.7 Content policy

**The instance's content policy is the owner's, and Darkhold's default for it is permissive:**

- **Adult content is allowed**, behind a mandatory age gate that is **separate from the consent
  gate** (D-9).
- **Content involving minors is permitted and labelled.** The archive's job is the *mechanism*:
  the age gate, the first-class archive warnings, and per-reader filters that **hide rather than
  delete**. The archive does not moralise, and it never hides a work from the operator.
- **The archive never generates such content**, and there is no model in the default deployment.
- **DMCA, abuse reporting and the takedown pipeline apply unchanged**, and the age-gate state is
  recorded on the account so a takedown can identify the audience that saw the material.
- The policy is **declared in the instance metadata** (`/api/v1/meta`), is versioned, and is shown
  on the sign-up path. A reader learns the rules before creating an account.

**The operational exposure of D-9 is recorded as a risk (§25 R-9), not as a policy prohibition.**
An instance that permits this content must not be indexed by third-party search engines, must keep
the age gate and the warnings load-bearing rather than advisory, and must keep the takedown path
fast. Those are engineering requirements on the deployment, and they are the operator's call.

## 15.8 Age and adult gates

- The age gate is **separate from the consent gate** and is mandatory where the content policy
  requires it.
- The reader's age state is stored on the **account**, with guardian authorisation for a minor's
  account, and is consulted by policy functions, never by the frontend.
- An adult-gated source site needs both a credential and the adult confirmation (§9.3).

---
