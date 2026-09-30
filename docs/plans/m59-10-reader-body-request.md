# Plan — M59-10: a reader's own copy of an external body (spec §11.15b)

**Status:** not started.
**Requirement row:** `M59-10` in `docs/requirements.csv`, currently `planned`.
**Amendment:** `docs/spec-amendments/crawling-retention-and-preservation.md`
§6.2–6.4 (`§11.15b`), the only one of M59's requirement rows that no phase in
`crawling-retention-preservation.md` was assigned to.
**Prerequisite:** phases A0, A, B, C, C1, D, E, F — all built.

This plan is executable by an LLM with no context beyond the repository. Every
step names the file, the code, and the command that proves it.

---

## Why this is a new plan and not a phase in the existing one

`crawling-retention-preservation.md` has phases A0, A, B, C, C1, D, E, F, and
**none of them covers §11.15b.** The plan's Phase C line 904 mentions
`body_request_min_trust` only as one of three *naming* decisions for
`retention_governance`; nothing builds the request. So this is a gap in the
plan rather than unfinished work in it, and it gets its own file.

## What the spec asks for, in the spec's own terms

§6.2, quoted because the design decisions below all come from it:

- The request is **a job**, bounded by the source's robots posture and pacing
  (§11.5) exactly as any import is. **Not a synchronous fetch.**
- It produces **a personal snapshot referenced by that reader's copy**, so the
  trusted reader reads, downloads and reads offline — the actual bytes, not a
  preview.
- It is **recorded, with who asked and when**, in the same audit surface as any
  other storage event.
- On a `cache` instance, every reader eligible to read the work may then read
  the snapshot through the normal read path, because §11.15's `cache` contract
  already makes a cached body available. **The request removes the *storage*
  barrier for one work; it does not create a readers'-tier around it.**
- On an `aggregate` instance it is **refused by name** with `RETENTION_AGGREGATE`.

§6.3: `retention_body_request_min_trust` defaults to **2** and is configurable.
Floor 0.

§6.4, the acceptance list, verbatim as the requirements to be tested:

1. A reader below the configured bar is refused **with the bar stated**, and no
   fetch is attempted.
2. A request on a `cache` instance produces a per-reader snapshot and the bytes
   are readable, downloadable and offline-capable by that reader.
3. A request on an `aggregate` instance is refused with `RETENTION_AGGREGATE`.
4. **No route, surface or rendered page varies in whether a body is shown
   according to the viewer's trust level**, and a test fails the build if one
   appears.
5. Every request is bounded by the source's robots posture and pacing, and a
   refusal to read is not retried.

---

## The two decisions the spec does not make, and how this plan settles them

### D1 — What "referenced by that reader's copy" means here

The spec defers to "§10.4.1's existing durable per-reader snapshot". **There is
no such machinery in the repository.** Verified, not assumed:

```sh
grep -rn 'body_snapshot\|reader_snapshot\|per.reader snapshot' crates/ --include=*.rs
# no hits
```

So the plan has to build the thing the spec points at, and there is one existing
durable per-reader artefact to model it on: `chapter_revisions`, which holds
`document_json` / `sanitized_html` / `plain_text` (`migrations/sqlite/0003_works.sql:101`).

**Decision: a new `reader_body_copies` table, one row per (work, reader).** It
stores the bytes as text and carries the state a job needs
(`pending | ready | refused | failed`) plus the refusal's reason code. It does
**not** touch `chapter_revisions`: that table is the work's canonical revision,
and a per-reader copy is a different fact with a different owner. Writing into
`chapter_revisions` would make one reader's request change what every other
reader sees, which is precisely what §6.2's last bullet forbids.

**A work has no `source_key` column, and the plan's first draft assumed one.**
`works` (`0003_works.sql:37`) is owner/title/summary/language/rating/visibility/
lifecycle and carries no source. The link is **`library_items`**
(`0006_imports.sql:108`): `work_id REFERENCES works (id)` beside
`source_key NOT NULL`. `import_jobs` also has `source_key` and reaches the work
through `library_item_id`.

That relationship is **one-to-many** — a work re-imported from a mirror gets a
second library item — so "the work's source" needs a stated rule:

> **A work's source is the `source_key` of its most recent non-deleted
> `library_items` row.**

