# Lorehaven Spec Amendment — Periodic De-identified Dataset Sharing

**Status:** Draft
**Date:** 2026-09-28
**Source:** Owner request — share a de-identified database dump periodically (monthly or so) without leaking the instance's IP or location
**Amends:** §11.15, §3.4, §38
**Adds:** §11.16, §11.17
**Milestone:** M60
**Plan:** `docs/plans/periodic-deidentified-dataset.md`

---

## Overview

An operator may publish a de-identified snapshot of this instance's data on a
schedule, so that other Lorehaven instances — and researchers — have something
real to work against without having to import a production corpus themselves.

The request has two halves, and they fail independently:

1. **the file must contain no identifying information**, and
2. **the transfer must reveal nothing about where this instance is.**

They fail independently, and that is the whole design of this section. A
perfectly anonymized dump published from a home connection identifies the
operator by its *timing* and its *host*. A dump distributed over an anonymity
network with raw emails in it identifies every account in it. **Neither half
compensates for the other**, so each is a requirement rather than a mitigation,
and each gets its own acceptance test.

The separation that makes this workable: **anonymization is decided at the
database, and distribution is decided at the edge.** They are different tools,
run at different times, in different places, and neither is allowed to be the
place where the other's job is done.

---

## 11.16 De-identified snapshots

### 11.16.1 What "de-identified" means here, precisely

A snapshot is de-identified when **no column in the dump carries a value that
identifies a person, an account, or a host**, and the row and column
*relationships* survive so the snapshot is still useful. That second half is
not a nicety: a dump with every key randomised independently is worthless for
the thing it is for, because every join becomes a no-op.

So the requirement is **consistent pseudonymisation**, not column destruction:
the same `pseud_id` must become the same replacement in every table that
references it, and different `pseud_id`s must become different replacements.

### 11.16.2 The columns, named

The schema's own migrations are the inventory. A rule that says "mask
identifying columns" is not implementable and not reviewable; this list is.

| Table | Column | Treatment | Why |
|---|---|---|---|
| `accounts` | `email` | **replace** | The primary identifier. Never reversible, never present in any form. |
| `accounts` | `password_hash` | **drop column** | A hash is a credential-adjacent secret. Anonymizing it is meaningless; it has no research value. |
| `accounts` | `handle`, `display_name` | **replace** | Chosen by a person, and reused across instances. |
| `pseuds` | `id` | **consistent replace** | The join key for the entire social graph. See below. |
| every table with `account_id` (26 of them) | `account_id` | **consistent replace** | **A second join key, and the one a first draft misses.** See below. |
| `webhook_endpoints` | `secret` | **drop column** | A bearer credential. Not PII — a live secret. |
| `sessions` | `token` and any session secret | **drop column** | A live session token in a published file is an account takeover, not a disclosure. |
| `device_*`, `*_deliveries` | any push/token/endpoint | **drop column** | A device token is a bearer credential for someone else's inbox. |
| `*_attempts`, `*_throttles`, limiter tables | any address material | **drop table** | See §11.16.5 — these hold no research value. |
| `imports` | `source_url`, `author_url` | **keep** | These are public catalogue URLs, not personal data. |
| `works`, `chapters` | body text, titles | **keep, gated** | The dataset's reason to exist. See §11.16.6. |
| timestamps | `created_at` etc. | **keep** | Relative time is the research value. See §11.16.5. |

A new column that carries personal data is **not** automatically covered by this
list, and that is a standing obligation rather than a one-time audit: **adding
a column to a table in this list requires deciding its row above.** The
enforcement is a test (§11.16.7) that reads the migrations and fails on a column
it has never been told about.

### 11.16.3 The `pseud_id` rule, which is the one that is easy to get wrong

`pseud_id` is the column every behavioural table joins on. If it is replaced
per-table, the snapshot is a set of unrelated tables. If it is replaced
*identically* per table, the snapshot is a usable synthetic social graph with no
person in it.

**The replacement must therefore be a deterministic function of the original
`pseud_id` alone** — the same input always yields the same output, with no
per-table salt and no randomness. It must also be a *UUID*, because the column
is `UUID` and a dump whose keys are the wrong type is a dump that will not
restore.

This is the one place where "just use a random UUID per row" is wrong, and the
reason is worth stating because the intuition runs the other way: **a random
re-key destroys the dataset.** `uuid_generate_v5(namespace, pseud_id)` over a
**fixed, published, non-secret** namespace is the correct construction. Secret,
because anyone holding the real `pseud_id` list could reverse it — and the
namespace must be published in the dump so a third party can verify the
construction rather than take it on trust.

### 11.16.3b `account_id` is a second join key, and it is the one that gets missed

`pseud_id` is the graph's join key and everyone remembers it. `account_id`
appears in **26 tables** and is a foreign key to `accounts.id` — so a snapshot
that re-keys `pseud_id` and keeps `account_id` has replaced a person's public
handle while leaving the row that identifies their **account** untouched, and
`accounts.id` is exactly the value every other `account_id` points at.

Two ways this fails, both of them real:

