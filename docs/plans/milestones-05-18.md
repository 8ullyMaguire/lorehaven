# Milestones 5–18

Same shape as `milestone-04-reader.md`, at a lower level of detail: enough to
start each one and to know what "done" means. Spec section numbers are given
for each because the spec still decides the details.

House rules from `README.md` apply to every task here without being repeated.

---

## Milestone 5 — Jobs, storage, cache boundaries and secret management

**Built.** Tag `v0.06-jobs`. The record of what was built, what was corrected and
what the tests are called is `milestone-05-jobs.md`; the evidence is in
`docs/verification.md`. The summary below is what the plan said, kept for the
milestones after this one that refer back to it. **This milestone unlocks M6 and
M7.**

Tables (migration 0005): `jobs`, `job_attempts`, `job_leases`,
`content_blobs`, `content_references`, `source_revision_cache_entries`,
`encryption_keys`, `secrets`.

First slice: a job that sleeps and logs, enqueued from an HTTP request,
executed by a worker, with its progress visible on a page.

Order:

1. `crates/domain/src/jobs.rs` — `JobState {Queued, Leased, Running, Succeeded,
   Failed, Cancelled}`, `JobKind`, `RetryPolicy { max_attempts, backoff,
   jitter }`, and a pure `next_attempt_at(attempt, policy, now) -> Instant`.
2. Repository `crates/db/src/jobs.rs`: `enqueue`, `claim_next(worker, lease)`,
   `heartbeat`, `complete`, `fail`, `cancel`, `requeue_expired_leases(now)`.
   **The claim is one statement** (`UPDATE … WHERE id = (SELECT … FOR UPDATE
   SKIP LOCKED)` on PostgreSQL, `… WHERE id = (SELECT …)` in a transaction on
   SQLite), not a select followed by an update.
3. Worker loop in `crates/app/src/worker.rs`, started from a `--worker` flag or
   the same process (`lorehaven serve --with-worker`). Graceful shutdown
   completes the job in flight and releases its lease.
4. `crates/db/src/storage.rs` — content-addressed blobs: `put(bytes) ->
   (checksum, storage_key)` writes to `storage/objects/<aa>/<hash>`, `get`,
   `stat`, `delete_if_unreferenced`. `content_references` is what makes deletion
   safe: a blob is only removed when nothing refers to it.
5. Secret encryption `crates/app/src/secrets.rs` with `xchacha20poly1305`, a key
   from the environment or a key file, `key_id` recorded per row so a rotation
   can re-encrypt lazily. Never log a plaintext or a key.
6. Outbox delivery: the worker drains `outbox_events` (already written by M3)
   and marks them delivered; a failing topic retries with backoff and records
   `last_error`.
7. Admin page `/admin/jobs` listing jobs, with cancel and retry. There is no
   staff model yet (M13 builds it, and it is a prerequisite for the rest of
   M17). For this milestone, add
   `config.administration.operator_account_id: Option<AccountId>` and gate the
   route on it, with a comment saying plainly that M13 replaces this with a
   trust level. Do **not** invent a boolean `is_admin` column.

Tests: `a_claimed_job_is_not_claimed_twice`; `a_lease_that_expires_is_requeued`;
`a_cancelled_job_stops_at_the_next_checkpoint`; `a_retry_uses_the_backoff`;
`the_same_bytes_stored_twice_share_one_blob`; `deleting_one_reference_keeps_the_blob`;
`a_secret_is_not_in_the_logs`.

Pitfalls:

* A worker that mutates state must check for cancellation **between** units of
  work, not only at the start, or a cancel is a lie.
* `content_references` is a security boundary, not a tidiness measure: physical
  dedup must never let one owner reach another owner's bytes. Reference rows
  carry the owning pseud; reads check it.
* Do not put the blob path in a URL. Serve bytes through an authorized route.

---

## Milestone 6 — Imports, source credentials, batches and preservation

Spec §11. Tag `v0.07-importing`. Depends on M5.

First slice (spec §1.3 gives it verbatim): paste a source URL → preview → import
privately → read → download → read offline. Build that for **one** adapter
before a second exists.

Order:

1. `crates/imports/` — a new crate. `trait Adapter { fn matches(&self, url) ->
   bool; async fn fetch(&self, ctx) -> Result<FetchedWork>; }` with
   `FetchedWork { metadata, chapters: Vec<FetchedChapter> }`.