`ORDER BY created_at DESC LIMIT 1`, and a work with **no** library item is
refused by name ("this work has no import record") rather than defaulted to the
instance mode. Defaulting would be a guess, and a guess here decides whether a
body may be stored.

### D2 — Trust governs the *request*, never the *read*

§6.2 says the request "does not create a readers'-tier around it", and §6.4.4
demands a test that fails the build if any surface varies body visibility by
viewer's trust.

**Decision: the trust bar is checked once, in the route, on the way in.** No
read path reads `body_request_min_trust`; the copy is attached to the reader and
every reader eligible for the work sees the same thing. The guard test (§4.4
below) is a **source scan**, not a behavioural test, because the property is
about a class of future code.

---

## Step 1 — migration 0095, both dialects

New files, because 0094 is applied to some environments:

- `migrations/sqlite/0095_reader_body_copies.sql`
- `migrations/postgres/0095_reader_body_copies.sql`

Identical ids and column order in both (§2.1 of `docs/plans/README.md`).

```sql
CREATE TABLE IF NOT EXISTS reader_body_copies (
    id             TEXT PRIMARY KEY,
    work_id        TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    account_id     TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    -- NOT a foreign key and NOT a copy of library_items.source_key: it is
    -- recorded so the fetch can be bounded by the source's robots posture, and
    -- so a work re-imported from a different source later does not silently
    -- re-point an in-flight request at another site's terms.
    source_key     TEXT NOT NULL,
    chapter_key    TEXT NOT NULL,          -- the work's chapter list at request time
    state          TEXT NOT NULL,          -- pending | ready | refused | failed
    reason_code    TEXT,                   -- a RetentionReason code, when refused
    plain_text     TEXT,                   -- the bytes, once fetched
    sanitized_html TEXT,
    requested_at   TEXT NOT NULL,
    settled_at     TEXT,
    created_at     TEXT NOT NULL,
    updated_at     TEXT NOT NULL,
    version        INTEGER NOT NULL DEFAULT 1
);

-- The audit surface §6.2 asks for. NOT retention_policy_changes: that table
-- records mode changes and its actor is an operator, so a reader's own request
-- written there would be rendered as a policy change they made. This table is
-- the record of who asked for what, and when.
CREATE TABLE IF NOT EXISTS retention_body_requests (
    id             TEXT PRIMARY KEY,
    work_id        TEXT NOT NULL REFERENCES works (id) ON DELETE CASCADE,
    account_id     TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    source_key     TEXT NOT NULL,
    trust_at_request INTEGER NOT NULL,     -- the reader's level, recorded, not re-derived
    requested_at   TEXT NOT NULL,
    outcome        TEXT NOT NULL,          -- pending | ready | refused | failed
    reason_code    TEXT
);

-- One copy per (work, reader): a second request is a re-fetch, not a second row.
-- Without this, a reader who asks twice has two blobs and the read path has to
-- pick, and "picks the older" is a bug nobody writes a test for.
CREATE UNIQUE INDEX IF NOT EXISTS idx_reader_body_copies_work_account
    ON reader_body_copies (work_id, account_id);
```

Both files carry the header comment every migration in this chain carries, and
in it the two facts a future author needs: **a copy is per-reader and the
read path does not consult it** (so narrowing a request is not narrowing a
read), and **`state` is a job state, not a retention decision** — a `refused`
row records that a request was refused, and says nothing about the work's
retention mode afterwards.

`chapter_key` rather than a `chapter_revisions` foreign key: the work's chapter
list is what was fetched, and a work whose chapters are re-imported must not
silently re-point a reader's copy at different bytes.

### Verify

```sh
ls migrations/sqlite/0095_reader_body_copies.sql migrations/postgres/0095_reader_body_copies.sql
cargo test -p lorehaven-db migration     # expect: the dialect-parity test to pass
```

## Step 2 — config: `retention.body_request_min_trust`

In `crates/app/src/config.rs`:

1. `RetentionConfig` gains `pub body_request_min_trust: i64`, default **2**.
2. `RetentionSection` gains `body_request_min_trust: Option<i64>`, and the
   load site reads it with `unwrap_or(2)`.
3. `docs/config-reference.md` gains the row, and `lorehaven.toml.example` gains
   the commented key.