* **A dump with re-keyed `pseud_id`s and live `account_id`s is not
  de-identified.** It is a snapshot with one column obscured, and the obscured
  column is the one that still joins to the table holding the email.
* **Worse, it looks de-identified.** Every table reads as pseudonymous because
  the behavioural columns are, which is the failure mode this section exists to
  prevent — a *claim* of privacy with the identifying key still in the file.

**`account_id` gets the same treatment as `pseud_id`**: a deterministic,
published, stable derivation. Different namespace salt from `pseud_id` so the
two cannot collide, and the same "same input → same output" property, because
several tables join on `account_id` too.

**`accounts.id` itself is replaced by the same derivation**, so the foreign keys
still resolve. The two derivations use different salts, which means an observer
cannot learn that two accounts share a `pseud_id` and an `account_id` — which is
the association §11.16.3's determinism was otherwise making trivial to compute.

#### 11.16.3c The mask cannot be applied in place, and that is a property of the foreign keys

Added 2026-09-29, from running the re-key against a real PostgreSQL rather than
from reasoning about it.

A snapshot that re-keys `pseuds.id` and `works.owner_pseud_id` **cannot do so in
the live database, in any order.** Updating the child first trips
`works_owner_pseud_id_fkey`, because `pseuds` does not yet hold the new value.
Updating the parent first trips the same constraint, because `works` still holds
the old one. PostgreSQL checks referential integrity per statement, so no
ordering of two statements satisfies both directions.

The requirement this creates: **masking happens on a copy.** The pipeline copies
to a scratch database and masks there. Three things follow, and all three are
safety properties rather than implementation details:

1. **The live instance is never mutated**, not even transiently. A partially
   applied mask is an instance where some pseudonyms have been replaced and
   others have not — and the unreplaced ones are still real, still joined, and
   indistinguishable from the synthetic ones by any column.
2. **A failed mask leaves nothing to clean up.** A crash mid-mask against the
   live database leaves the operator with a database they cannot restore from
   their own backup with confidence about which rows moved.
3. **The ordering constraint becomes a design choice** rather than an FK
   violation, so the pipeline can be tested against a real constraint instead of
   being written around one.

This is also why the re-key is a **database function** rather than a value
computed by a script: the function is what makes "the same input yields the same
output in every table" structural, and the copy-then-mask shape is what makes
the whole thing safe to run. Either one alone is insufficient — a script with a
copy step still re-keys per row, and functions without a copy still cannot be
applied.

### 11.16.4 What is NOT in the dump, and why the answer is not "everything risky"

**Client IP addresses are not in the database and cannot leak from a dump.**
This was checked rather than assumed, because the owner's request names IP
leaking specifically and the natural fear is that the dump is full of them:

- No migration declares an `inet`, `ip_address`, `client_ip` or `remote_addr`
  column. `grep -rniE 'ip_address|client_ip|remote_addr|inet' migrations/postgres/`
  returns nothing.
- `crates/app/src/limiter.rs` reads an `IpAddr` from `X-Forwarded-For` or
  `ConnectInfo` and uses it to build a **rate-limit bucket key in memory**
  (`format!("ip:{}:{address}", class)`). It is never written to a table and does
  not outlive the request.

**So the IP exposure in this feature is entirely a property of the transfer, not
of the file.** That is why §11.16 and §11.17 are separate sections with separate
requirements, and why no amount of work on the dump addresses it. It is also
why §11.17 exists at all: a perfect dump published over clearnet still exposes
the operator's network.

### 11.16.5 Timestamps and absolute dates

Timestamps are kept, because "how long do people keep a work in their library
before abandoning it" is a question the dataset exists to answer, and
`timezone('epoch', created_at)` is a *loss of precision*, not anonymisation.

Two coarsenings are **not** anonymisation and must not be sold as such, and one
is a real leak:

- Coarsening to a day or a week reduces precision and is acceptable **as long as
  it is not coarsened to a constant**. A column of identical timestamps is a
  fingerprint.
- **A month offset is a linkable identifier.** Two dumps shifted by the same
  offset can be joined row-for-row on every timestamp. If consecutive snapshots
  are published, each must use a **different, unpublished** offset, and the
  offset must be applied *inside the snapshot* rather than recorded next to it.
  §11.17.4 is where the rotation is specified.

### 11.16.6 Bodies in a shared dump

§7.7 exists to keep a body from a reader outside its audience. A dump that
publishes every body undoes that for every audience at once, and the audience
is **not** expressible in a SQL dump.

So: **a published snapshot contains bodies only on an instance whose retention
mode publishes them** — that is, an instance that is an aggregating mirror must
publish a snapshot with no chapter text, and one that keeps bodies may publish
with them. This is §11.15's `cache | aggregate` doing the work it was written
for, and it is the reason a shared dump is a decision rather than a default.

**`body_audience` columns are dropped from the snapshot entirely** (§7.7.3: an
access rule that is readable is not one, and a dump that publishes the rule
invites bypassing it).

### 11.16.7 The dump is testable, or it is not a guarantee

The anonymization rules are **code with a test**, not a config file someone
applies. A masking rule that was never verified against a real dump is a
*claim*. The acceptance test:

1. builds a fixture database containing one of every identifying shape, with a
   known plaintext PII string per column;
2. runs the snapshot pipeline;
3. asserts the known plaintext appears **nowhere** in the output bytes;
4. asserts the `pseud_id` join still resolves across two tables;
5. asserts the output restores into an empty PostgreSQL and passes
   `lorehaven doctor`.

**A rule file that has not been through this is not a masking policy.** The same
argument the repository has already made about migration syntax, type casts and
sweeper logic applies here: an unverified claim about a data-leaking path is the
worst kind of unverified claim, because the failure is silent and irreversible.

---

## 11.17 Distribution

### 11.17.1 The two requirements are independent, and both are testable

| | Requirement | Fails how |
|---|---|---|
| **File** | contains no identifying value (§11.16) | a reader of the dump learns who |
| **Channel** | reveals no IP, hostname, or location (§11.17) | an observer of the network learns where |

**Passing one and failing the other is the two realistic bad outcomes**, and each
needs its own check. A dump is distributed **only** when both hold. There is no
"the data is anonymised so a normal upload is fine" path: anonymised data with a
timing side channel is still an operator identification, and an encrypted dump on
clearnet is still a location disclosure to whoever observes the connection.

### 11.17.2 Distribution channels, and what each does and does not hide

**OnionShare over Tor — the default, for a monthly file.** A temporary `.onion`
address; the server runs only while the session is open. No index ever sees it,
and the recipient needs no client software. **Trade-off, stated because it is
the reason the alternative exists:** the operator's machine must be online for the
duration, and a slow recipient extends that. For a monthly dump to a known set of
people, that is the right cost.

**I2P + I2PSnark — for a file that must stay available.** I2P has no exit nodes,
so traffic does not reach the clearnet and the real IP is not revealed to peers.
Two operational requirements: a **fresh I2P destination per snapshot** (a reused
one lets a recipient correlate every month's dump to one identity), and seeding
for as long as the file should be available. **Slower than clearnet by a wide
margin**, which for a large dump is a real operational fact and not a footnote.

**Refused: any clearnet host.** Object storage, a public git repository, a
paste site, a torrent tracker, email as a file channel, or a "temporary" direct
link. Every one of them records the uploader's address, and several retain it
after the file is removed. **A monthly cadence makes this worse than a
one-off**: a recurring upload from a stable address is a *subscription* to being
located, and the correlation across months is the product.

**Also refused: seeding a clearnet torrent behind a VPN.** A VPN hides the
address from the tracker; it does not hide it from the swarm. The peer set still
learns it.

### 11.17.3 Operational security, as requirements

- The sharing service runs in a **dedicated VM or container**, not on the host
  that serves Lorehaven. Compromising the share host must not be a path to the
  instance.
- The file is **encrypted at rest with a passphrase held nowhere near the
  download link** (§11.17.2 separates channels; this extends the separation to
  the key). Encryption is not a substitute for anonymization — it protects a
  copy that is intercepted, and does nothing for a reader who has the passphrase.
- A **new passphrase per snapshot.** A reused one means a single intercepted
  file is a standing key to every future one.
- The dump is **inspected before release** for the metadata §11.16.7 cannot see:
  timestamps of the dump itself, comments, tool version strings, and any
  filesystem path the tool embedded.
- Nothing in the share host may carry a reverse DNS record, a hostname, or a
  page with an author credit naming the operator.

### 11.17.4 Cadence, rotation, and what a recipient can correlate

Monthly is the default, and **the cadence is a privacy parameter, not a
convenience one**: each snapshot is an opportunity for someone to correlate it
with the last, and correlation is the actual threat.

- **A new I2P destination per snapshot.** Required for any I2P distribution.
- **A new timestamp offset per snapshot**, unpublished, per §11.16.5.
- **A new passphrase per snapshot** (§11.17.3).
- A **versioned manifest** — snapshot date, anonymization rule version, row
  counts per table — published alongside each dump, so a recipient can tell
  which rules produced a given file. It must not name the operator or the host.

What this explicitly does **not** achieve, because §7.7.4's argument applies
with more force here: **a de-identified dump is a forward-looking disclosure, not
a recall.** A dump published in month 1 cannot be un-published in month 3, and
no property of the format changes that. §11.16.3's deterministic `pseud_id`
re-key means a second snapshot from the same instance produces the *same*
replacements — which is what makes the graph usable and also what makes two
snapshots joinable by anyone holding both.

---

## 11.17.5 What is out of scope, stated so it is not oversold

- **This is not a synchronisation mechanism.** Snapshots are periodic and
  one-way; there is no update path and no conflict resolution.
- **This is not a backup.** A de-identified snapshot cannot restore an instance,
  and must never be presented as one.
- **This does not make the operator anonymous.** It stops a dump from carrying
  identities and a transfer from carrying an address. An operator who
  *participates* in a public forum under a real identity is findable by other
  means, and this section does not address that.
- **This does not protect readers' content from the platform operator.** A
  published snapshot is a disclosure the operator makes on their readers' behalf.
  §3.4 is unaffected by any of this and nothing here weakens it.