2. `crates/domain/src/imports.rs` — `ImportDestination {PrivateLibrary,
   DraftWork}`, `ImportOutcome`, and the rules that a private import never
   becomes public content by accident.
3. Migration 0006: `sources`, `adapter_versions`, `external_records`,
   `library_items`, `import_snapshots`, `import_chapters`, `import_jobs`,
   `import_attempts`, `provenance_records`, `source_credentials`,
   `source_credential_consents`, `import_batches`, `import_batch_entries`,
   `source_health_windows`, `source_incidents`.
4. Adapters one at a time, each with stored HTML fixtures under
   `fixtures/importers/<source>/`. **A fixture, not a live request**, for CI.
   Status in `requirements.csv` is `implemented and fixture tested` until a
   live run is recorded in `verification.md`.
5. Safe fetching (spec §11.5): a dedicated HTTP client with a timeout, a size
   cap, a redirect cap, and an SSRF guard that refuses private address ranges
   and non-http schemes. Write `refuses_a_loopback_url` and
   `refuses_a_redirect_to_a_private_address` first — they are easy to write and
   embarrassing to omit.
6. Per-source credential vault using M5's encryption, with consent rows and
   expiry. A credential is bound to the pseud that supplied it.
7. Batches: `import_batches` + a job per entry, with progress, per-entry
   outcomes and a resumable state. Author/bibliography batches (spec §11.9)
   reuse it.
8. Update checking (spec §11.12): a job that re-fetches a known
   `external_record`, compares a revision signal, and records a change; never
   overwrites a library item without the owner's choice.
9. Pages: `/library/import` (paste URL → preview → destination → progress),
   `/library/imports/batches/:id`, source health on `/admin/sources`.

Pitfalls:

* **Media in an imported work** (images) is not part of the first slice;
  decide and record the decision rather than storing base64 in a revision.
* Imported HTML goes through the *same* `Document::from_json` schema as typed
  text. A converter that produces anything else is a leaking sanitizer.
* Never store a credential in a job payload. The payload holds a credential
  id; the worker reads the secret.

---

## Milestone 7 — Exports, device delivery and offline reading

Spec §12. Tag `v0.08-offline`. Depends on M5.

Order:

1. Export order (spec §12.1 fixes it): EPUB → HTML → plain text → Markdown →
   PDF → MOBI. Each is a job writing a `content_blob` plus an export record.
2. EPUB is written by hand against the OPF/NCX spec (a zip of XHTML); validate
   every generated file in a test with a strict parser. `docs/spec.md` §12.3
   requires validation, so an unvalidated EPUB is not finished.
3. `ebook-convert`/`pandoc` are *optional* converters: detect them, and have a
   working `CONVERTER_UNAVAILABLE` path with a message naming the remedy.
   `doctor` already detects them; reuse that check instead of a second one.
4. Send-to-Kindle (spec §12.4) behind a disabled-by-default integration with a
   working configuration check, a delivery log, and a failure the reader can
   see. If SMTP is not configured the control is absent, not broken.
5. PWA: manifest, service worker, offline shell. **Offline caches must respect
   the privacy classification** (spec §12.6): an offline copy of a
   signed-in-only work must not persist after sign-out. Write
   `signing_out_evicts_offline_content` as a test.
6. Full-text search inside an offline work uses the stored plain text.

Pitfalls: the service worker is the easiest place in this codebase to leak
another account's data through a shared cache. Version the cache by
account-or-anonymous, and evict on sign-out.

---

## Milestone 8 — Library, saved views, bookmarks and updates

Spec §13. Tag `v0.09-library`.

Order: migration 0008 (`library_items` extensions: shelves, private tags,
bookmarks, saved_searches, saved_views, reading_status, storage usage);
`crates/db/src/library.rs`; routes `/library`, `/library/shelves`,
`/library/bookmarks`, `/library/saved-searches`; batch operations that report
**per-item outcomes** rather than one boolean (spec §13.3); pages.

Acceptance: a shelf is a pseud-scoped object; a saved search is evaluated with
the same query compiler M9 builds, so M9 must be started before saved searches
with a query language are honest. A basic saved *filter* can ship in M8.

---

## Milestone 9 — Taxonomy, body search and query language

Spec §14. Tag `v0.10-search`. This is the largest backend milestone; budget
accordingly and build the vertical slice (filter by character → results) first.