**The `FileConfig` half is the half that is easy to skip and has bitten this
milestone before** — `be68f32` found `roadmap.min_trust` and
`retention_governance.*` reading as configurable in every test while being
silently ignored in a real file. So:

- `RetentionSection` already has `deny_unknown_fields`, so a misspelled key is
  refused at load.
- Extend `crates/app/tests/config_sections.rs` with a case asserting
  `[retention] body_request_min_trust = 5` loads and that an absent key gives 2.

### Verify

```sh
cargo test -p lorehaven-app --test config_sections    # expect: one more passing than before
```

## Step 3 — the store, `crates/db/src/reader_body_copies.rs`

```rust
pub enum CopyState { Pending, Ready, Refused, Failed }   // parse_stored, as BodyMode does

pub struct ReaderBodyCopy {
    pub id: String, pub work_id: String, pub account_id: String,
    pub source_key: String, pub chapter_key: String,
    pub state: CopyState, pub reason_code: Option<String>,
    pub requested_at: String, pub settled_at: Option<String>,
    pub version: i64,
}

pub async fn source_for_work(db, work_id) -> Result<Option<String>>   // library_items, most recent
pub async fn request_copy(db, work_id, account_id, source_key, chapter_key, trust) -> Result<ReaderBodyCopy>
pub async fn copy_for(db, work_id, account_id) -> Result<Option<ReaderBodyCopy>>
pub async fn settle_ready(db, id, plain_text, sanitized_html) -> Result<bool>
pub async fn settle_refused(db, id, reason_code) -> Result<bool>
```

`request_copy` is an upsert on `(work_id, account_id)` that resets a `refused`
or `failed` row to `pending` and returns a `ready` row untouched — so a reader
whose source is unblocked can ask again without a second row, and a reader whose
body is already there is not made to wait.

**The dialect rules, per `db-migration-integrity`:**

- `?1` on SQLite, `$1::uuid` on Postgres — **each arm gets its own
  placeholder.** A `?1` on the Postgres arm is
  `cannot cast type integer to uuid`, an error that names a cast while the fault
  is numbering. Established at `retention_proposals.rs:334`.
- No `count(table)` anywhere: migration 0094's system account means a raw count
  of `accounts` is one too many. See §4.1.

### Verify

`crates/db/tests/` has no reader-copy suite yet; write
`crates/app/tests/reader_body_copies.rs` alongside the route tests in step 4 and
run it there. Do not add a lib-only test that never reaches a route.

## Step 4 — the route, `crates/app/src/routes/reader_body_copies.rs`

```text
POST /api/v1/works/{id}/body-request     (>= retention.body_request_min_trust)
GET  /api/v1/works/{id}/body-request     (the reader's own copy, or none)
```

Both `RequireSession`. Mounted beside the retention proposal routes in
`server.rs`, classified `RouteClass::Write` for the POST, and **tabulated in
`crates/app/tests/route_inventory.rs`** as `Audience::Authenticated` — the
inventory is enforced by a test and will fail otherwise.

### 4.1 The gate, in the order the spec states

0. **Resolve the work's source** from `library_items` (the most recent
   non-deleted row). None → `422` naming that the work has no import record. This
   comes first because the mode check needs a source, and defaulting to the
   instance mode would be a guess about where the work came from.
1. **Resolve the effective mode for that source.** `aggregate` → `422` with
   `code: "RETENTION_AGGREGATE"` and the work's source named. **No row is
   written** — a refused request leaves no trace beyond the response, because a
   row saying "this reader asked and was refused" is a record of a reader
   wanting bytes this instance does not hold.
2. **Then the trust bar.** Below it → `422` naming **both** the bar and the
   caller's own level, as `trust_refusal` in `retention_proposals.rs` already
   does. Reuse that helper's shape rather than writing a second one.

Order matters and is worth a test: an `aggregate` instance refuses on mode
regardless of trust, because a reader with all the trust in the world still
cannot make an `aggregate` instance store text. The opposite order would leak
the instance's retention mode to a reader who is not entitled to know it.

3. **Then write the `pending` row and return `202`** with the id and state.

### 4.2 The read path

`GET` returns the caller's **own** copy and nothing else — a reader may not read
another reader's copy, and `copy_for` takes the account from the session, never
from the query, so there is no parameter to get wrong.

