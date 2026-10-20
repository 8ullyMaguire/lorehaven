# 8. Architecture

**Status:** reserved
**Milestone:** M1
**Source:** the corresponding section of the Darkhold specification

## 8.1 Shape

```
                    ┌───────────────────────────────────────────┐
   browsers ───────▶│  darkhold (binary)                       │
   OPDS clients ───▶│    ├─ HTTP surface: /api/v1, /opds, Atom  │
   bots ───────────▶│    ├─ jobs + worker (in-process or split) │
   export clients ─▶│    ├─ embedded Svelte 5 frontend          │
                    │    └─ SQL: SQLite  or  PostgreSQL         │
                    └───────────────┬───────────────────────────┘
                                    │ catalogue export (read-only, versioned)
                                    ▼
                    ┌───────────────────────────────────────────┐
   taste clients ──▶│  taste (binary)                           │
                    │    ├─ CSR tag graph (in memory)           │
                    │    ├─ spectral embeddings (in memory)     │
                    │    ├─ item-item co-bookmark index          │
                    │    ├─ arena (Glicko-2) + theme weighting   │
                    │    └─ recommend / rank / blend             │
                    └───────────────────────────────────────────┘
                                    ▲
                    signed, anonymised, k-anonymous snapshots (peers)
```

## 8.2 Two binaries, one workspace — and why

**The problem.** The taste engine's working set is an in-memory graph: 26.8 MB capped adjacency +
~80 MB of 32-dim embeddings + ~23 MB co-bookmark index + ~30 MB entity index. The archive's
working set is a request/response process with a database pool, a sanitizer, a job queue and a
web server.

**The measurement that decides it.** kindred's `ingest` peaks at 181.6 MB and its `embed` at
203.2 MB against a 220 MiB cap. In one process the peak is the *sum* of the phases, not the max —
the sum does not fit. lorehaven's ADR 0001 rejected a two-process deployment for the opposite
reason: the whole platform fit in one process, so a second process doubled the operational surface
for no benefit. **Both rejections were correct for their own context.** The archive is the case
where neither holds, so the split is **configurable** and the domain is shared regardless:

- `taste.mode = embedded` (default) — the taste engine is a crate the archive links directly. The
  catalogue is ingested into an on-disk index at startup. For a small instance this is right and
  there is one binary to deploy.
- `taste.mode = service` — a separate `taste` process against the same catalogue bundle. For a
  large instance this is right, because the graph's RSS is then the *service's* problem and the
  archive's RSS stays flat.

**What is shared either way:** the domain crate, the schema, the vocabulary, the identifier
scheme, the catalogue wire format, and the API contract. The two processes never share a database
connection (D-7).

**The budget is a hard number, and it is enforced.** The taste service carries kindred's caps
(220 MiB full, 60 MiB lite), a `make budget` gate that boots the real binary against the real
catalogue and reads `VmHWM` after each step, and a `/healthz` that returns **503** when the
process is over its own cap. "A gate that has never once failed, suspect the gate before you
trust it."

## 8.3 Workspace layout

One Cargo workspace, one domain crate, two binaries.

```
darkhold/
  crates/
    domain/          identifiers, errors, policies, the vocabulary, no I/O, no transport
    db/              migrations (both dialects), repositories
    app/             the `darkhold` binary: CLI, config, HTTP surface, worker, embedded frontend
    taste/           the taste engine: graph, embeddings, arena, ranking, catalogue ingest
    scrapers/        source adapters, the safe fetcher, the sanitiser, the robots posture
    metadata/        the exchange contract (serde only, publishable independently)
    decisions/       calibrated decision models: a model may narrow an acceptance to a hold,
                     never reject, never accept
    test-support/    a backend-aware harness: SQLite file by default, fresh PG per test
  frontend/          Svelte 5 + TypeScript + Vite, embedded into the binary
  migrations/
    sqlite/          one set per dialect, identical filenames, parity-tested
    postgres/
  docs/
    spec/            this document, split into sections (§24.2)
    requirements.csv the ledger — the row-by-row authority
    verification.md  the evidence log
    adr/             one file per decision
    known-gaps.md    open defects, each pinned by a test asserting current behaviour
```

**The domain crate is the only crate everything may depend on, and it depends on nothing.**