Order:

1. Migration 0009: `canonical_entities`, `entity_aliases`, character and
   relationship assertions, `work_tags`, `tag_aliases`, `search_documents`
   (+ `search_documents_fts` on SQLite, a `tsvector` column and GIN index on
   PostgreSQL), `query_logs` (aggregate only), `zero_result_demands`.
2. `crates/domain/src/query.rs` — the typed AST and its **parser**, returning
   `QUERY_INVALID` / `QUERY_TOO_COMPLEX` with a position. Compile the AST to
   both dialects here.
3. **Unknown metadata is a first-class value**, not an absence: `Unknown` must
   be filterable and must not be silently excluded from a negative filter.
   Spec §14.5 requires an explicit completeness state; write
   `unknown_metadata_is_not_treated_as_false` first.
4. Indexer consuming `chapter.revised` outbox events (already emitted by M3).
5. Ranking (spec §14.8) with the weights as named constants and a documented
   interpretation.
6. Canonicalization and correction proposals (spec §14.9) with a review state.
7. People directory, filters, and the query-language UI with an AST preview.
8. The acceptance fixture in spec §14 `Acceptance fixture` becomes a test
   module: build the described works, run the queries, assert the sets.

Pitfalls: `LIKE '%term%'` is not search. Use FTS5 on SQLite and `tsquery` on
PostgreSQL, and hold the two behind the same ranking interface so a query
behaves the same on both.

---

## Milestone 10 — Discovery, private taste influence, recipes, dashboards

Spec §15. Tag `v0.11-discovery`. Depends on M9.

Order: baseline engines (recent, popular with a decay, tag-similarity) →
the administrator taste profile, which is **built from aggregate behaviour and
never names the administrator** → the influence layer with a slider and a
**meaningful opt-out** (off means off: not "reduced") → a recommendation recipe
builder with a preview → a widget-composed dashboard the reader arranges → the
administration page showing, per engine, what it uses and how to turn it off.

Tests: `an_opt_out_removes_the_engine_entirely_from_the_response`;
`a_minor_gets_no_taste_influenced_results`; `a_recipe_preview_matches_the_served_results`.

Pitfall: a recommendation that reads another account's private history is a
disclosure, not a bug report. Every engine takes *this* account's facts.

---

## Milestone 11 — Comments, forums, groups and messaging

Spec §16. Tag `v0.12-community`.

Order: migration 0011; comments and reviews on works; forum categories, threads,
posts with read state; forum search and tags; mentions and notification
preferences; groups with membership and roles; messages (direct and group) with
**block enforcement on every path**; real-time delivery over SSE with catch-up
from the database (Redis pub/sub is an optimization, never the source of
truth — spec §2.2); scoped sanctions.

Tests: `a_block_prevents_a_message_not_just_hiding_it`;
`a_realtime_client_that_missed_events_catches_up`; `a_sanction_is_scoped`.

Pitfall: blocks are already tables with no behaviour (M2-06 in
`requirements.csv`). This milestone is where they must become real everywhere —
searching, mentioning, replying, messaging and notifications. A block that hides
a comment but lets a mention through is not a block.

---

## Milestone 12 — Collections, challenges, requests and events

Spec §17. Tag `v0.13-events`. Smaller; mostly reuse of M8/M11 primitives.
Collections of works with curation and ordering; challenges with rules,
participants and entries; mentorship pairing; sprints with progress; requests
and bounties (the credit side lands in M14 — leave the hooks, not the ledger).

---

## Milestone 13 — Trust, reports, quorum, appeals and process feedback

Spec §18. Tag `v0.14-governance`.

**First task, and it is a prerequisite rather than a feature: introduce the
staff concept.** This repository has *no* administrator model at all today —
no column, no policy function, no route guard. M5 used an interim
`operator_account_id` from configuration for its jobs page; this replaces it,
once, with a domain type:

```rust
// crates/domain/src/policy.rs
pub enum TrustLevel { Member, Contributor, Moderator, Steward, Operator }
pub fn may_review_reports(level: TrustLevel) -> bool;
pub fn may_take_emergency_action(level: TrustLevel) -> bool;
pub fn may_read_private_pseud_linkage(level: TrustLevel) -> bool; // spec §4.2:
// "only specifically authorized staff may retrieve private pseud ownership"
```

