# M23-02 remainder — webhook watches and grant-gated bulk export

Planning document, 2026-09-18. Written against `6d1e1bf` after reading the spec,
the code and the milestone tests; every claim below carries a `file:line` so the
next reader can check it rather than trust it. Nothing here is implemented yet.

Scope: the two things `M23-02` still owes — "every query can be watched by a
webhook (§21) and exported as a grant-gated bundle (§13, fair-queued §20)"
(`docs/spec.md:4659-4663`), whose acceptance is "Bulk export of a large query
completes through the job queue, is rate limited and fair-queued, and never
bypasses download grants" (`docs/spec.md:4679-4680`).

---

## 1. What is already there

### 1.1 The query engine and the doors (reusable as-is)

- `GET /api/v1/media`, `/media/feed`, `/media/{id}`, `/media/{id}/files`,
  `/media/{id}/editions`, `/creators…`, `/distributors…`, `/media-collections…`
  (including two feeds), `/canons/{id}/media`, `/spaces/{id}/media` — read
  router at `crates/app/src/routes/media.rs:1126-1147`.
- `POST /api/v1/media/query` (JSON query), plus creator/distributor/collection
  writes — `crates/app/src/routes/media.rs:1150-1161`.
- One filter surface, `MediaQuery` (`crates/app/src/routes/media.rs:1168`), fed
  into `list_media_filtered` (`crates/db/src/media.rs:312`) which builds SQL from
  `media_filter(query, account_id)` — **eligibility is a parameter of the query
  itself**, which is why a cursor walk of it cannot leak a work the caller may
  not see. Cursor is compound (`created_at|id`, `crates/db/src/media.rs:369-379`).
- The query AST is a domain type, `QueryAst` (`crates/domain/src/query.rs:82`), so
  a stored query is a serializable value rather than a URL string to re-parse.
- Files are listed per work (`crates/db/src/media.rs:1213`) from `media_files`
  (`migrations/sqlite/0025_media_files.sql:5-18`: id, work_id, edition_kind, url,
  size_bytes, mime_type, checksum). **There is no per-file download door** — the
  bundle in Phase 2 would be the first path that hands a reader media bytes.

### 1.2 The export machinery (reusable in shape, not in size)

- `export_jobs` (`migrations/sqlite/0008_exports.sql:49-93`): `subject_type IN
  ('work','library_item')`, `format`, `options_json`, `state`, pointer columns for
  the artifact (`output_blob_checksum`/`output_bytes`), `converter_version`, and a
  unique `job_id`. The index on `(account_id, subject_type, subject_id, format)`
  is deliberately the "one live export per reader per subject per format" rule.
- `download_grants` (`:95-105`): `export_job_id NOT NULL REFERENCES export_jobs`,
  SHA-256 of the token (never the token), `expires_at`, `used_at`, `single_use`.
- Route shape to copy (`crates/app/src/routes/exports.rs:1-10`): `POST
  /exports/{id}/grant` mints, `GET /exports/{id}/download` serves the owner, `GET
  /exports/download/{token}` serves a bearer (`:266-329`), and the bytes come from
  the reference-counted `BlobStore` (`:332+`).
- The job is one arm of a closed set: `JobKind` (`crates/domain/src/jobs.rs:85-119`)
  is documented as "a new kind is a code change, because the worker needs a
  handler for it", and the arm dispatches to `crate::exports::run`
  (`crates/app/src/worker.rs:450-455`). `jobs` already carries
  `progress_permille` and `checkpoint` (`migrations/sqlite/0005_jobs_and_storage.sql:80-82`),
  which is everything a long walk needs.

### 1.3 The queue (priority only — no fairness)