**kindred is a reference implementation, not a port target.** Its *algorithms* are reimplemented in
Rust against the same measured corpora, and the results are compared. The Glicko-2
implementation, the CSR builder, the snapshot format and the budget gate all have Rust
counterparts that must reproduce kindred's numbers on kindred's corpus before they are believed.
Porting Go line-by-line would import Go's allocation model and lose the representation win — the
3.0 GB → 124 MB cut came from the *data structure*, not the language.

## 8.4 The database

- **Both engines, both first-class.** SQLite is the development default and the small-instance
  target; PostgreSQL is the production target. Every schema change is gated on both (N-8).
- **PostgreSQL extras are optional and announced.** `pgvector` (semantic similarity) and
  `pg_trgm` (fuzzy matching) are detected at boot. If `pgvector` is absent, semantic features
  degrade to the spectral embedding path and the API says so in `meta.degraded[]`.
- **Redis is optional.** A rate limiter and a cache are useful; nothing in the core path requires
  them. An in-process fallback is provided and the degradation is documented.
- **Ollama is optional.** Nothing in the core reading, writing, importing or community path needs
  a model runtime.

## 8.5 The frontend

- **Svelte 5 + TypeScript + Vite, built to static assets and embedded in the binary.** One
  artefact to deploy, back up and version.
- **Plain Vite, not SvelteKit** — lorehaven ADR 0001. Routing, data loading and forms are
  hand-written. This buys the single-binary deployment and the Pi target.
- **The reader is server-rendered HTML** — no JavaScript required to read.
- **The theme is reworked around the new identity** (§21). The *architecture* of the token system
  survives; the *values* are new. This is the one decision in the set that re-opens the 543
  passing frontend tests, and §24.3 sequences it so the rework lands after the backend the pages
  depend on.
- **Site profiles** (the "replace any archive" goal) are a *configuration* — layout template, token
  set, navigation structure, default reader settings, feature toggles — selected by TOML. A skin
  changes presentation, never policy.

## 8.6 Jobs

One job system, one state machine:

```
queued → running → succeeded
running → retry_wait → queued
running → failed
queued|running → canceled
```

Fields: `kind`, `owner_account_id`, `owner_pseud_id`, `resource_class`, `priority_class`, `payload`,
`idempotency_key`, `attempt_count`, `run_after`, `lease_owner`, `lease_expires_at`, `progress`,
`error_code`.

- Transactional claims, leases and renewal; backoff; bounded attempts; abandoned-job recovery;
  cancellation at safe checkpoints; idempotent handlers; **at-least-once delivery**.
- **Fair queues** with aging: a paid job MUST NOT starve a free one. Separate resource classes,
  so a large converter cannot block all small imports.
- Recurring: nightly half-life recompute, nightly catalogue export, retention sweeps, preservation
  rechecks, link-health checks, deferred webhook delivery.

## 8.7 The adapter contract

```rust
#[async_trait]
pub trait SourceAdapter: Send + Sync {
    fn key(&self) -> &str;
    fn site(&self) -> SourceSite;              // posture, reachability, adult gate
    fn can_handle(&self, url: &str) -> bool;
    fn requires_login(&self) -> bool { false }

    async fn lookup(&self, ctx: &FetchCtx, url: &str) -> Result<SourceMetadata, AdapterError>;
    async fn fetch_units(&self, ctx: &FetchCtx, meta: &SourceMetadata) -> Result<Vec<SourceUnit>, AdapterError>;

    // the _from_html family — one API, five capabilities
    async fn lookup_from_html(&self, html: &str, url: &str) -> Result<SourceMetadata, AdapterError>;
    async fn fetch_units_from_html(&self, html: &str, meta: &SourceMetadata) -> Result<Vec<SourceUnit>, AdapterError>;
    async fn extract_tags_from_html(&self, html: &str) -> Result<Vec<ExtractedTag>, AdapterError>;
    async fn fetch_html(&self, ctx: &FetchCtx, url: &str) -> Result<String, AdapterError>;
    async fn list_creator_works(&self, ctx: &FetchCtx, url: &str) -> Result<Vec<String>, AdapterError>;
}
```

**The `_from_html` family is the most important idea in the ingestion design.** It enables Wayback
ingest, user-supplied-cookie ingest, browser-extension capture, paste-the-HTML, and fixture-based
tests — through one API, with no network. A test that needs the network is a test that cannot run.

**Config-driven families** are how 107 sites stay tractable: one adapter module returns a whole
family (`EfictionAdapter::all()` → 19 sites, `EficVariantAdapter::all()` → 26 sites).

---