Store it on the account (`accounts.trust_level TEXT NOT NULL DEFAULT 'member'`,
migration 0013), load it with the session in `crates/app/src/auth.rs`
(`SessionUser` gains the field), and add a `RequireStaff(minimum)` extractor
next to `RequireSession`. Every staff route in M5 and M17 then names the level
it needs, and the level is decided in a policy function rather than by comparing
account ids in a handler.

Then: report intake with categories
and evidence, quorum defaults in configuration, a voting queue with conflict-of-
interest refusal, emergency actions with a forced review, bootstrap mode for a
young instance, a **public modlog** that never leaks the reporter, and a
process-feedback surface.

Do not invent a second staff check for the same action. If a route needs to
know whether the caller is staff, it asks the policy functions above, and the
level comes from the session.

Tests: `a_reporter_is_never_named_in_the_modlog`; `a_quorum_that_is_not_met_does_nothing`;
`an_emergency_action_expires_without_review`; `a_moderator_cannot_vote_on_their_own_report`.

---

## Milestone 14 — Credits, fair queues, bounties and billing

Spec §19. Tag `v0.15-economy`.

Order: an append-only ledger (`credit_ledger_entries`, never a mutable balance
column), reservations that expire, job charging with a quote shown before the
job starts, the initial economy values in configuration, fair scheduling,
bounties on M12's requests, and optional billing behind a disabled-by-default
integration.

Tests: the ledger sums to the balance the API reports; a reservation that
expires returns its credits; a job that fails is not charged; **no float ever
touches a balance** (spec §3.2) — grep for `f64` in the ledger module and make
it fail review.

---

## Milestone 15 — Marketplace, extension isolation and gallery mechanics

Spec §20. Tag `v0.16-marketplace`.

Order: manifest schema and validation; categories; the capability model with
explicit grants; a WASM host (wasmtime) with a fuel limit, a memory ceiling, no
filesystem and no network beyond a declared allowlist; a review workflow before
publication; gallery mechanics; theme and layout safety rules (an extension may
not inject script or arbitrary CSS into another surface); a paid marketplace
path that stays disabled by default.

Pitfall: this is the highest-risk code in the project. Write the sandbox tests
first: `a_plugin_cannot_read_a_file`, `a_plugin_cannot_open_a_socket`,
`a_plugin_that_loops_is_stopped`, `a_plugin_without_a_capability_is_refused`.

---

## Milestone 16 — Public API, bots, feeds, push, federation and AI

Spec §21. Tag `v0.17-integrations`.

Order: scoped API tokens (the table exists from M1); the public REST API with
the same error envelope and cursor pagination; chat-bot shaped endpoints; an
outbox-driven notification pipeline with per-channel preferences, **push
payloads kept generic on the lock screen**; RSS, Atom and OPDS; ActivityPub
with a working disable path; an AI provider interface that is off by default;
a translation-review workflow.

Pitfall (spec §1.6): every optional integration must have working *disabled*,
*unavailable*, *misconfigured* and *failed* states, and each must be a test.

---

## Milestone 17 — Administration, statistics, abuse defence, privacy and ops

Spec §22. Tag `v0.18-operations`.

Order: the administrator surface, using the `RequireStaff` extractor and trust
levels that M13 introduced; public statistics with a
documented method; privacy-preserving analytics (aggregate, non-identifying);
an abuse dashboard; the layered defences from §22.5; **data export** for a
whole account; **deletion** that reaches backups on a stated schedule;
documented backups with a restore that has actually been performed once;
and an upgrade runbook matching the `--no-migrate`/`migrate` split the code
already implements.

Pitfall: a deletion feature is not implemented until a restore of a real backup
has been done and the schedule is recorded in `docs/verification.md`.

---

## Milestone 18 — Hardening and release

Spec §23. Tag `v1.0-release`.

Order: a Playwright (or equivalent) suite for the journeys in §23.1, run against
the compiled binary; the security tests in §23.2; accessibility work in §23.3
including the measurements that are still outstanding (`320 CSS pixels`,
`200%` zoom, screen-reader passes); the performance profile in §23.4 with the
budgets written down; and the platform/integration evidence in §23.5 — which is
where the **PostgreSQL instance this project has never run against** finally
gets exercised. Until then, every PostgreSQL claim in `verification.md` stays
`implemented but not executed`.