### 4.3 The job

`JobKind::BodyFetch` appended at **index 13** — appended, not renumbered,
because the existing indices are load-bearing for any persisted array (the same
reason Phase E gave for index 12). The worker arm:

- `pending` → fetch, bounded by the source's robots posture and pacing, exactly
  as an import is (`lorehaven_scrapers::robots::resolve_posture`);
- **a refusal to read is not retried** (§6.4.5) — settle the row `refused` with
  the code and move on;
- success → `settle_ready`.

### 4.4 The guard test — §6.4.4, as a source scan

```sh
# Fails the build if any read path consults the request bar.
grep -rn 'body_request_min_trust' crates/ --include=*.rs \
  | grep -v 'config.rs\|reader_body_copies.rs\|routes/reader_body_copies.rs'
```

Empty is the pass condition, and it is asserted in
`crates/app/tests/config_sections.rs` by reading the same paths — a test that
only someone remembers to run is a comment.

**Why a scan and not a behavioural test:** the property is about *future* code
that might branch on the viewer's trust. A behavioural test proves today's
readers agree; only a scan constrains tomorrow's.

## Step 5 — the audit surface

§6.2 requires the request recorded "in the same audit surface as any other
storage event".

**`retention_policy_changes` cannot be that surface, and the first draft of this
plan said it was.** Its columns are `actor / from_mode / to_mode / source_key /
reason / changed_at` — there is no `action` column, so there is nowhere to write
`body_request` — and its `actor` is `NOT NULL REFERENCES accounts(id)`, which
means every row in it is a statement that *an account changed the instance's
retention mode*. A reader asking for a copy of a body is not that, and writing
it there would report a reader's request as a policy change.

So the migration creates **`retention_body_requests`** (above), and
`request_copy` writes one row per request: the reader's account, the work, the
source, and **the trust level they held at the time**. That last field is the
reason this is a table and not a re-derivation: trust moves, and a record of who
asked at what standing has to keep the standing, the same reasoning that made
Phase E's binding settlement record against a fixed system account rather than
whichever account happened to be available.

## Step 6 — the ledger

`docs/requirements.csv` `M59-10` → `implemented-locally-tested`, with the
tests named in `evidence` and the decisions D1/D2 recorded in `notes`.

`docs/verification.md` gains a newest-first section, **both dialects**, and the
counts from runs that completed.

`docs/plans/remaining-work.md` gains the M59 row update: 16 of 27, with the 11
remaining ids named.

`docs/spec-amendments/crawling-retention-and-preservation.md` — the build-status
section says what reached the spec. Add one line: §11.15b is built; the read path
consults the bar nowhere; the copy is per-reader and attached to a copy table, not
to `chapter_revisions`.

---

## The tests, and what each one is for

`crates/app/tests/reader_body_copies.rs`:

| Test | What it pins | What would break it |
|---|---|---|
| `a_reader_below_the_bar_is_refused_with_the_bar_stated_and_no_row_is_written` | §6.4.1 | a gate that fetches first, or a message without the bar |
| `an_aggregate_instance_refuses_by_name_and_never_reaches_the_trust_gate` | §6.4.3 **and the order in 4.1** | checking trust before mode, which leaks the mode |
| `a_cache_request_creates_a_pending_copy_the_reader_can_read` | §6.4.2 | a route that returns 200 without a row |
| `a_reader_cannot_read_another_readers_copy` | §6.2's "that reader's copy" | taking the account from the query string |
| `asking_twice_updates_the_copy_rather_than_adding_one` | the unique index | a plain INSERT |
| `no_read_path_consults_the_request_bar` | §6.4.4 | any future branch on the viewer's trust |
| `a_source_that_refuses_to_be_read_is_settled_refused_and_never_retried` | §6.4.5 | a retry loop |

**The privacy one is verified by injection before this plan is considered done** —
add a `body_copy` field to a response projection and watch
`a_reader_cannot_read_another_readers_copy` fail, then revert. A test that has
never failed is not evidence, and the Phase E privacy test is the precedent.

## Order

Migration → config → store → route → job → ledger. Each step's verify command
is above; run them in order and do not start the next until the previous is
green. The guard test is written **last**, because until the route exists it has
nothing to be true about.