`claim_next` is `ORDER BY priority DESC, available_at ASC LIMIT 1`
(`crates/db/src/jobs.rs:237-256`), backed by `jobs_claimable (state,
available_at, priority DESC)` (`0005_jobs_and_storage.sql:97`). `priority` is a
column nobody sets: the one enqueuer passes `0`
(`crates/app/src/routes/jobs.rs:408`). So "fair-queued" (§20.4: "Weighted queues
with aging. Reserve capacity for standard jobs. Paid demand must not starve free
users. Separate resource classes: a large converter cannot block all small
imports" — `docs/spec.md:3018-3022`) is **not implemented for anything**, not just
for exports. A reader who queues fifty exports starves every other tenant.

### 1.4 The webhook state — less than it looks

- Tables: `webhook_endpoints` and `webhook_deliveries`
  (`migrations/sqlite/0018_marketplace.sql:55` and neighbours).
- Routes: `GET`/`POST /api/v1/me/webhooks` only
  (`crates/app/src/routes/marketplace.rs:349`) — the spec's M16 line promises
  `DELETE` too (`docs/plans/junior-implementation-plan.md:1838`).
- Repository: `create_webhook` (`crates/db/src/marketplace.rs:479-521`) stores url
  + secret + an events list; `record_delivery` (`:523-553`) writes a delivery row.
- **Nothing ever sends one.** `record_delivery` has no caller on a delivery path,
  and `crates/app/src/worker.rs` contains no occurrence of `webhook` at all: the
  `Notify` arm only drains the outbox into registered topic handlers
  (`crates/app/src/worker.rs:300-340`, `:438-449`). There is no HTTP client in the
  app crate at all (`crates/app/Cargo.toml` among workspace deps has `sha2` and
  not `reqwest`, `Cargo.toml:53`). No event→endpoint matching, no retry, no
  delivery log worth the name.
- **The signature is not HMAC.** `WebhookEvent::sign` is
  `SHA256(secret ‖ event_type:event_id:created_at ‖ payload)`
  (`crates/domain/src/webhook.rs:20-27`) — a length-extendable prefix hash, not
  HMAC. Its unit test only signs and verifies with the same function
  (`:54-67`), so it cannot see the difference, and `requirements.csv` M16-01
  currently claims "HMAC signing, bounded payload, replay protection"
  (`docs/requirements.csv:103`). `hmac` is not a dependency (`Cargo.toml`).

### 1.5 Consequences for the milestone

The "smaller" of the two options is not smaller. Watching a query by webhook needs
the *delivery pipeline that does not exist* (HTTP client, signature correctness,
retry with backoff, dead-lettering, per-endpoint limits, an SSRF rule) before a
single watch can fire. Bulk export needs no new outbound capability at all: the
query engine, the job queue, the blob store and the grant/download path are all
here, and only the bundle builder and the fairness the spec names are missing.

---

## 2. Decisions

Each with the alternative it beat.

**D1 — a new `JobKind::BulkExport`, not an extension of `Export`.**
The enum is closed and the worker match is exhaustive, so a new kind is a
compile-checked change. The two jobs differ in the three things the queue cares
about: resource class (a bundle of 200 works versus one EPUB), progress semantics
(items walked versus one render), and idempotency (one live bulk export per
*query*, not per work). The payload shapes share nothing but the word "export".
*Alternative:* extend `Export` with a `subject_type = 'query'`. Rejected: it makes
the worker's first act a dispatch on a payload discriminator, and it puts a
long-running walk in the same queue row shape as a render.

**D2 — reuse `export_jobs` with a third `subject_type`, plus a new
`bulk_export_items` table.**
Reusing the row keeps `download_grants` (whose FK points at `export_jobs`), the
`/grant` + `/download/{token}` routes, the retention sweep and the "one live
export" index working untouched, and `options_json` is already the right home for
the stored query ("a JSON *document*: nothing queries inside it",
`0008_exports.sql:63-65`). SQLite cannot extend a `CHECK` in place, so this is a
table rebuild in a *new* migration, which is the repo's normal path
(`migrations/sqlite/0025_media_files.sql:1-3` explains why new tables get new
migrations and applied ones stay immutable). The new `bulk_export_items`
(id, bulk/export id, work_id, decision, reason, blob checksum, byte size) is
purely additive and is what makes "never bypasses download grants" an assertable
fact rather than a claim: the test reads the table.
*Alternative:* a separate `bulk_exports` table. Additive-only and no rebuild, at
the cost of a second grant table or a nullable-FK rebuild of `download_grants`,
plus a second serve/retention/listing path. Two of those cost more than one
rebuild; flip it if you prefer zero rebuilds.

**D3 — the grant rule, stated once and enforced in the job.**
Three checks, in this order, per matched work: (1) *eligibility* — inherent, the
query is run with the requester's `account_id` (`crates/db/src/media.rs:312-323`)
and so returns only what they may see; (2) *entitlement* — for a work with a price,
`has_entitlement` (`crates/db/src/monetization.rs:463-490`) must hold, including
expiry; (3) *delivery* — the archive leaves through a minted, single-use,
expiring grant (`crates/app/src/routes/exports.rs:266-329`), never a public path
(§13.2: "Generated download URLs are authenticated or short-lived and scoped",
`docs/spec.md:1939`). A work that fails (2) is **skipped and named** in the
manifest and in `bulk_export_items` — not included, not silently dropped. Bundle
membership is therefore a recorded decision, not an emergent property of a query.

**D4 — bundle format and bounds.**
A ZIP with one directory per work (`manifest.json` at the root: query, generated
at, counts, per-item decision + reason + SHA-256), because ZIP is what a reader's
OS opens without a bespoke step (the OPDS acceptance shows this milestone cares
about that, `docs/spec.md:4675-4676`). `zip` is not currently a dependency, so:
try `zip` first; if the registry is unreachable, write stored-entry (method 0)
archives by hand and verify by recomputing CRCs — either way the test asserts the
archive's structure rather than shelling out to `unzip`. Bounds are refused **at
request time**, before a job exists: a `COUNT` preflight against the same filters,
with `max_items` (default 200) and `max_bytes` (default 1 GiB) as config; over the
cap is a 422 naming the cap, not a ten-minute job that ends in a shrug.

**D5 — fairness: the subset of §20.4 that this milestone's acceptance names.**
(a) resource classes *derived from the kind* (a `JobKind::resource_class()` method)
so no migration is needed; (b) a worker option that claims only standard classes
(or only bulk), which is what "a large converter cannot block all small imports"
means operationally; (c) because the single-process deployment is the common one,
also a bulk-concurrency guard: claim a bulk job only when fewer than
`max_bulk_concurrent` (default 1) bulk jobs are running; (d) per-requester fairness
in the claim query — skip a job whose `requested_by` already has a job leased or
running. (d) is the anti-starvation half and is exactly what a test can prove:
one requester with a job in flight does not get their second job while another
tenant's first job waits.
*Deliberately not now:* weighted aging curves and subscription priority slots
(§20.6) — recorded as their own item so this milestone does not silently claim
them.

**D6 — a `RouteClass::Export` rate limit.** `RouteClass` currently has Auth, Write,
Search, Default (`crates/app/src/limiter.rs:139-146`) with a round-trip test
(`:507-514`); a fifth class plus `rate_limits.export { burst, per_minute }` in
config (`crates/app/src/config.rs:644`) is the smallest honest way to satisfy
"rate limited". Combined with the existing one-live-export index and the
concurrency guard in D5(c), that is three independent brakes, which is what the
acceptance implies.

**D7 — a watch is triggered by a job, not by an in-process topic handler.**
Topic handlers are registered only when the process runs `--with-worker`
(`crates/app/src/server.rs:93-121`); a deployment with a separate `lorehaven
worker` (`:90-92`) would never run them. So the publish/ingest path enqueues a
watch-match job with an idempotency key (`watch:<work_id>`), which works in both
deployment shapes and is testable by asserting the job, the match, and the
delivery row.

**D8 — HMAC for real, with the dependency we already have.** RFC 2104 HMAC-SHA256
over `timestamp.body` using `sha2` (already a dependency), compared in constant
time, with RFC 4231 test vectors as the known-answer test — the current
sign-and-verify-with-the-same-function test cannot prove anything about the
algorithm. A `X-Lorehaven-Signature: t=…,v1=…` header (the shape subscribers
already expect from Stripe-style webhooks) and a bounded age so a captured
delivery cannot be replayed (`docs/books/lorehaven-tutorial.md:2679-2681` is the
acceptance in prose).

---

## 3. Phases

Each phase is committable on its own and leaves the tree green
(`cargo fmt`, `cargo clippy --workspace --all-targets -- -D warnings`, workspace
tests). New routes get rows in `crates/app/tests/route_inventory.rs`, which as of
`127f72e` checks both directions: rows against extractors
(`every_route_has_correct_audience`) and registrations against rows
(`registered_routes_are_tabled` via `collect_registered`, which walks each
module's `router()` and `*_routes()` and resolves `.nest()` prefixes).

### Status (fourth review, 2026-09-19 — Phases 1 and 2 landed)

Landed and committed:

- **Phase 1 — queue fairness, done.** `claim_next` now takes
  `resource_classes: Option<&[ResourceClass]>`, `max_bulk_concurrent: u32`, and
  `fairness: bool` (`crates/db/src/jobs.rs`). Class filtering Live — a worker
  restricted to `[ResourceClass::Interactive]` never claims a `bulk_export` job
  (test: `a_worker_is_restricted_to_its_resource_classes`). The bulk concurrency
  guard is independent of fairness and applies to every claim. Per-requester
  fairness: a requester with a `leased` job is skipped while another tenant's
  job waits; NULL requesters (system jobs) pass through, because `NULL NOT IN
  (...)` is NULL, not TRUE — the trio of filters (state, owner, version) is in
  `claim_sql`, one place per dialect. Weighted aging and subscription priority
  slots remain deferred (plan §6 out of scope).
- **Phase 2 — bulk export, done end to end and reachable.**
  - Migration `0035_bulk_export.sql` (both dialects): `export_jobs.subject_type`
    gains `'query'`, and `bulk_export_items` records each work the bundle holds.
  - `crates/app/src/bulk_export.rs` — `run_bulk`: resolves the query's creator
    (or fails closed unless an operator requested it), counts against the cap,
    then walks the cursor deciding per work (right + entitlement, like the
    single-work Zip Walk in §1.2), renders HTML, ZIPs with a stored-method
    writer (`crc32fast`), stores via `BlobStore::put`, mints one download grant
    at the end, and records the items and file on the row.
  - `POST /api/v1/exports/bulk` (`crates/app/src/routes/exports.rs`,
    `start_bulk_export`) enqueues a `JobKind::BulkExport` with the query on it
    and acknowledges the privacy notice on the row; the existing
    `/exports/{id}`, `/grant`, `/download`, and `/download/{token}` doors then
    serve the bundle. Nothing bypasses a minted grant, so "no archive leaves
    except through the grant path" is assertable, by construction.
- `ResourceClass { Interactive, Bulk }` has consumers now; it never needed to
  move crates — `claim_next` takes the slices.
- `POST /jobs` still accepts only `maintenance`. A bulk export is enqueued by
  the reader route, which is the only door that can name a query.
- The `job_kind!` macro (a6f0e20) emits the enum, `ALL_KINDS`, `as_str`, and
  `parse` from one token tree, so the variant list and the string mapping can
  no longer drift; `kind_index()` and `resource_class()` remain separate
  exhaustive matches, so a new variant is still a compile error until it has
  both.

Still open: Phase 3 (webhook delivery) and Phase 4 (parity and records).
  `states_and_kinds_round_trip_through_their_columns` still walks six kinds by hand.

### Phase 1 — queue groundwork (½–1 day, no new surfaces)

- **Done:** `JobKind::resource_class()` returning a domain `ResourceClass`
  (`crates/domain/src/jobs.rs`) — see the status block; nothing consumes it
  yet. What remains here: the worker option + config `max_bulk_concurrent`,
  and the class-filtered claim.
- Per-requester fairness in `claim_next` (`crates/db/src/jobs.rs:237-256`), both
  dialects, with an index if the plan wants it.
- `RouteClass::Export` + config field + defaults + the class round-trip test.
- Tests: fairness (one requester in flight, another waits — seen failing without
  the claim change); class filter (a standard-only worker never claims a bulk job);
  concurrency guard; the rate class answers 429 at the configured burst.

### Phase 2 — bulk export (2–3 days)

- Migration: `export_jobs.subject_type` gains `'query'` (rebuild on SQLite and
  PostgreSQL per the migration convention); `bulk_export_items` created.
- `POST /api/v1/media/export`: session + the `ContentRead` API scope (the same
  scope the media read doors enforce, `docs/requirements.csv:124`), the same
  filter surface as `/media` (`MediaQuery` plus the JSON `QueryAst` body form),
  preflight count against the caps, one-live-export-per-query check, enqueue
  `JobKind::BulkExport`, answer `202` with the job and a status URL (the shape
  `POST /library/updates/check` already uses, `crates/domain/src/jobs.rs:103-108`).
- `crates/app/src/exports.rs`: `run_bulk` replaces the stub — cursor walk of
  `list_media_filtered` with the requester's account, per-work decision (D3),
  item rows, `checkpoint` after each page so a retry resumes, `progress_permille`
  as items/expected, bundle build (D4), blob reference via `BlobStore` with a bulk
  owner type, `state = 'ready'`.
- Routes `GET /api/v1/media/exports` (own, newest first), `GET …/{id}`,
  `POST …/{id}/grant`, `GET …/{id}/download` — the token path is the existing
  `/exports/download/{token}` reused as-is.
- Tests (all against SQLite, harness style): an export of a small query produces
  an archive whose entries match a fixture, byte for byte; a paywalled work is
  absent from the archive **and** present in the manifest with its reason, for a
  reader without an entitlement, and present for a reader with one; over-cap is
  refused before a job exists; a retried job resumes from `checkpoint` instead of
  re-bundling; the minted grant is single-use and expires; the blob is
  unreferenced when the export row is deleted.

### Phase 3 — webhook delivery, then watches (3–4 days)

- 3a delivery: real HMAC (D8), an HTTP sender in the worker (add `reqwest` to the
  app crate — the workspace already pins it, `Cargo.toml:53`), timeout, bounded
  payload (`bound_payload` exists), retry with backoff through the jobs table's
  `attempts`/`available_at`, terminal failures recorded in `webhook_deliveries`,
  an SSRF rule (refuse loopback/private/link-local unless the operator opts in),
  and `DELETE /me/webhooks` to go with the spec's M16 line.
  Tests: a local test receiver asserts the signature against an RFC 4231 vector,
  the timeout, a 500 followed by a success on retry, and a refused private
  address.
- 3b watches: `webhook_subscriptions` (endpoint_id, owner, the serialized query,
  scope, last-seen marker, state) with create/list/delete doors; the publish and
  ingest paths enqueue the match job (D7); the match runs the stored query with
  the watcher's scope, diffs against the marker, and writes deliveries for the
  new items only — `media.matched` carrying ids, titles and links of *visible*
  media and nothing else (§12's "no payloads in webhooks" rule for anything
  private; `docs/books/lorehaven-tutorial.md:2113`, `:2245`).
  Tests: a watch fires exactly once for a matching publish and not at all for a
  non-matching one; the marker stops a replay from re-firing; a watch whose
  watcher loses eligibility stops firing for that work.

### Phase 4 — parity and records (½ day)

- PostgreSQL run of the whole suite if a scratch database exists; otherwise the
  PG claim stays marked unverified (the environment note in `docs/handoff.md`).
- `docs/requirements.csv`: M23-02 to `implemented-locally-tested` (or its honest
  part), M16-01's claim corrected, new rows for the fairness subset if the
  operator wants them.
- `docs/verification.md`: the run history, the mutation/"seen failing" evidence
  per rule, and the PG status. `docs/handoff.md`: the open items this creates
  (weighted aging, subscription priority slots, per-file download door).

---

## 4. Acceptance → test map

| Spec line | Test |
| --- | --- |
| `docs/spec.md:4679-4680` "completes through the job queue" | the export request enqueues one `BulkExport` job and the worker produces `state='ready'` |
| "is rate limited" | `RouteClass::Export` burst → 429; one live export per query |
| "is fair-queued" | the claim-query fairness test plus the bulk concurrency guard |
| "never bypasses download grants" | the paywalled-work test (absent from the archive, named in the manifest) and the entitlement test (present for an entitled reader) |
| `:4660-4661` "every query can be watched by a webhook" | the watch test: one delivery, correct signature, no delivery for a non-match |
| §13.2 `:1939` "authenticated or short-lived and scoped" | grant redeem: single use, expiry, and 404 for spent/expired/never-existed alike |
| §20.4 `:3018-3022` (subset) | class separation, reserved capacity, per-requester fairness |

## 5. Risks and open questions

- **`zip` may not be addable offline.** Fallback is a stored-entry writer with CRC
  verification in the test; decided in Phase 2, not now.
- **Outbound HTTP in tests.** The delivery tests use a local listener; nothing in
  the suite should reach the internet (the 13 ignored network tests stay ignored).
- **SSRF rule vs self-hosted operators.** Default-deny for private ranges with an
  explicit operator allowlist is the safe reading; confirm before implementing.
- **The `export_jobs` rebuild** touches a table with a grant FK. Do it in Phase 2
  with the migration dialect skill and existing tests, not as a tail-end edit.
- **Watch volume.** A popular query watched by many accounts re-runs the query per
  watcher per publish. Acceptable at this scale, but the marker column should be
  indexed from the first migration so a batched sweep can replace the fan-out
  later.
- **Open question for you:** should the export be *one archive per query* (D2) or
  also *one per collection/canon* (`/media-collections/{id}/media` is already a
  door, `crates/app/src/routes/media.rs:1141`)? The filters make it a query either
  way; the difference is only in the request surface.

## 6. Out of scope

- §20.4 in full: weighted queues, aging curves, subscription priority slots
  (`docs/spec.md:3041`, `:3045-3051`). Phase 1 implements the anti-starvation
  subset and says so in the requirement row.
- A per-file download door (`/media/{id}/files/{file_id}/download`). The bundle is
  the first reader-facing path for media bytes; the direct door should be built
  next and must reuse Phase 2's decision function, not re-derive it.
- Send-to-Kindle delivery (`docs/spec.md:1945-1955`), which is M8 debt.
