# Junior implementation plan — the rest of Lorehaven

**Audience:** an AI agent or junior dev who knows Rust, SQL and Svelte but has
never opened this repository. Read this file top to bottom **before opening an
editor**, then read the spec section named at the top of the milestone brief
you are about to build, in full.

**What "current" means here:** as of **2026-09-14** (commit `cd87a07`),
Milestones 0–8 are built and tagged (`v0.09-library` is the latest tag); M9's
positivity rows are fully locally-tested; M10–M14 exist as **partial**
single-row passes; M15–M20 exist as committed single-row milestones whose
depth varies — `docs/requirements.csv` is the row-by-row authority, and
`docs/sessions/2026-09-14.md` is the hand-off that explains how to read the
two against each other. Milestone 21 is a **skeleton**: migration, contract
routes (501), domain rule modules and contract tests are in; the bodies are
the next work (§15 below). If `docs/verification.md`, `docs/requirements.csv`
or the git log disagree with this plan, they were updated after it was —
re-verify before acting, and update this plan file in the same commit as any
change you find.

---

## 0. Where the site stands (honest baseline)

The spec's section numbers (§0–§31) do **not** correspond one-to-one to the
repository's worked milestones. This plan uses spec **§-numbers** for
requirements and repo **M-numbers** for schedule slots; every brief below names
both.

### 0.1 What is built

| Spec | Repo | Topic | State |
|---|---|---|---|
| §5 | M0 | Repository, tooling, running application | built, tagged |
| §6 | M1 | Design system, navigation, localisation | built, tagged |
| §7 | M2 | Accounts, pseuds, privacy, age policy | built, tagged; block/mute are inert tables (0.3) |
| §8 | M3 | Drafts, chapters, publishing, revisions | built, tagged |
| §9 | M4 | Reader, ratings, reactions, history | built, tagged; search-within-work re-scoped to repo M10 |
| §10 | M5 | Jobs, storage, cache boundaries, secrets | built, tagged |
| §11 | M6 | Imports, credentials, batches, preservation | built; preservation batches deferred (0.3) |
| §13 | M7 | Exports, device delivery, offline reading | built; device delivery refused by default (0.3) |
| §14 | M8 | Library, saved views, bookmarks, updates | built, tagged `v0.09-library` |

Facts you can rely on:

- Migration head is `0009_library.sql`, present under both `migrations/sqlite/`
  and `migrations/postgres/` (a test keeps the two sets identical).
- Route modules: `crates/app/src/routes/{auth,pseuds,works,reading,settings,imports,exports,library,jobs,collaborators,health,meta}.rs`,
  wired in `build_router` (`crates/app/src/server.rs`).
- Frontend routes live in `frontend/src/routes/`, registered in
  `frontend/src/lib/router.ts`; linked-but-unbuilt pages render a `Planned`
  panel naming the milestone that will fill them — never mock data.
- `docs/verification.md` holds the evidence for every claim above; `just check`
  is the gate CI runs.

### 0.2 The numbering map for the rest of the build

Two spec milestones (§12 and §22) were skipped by the schedule that produced
tags `v0.05-reader` … `v0.09-library`: they have no rows in
`docs/requirements.csv` and no code. This plan restores them.

|| Repo | Spec | Topic | Depends on |
||---|---|---|---|
|| M9 | §12 | Positivity filter and feedback delivery | M3–M5 |
|| M10 | §15 | Structured taxonomy, body search, query language | M3, M4 |
|| M11 | §16 | Discovery, private taste influence, recipes, dashboards | M10 |
|| M12 | §17 | Comments, forums, groups, messaging, presence | M9 |
|| M13 | §18 | Collections, challenges, requests, wishlists, events | M9, M10 |
|| M14 | §19 | Trust, reports, quorum, appeals, sanctions | M9, M12 |
|| M15 | §20 | Credits, fair queues, bounties, billing | M9–M14 |
|| M16 | §21 | Marketplace, extension isolation, webhooks, gallery | M15 |
|| M17 | §22 | Translation pipeline | M9, M10, M15 |
|| M18 | §23 | Public API, bots, feeds, push, federation, AI providers | M9–M17 |
|| M19 | §24 | Administration, statistics, abuse defence, privacy, operations | M14–M18 |
|| M20 | §25 | Hardening and release | everything |
|| M30 | §0.4 | Instance topics — configurable public/private/flags with gamification bonuses | M15 (credits), M11 (leaderboards) |
|| M39 | §39 | Resource directory — community-curated ranked external resources | M2, M11 |
|| M40 | §40 + §33.1 | Fork with provenance, permission statements enforceable | M3, migration 0030/0036 |
|| M41 | §41 | Content half-life ranking signal, interaction warmth tiers | M4, M11, M12 |

The order is a dependency order, not a preference. Three examples, because a
junior will be tempted to reorder:

- **M9 before M12.** Every comment surface must apply feedback preferences
  *before* storage. Building forums first would mean re-opening every comment
  path once the classifier lands.
- **M10 before M11.** Two recommendation engines read the inverted index and
  the tag graph; neither exists until M10 builds them.
- **M14 before M15.** Credits must never purchase trust or moderation authority
  (spec §0.3). The trust model has to exist and be tested before anything
  buyable is minted, or the invariant cannot even be stated.

### 0.3 Debt register (carried explicitly, closed inside named milestones)

**How to read this register after the single-row passes (Sept 13–14):** each
brief below describes a milestone's FULL scope. Where the CSV shows the
milestone's rows already `implemented-locally-tested`, that covers only what
those rows state — the brief's unlisted acceptance criteria are still owed and
are the reason the full briefs are kept. `docs/sessions/2026-09-13.md` records
per-milestone review findings (M10, M12) that list concrete owed pieces.

| Item | Where it lands |
|---|---|
| `M2-06` block/mute primitives enforce nothing | **M12** — a block must hold on every new path (search, mentions, replies, messages, notifications); M12 is the first milestone with all of those surfaces |
| `M6-10` approved preservation batches | **M14** — behind a documented permission basis, the operator role and a dry-run report (spec §11.5) |
| `M7-03` device delivery (Kindle/email) refused by default | **M18** — when a mail transport arrives; priced through the credit quote flow |
| Source revision cache unpopulated (`M6-12`) | **M10** — its search reading pass is the natural populator; if not reached, re-register before tagging M10 |
| Chapter delete/reorder routes have no pages (verification limitation 14) | **M12** opportunistically, when WorkEditor is opened; otherwise the next milestone that opens it |
| `M17-01` administration beyond `doctor` | **M19** — that *is* the admin milestone |

### 0.4 Ledger first, code second

`docs/requirements.csv` currently has **no rows** for spec §12 (positivity) or
§22 (translation). The **first task** of M9 and M17 respectively is to add
those rows — before any code — using the same `M<repo>-NN` id scheme, so the
requirements ledger never loses a section again. Each brief below lists its
rows; step 1 of the workflow (§1) is always this.

---

## 1. The workflow loop (identical for every milestone)

The vertical-slice rule from spec §1.3, made concrete. Never write all
migrations for a milestone, then all routes, then all pages. Build **one
journey end to end**, check it by hand in a browser, then broaden:

```text
 1. ledger rows       add M<n>-NN rows to docs/requirements.csv (status planned)
 2. migration         BOTH dialects, identical sets (the drift test enforces it)
 3. domain types      crates/domain/src/<module>.rs — policy functions, no I/O
 4. repositories      crates/db/src/<module>.rs — both dialects written out
 5. routes            crates/app/src/routes/<module>.rs, wired in build_router
 6. pages             frontend/src/routes/<Name>.svelte + api.ts + router.ts
 7. hand journey      drive it in a browser against `just serve-dev`
 8. pin it            acceptance tests in crates/app/tests/milestone_<n>.rs
                      + component tests in *.test.ts beside each page
 9. record            verification.md rows with real evidence; flip CSV rows;
                      update docs/plans/README.md's table if scope moved
10. tag               v0.<nn>-<name> per docs/tutorial/README.md
```

Steps 8–10 are part of the feature, not a follow-up. A milestone is done only
when `just check` is green, the CSV rows are flipped **with evidence**, and the
hand journey has actually been driven. If any step cannot be completed, the
milestone is not done — say so in `docs/verification.md` using the status
vocabulary (`implemented but not executed` exists for exactly this).

---

## 2. House rules

Each rule exists because its absence already caused a defect in this
repository; the commit messages say so. Skim them here, re-read the section for
the layer you are touching.

### 2.1 Database layer (`crates/db`)

- Every statement is written **twice**: once for SQLite, once for PostgreSQL.
  `db.sql("<sqlite with ?>", "<postgres with ?>::uuid>")` picks one and rewrites
  `?` to `$1…$n` for PostgreSQL. Copy any function in
  `crates/db/src/content.rs` for the shape.
- Statements assembled from shared column lists use `sql_owned` — `db.sql`
  borrows, and a temporary `format!` result does not live long enough.
- Bind only `String` and `i64` (and `Option` of those). PostgreSQL `uuid`
  columns are bound `?::uuid` and read `id::text AS id`; integers are `BIGINT`
  (ADR 0004). This is what lets one row type decode on both engines.
- Never put a literal `?` inside SQL text; it is always a placeholder.
- Multi-statement transactions: write the SQLite branch and the PostgreSQL
  branch out **separately** — they are different transaction types. Do not
  abstract over them; the duplication is how dialect drift gets noticed.
- Migrations are numbered `NNNN_name.sql` under both `migrations/sqlite/` and
  `migrations/postgres/`, kept identical by a test. Add the next number; today
  that is `0010_<topic>.sql`.
- List endpoints return the cursor envelope
  `{ "items": [...], "next_cursor": ... }` (spec §3.3) and use keyset cursors
  (copy the library's `library_items` cursor). Bare arrays are a known
  inconsistency; do not add to it.

### 2.2 Domain crate (`crates/domain`)

- No I/O and no transport types. Policy functions live here so routes, workers
  and tests share them: the positivity classifier (M9), taxonomy shapes (M10),
  trust thresholds (M14). Each gets `crates/domain/src/<module>.rs` with unit
  tests in the same file.
- Errors use the one taxonomy in `crates/domain/src/error.rs`. Add variants
  there; never `anyhow::bail!` a user-facing condition. Map taxonomy codes to
  HTTP status in the route layer (the way `REVISION_CONFLICT` → 409 does).
- Identifiers come from `crates/domain/src/ids.rs`; do not invent a second id
  scheme.
- Content that reaches `{@html}` is produced only by the sanitiser in
  `crates/domain/src/document.rs`. New rich-text surfaces extend that module;
  they never hand-roll escaping.

### 2.3 HTTP layer (`crates/app`)

- Handlers use `classified(...)` which layers the request-class limiter outside
  the limiter marker. Use it; do not write the layers by hand.
- Cookie-authenticated state changes go under the CSRF layer (the
  `account_routes` subtree in `build_router`). New `Write`-class routes belong
  under it.
- `RequireSession`/`RequirePseud` where sign-in is required (extracting the
  extractor **is** the auth check); `MaybeSession` where a visitor may reach.
- Pseud isolation (ADR 0003): a work belongs to its pseud; switching faces
  makes it a 404, never a transfer.
- Optimistic concurrency: works/chapters carry `version` in the `WHERE` clause
  of every mutating statement; a stale save writes nothing and answers
  `409 REVISION_CONFLICT`.
- Limits (rate, body, batch sizes) are configuration values with documented
  defaults, never numbers in code — `lorehaven.toml.example` is the pattern.

### 2.4 Frontend rules

- Svelte 5 runes only: `$state`, `$derived`, `$effect`, `$props`. No stores, no
  `export let`.
- Field components take a `$bindable` value; `bind:value` without
  `$bindable()` on the child compiles and silently submits empty forms — this
  shipped once. `FieldBinding.test.ts` pins it.
- `frontend/src/lib/api.ts` mirrors the routes exactly and never navigates on
  failure. Types come from the server's response shapes; where the server omits
  a field, the type has no field.
- Router paths live in `frontend/src/lib/router.ts`. A linked-but-unbuilt
  destination resolves to `Planned` and names the milestone that will fill it.
- `{@html}` only for `sanitized_html` produced by
  `crates/domain/src/document.rs` — never for author text any other way.
- Every user-facing string goes through `frontend/src/lib/labels.ts` with both
  `en` and `eo` filled in the same commit (spec §7: translate everything).
- Run the frontend through `bash frontend/scripts/fe.sh build|test|check`;
  `npm run` may not find the binaries on this checkout (see `justfile` header).

### 2.5 Testing and honesty

- Rust: `cargo test --workspace`. Acceptance tests live in
  `crates/app/tests/milestone_<n>.rs` against the real router, a real SQLite
  file and a cookie jar that mimics a browser. Copy the harness from
  `milestone_8.rs` (the latest).
- Seed through the product's own entry points — e.g. imports seed library items
  via `imports::upsert_library_item` — never through hand-shaped fixtures.
- Frontend tests: `vitest`, in `*.test.ts` beside what they test.
- A test asserts the **property**, not the implementation: "a stale save writes
  nothing at all" is a property; "update_work returns Ok(false)" is not.
- When you find a bug during anything else, fix it and write the test that
  would have caught it.
- `docs/verification.md` has a status vocabulary; use `implemented but not
  executed` when that is the truth. Never claim a run you did not run.
- Every milestone ends with: `just check` green; CSV rows flipped with an
  `evidence` column naming a command or test (never a bare file path); a tag
  `v0.10-positivity` (next number, name your milestone); and the tutorial
  chapter written when `docs/tutorial/README.md` lists one.

---

## 3. Milestone 9 (repo) — Positivity filter and feedback delivery

**Read first:** spec §12 in full, plus §0.2 (priority 3) and §0.3. Also read
spec §8.6 — the feedback preferences stored since M3 are inert data; M9 is
where they start governing delivery.

### 3.1 Why this is next

Every later milestone creates comment-shaped surfaces: forums (M12), extension
reviews (M16), translation reviews (M17). Spec §12 is the gate they all pass
through. Building it on the two surfaces that already exist (work comments and
reviews) means the classification rule is built **once** and later milestones
inherit it.

The non-negotiable (spec §0.3): only positive or constructive criticism reaches
authors; constructive critique requires opt-in; nothing destructive is ever
stored or shown.

### 3.2 Ledger rows (add these before any code)

`docs/requirements.csv` has no §12 rows today. Add:

- `M9-01` incoming text classified before storage, per the author's preferences
  (spec §12.1–12.2)
- `M9-02` constructive critique reaches only opted-in authors, framed as
  requested, withdrawable by its writer (spec §12.3)
- `M9-03` delivery through the positivity layer with receipts; non-delivery is
  invisible to the sender where the author chose silence (spec §12.4–12.5)
- `M9-04` appeals limited to classification errors, resolved by evidence; no
  re-litigation of taste (spec §12.6)
- `M9-05` the feedback-preferences panel is live and shows its **effective**
  policy, not just toggles (spec §8.6, §12.2)

### 3.3 Migration `0010_positivity.sql` (both dialects)

```text
classifications(
  id TEXT PRIMARY KEY,          -- uuid (TEXT + ?::uuid binding, ADR 0004)
  subject_type TEXT NOT NULL,   -- 'comment' | 'review'
  subject_id TEXT NOT NULL,
  class TEXT NOT NULL,          -- 'positive' | 'constructive' | 'other'
  confidence_bp INTEGER NOT NULL,  -- basis points 0..10000 (binds as i64)
  signals TEXT NOT NULL,        -- JSON document, machine-inspectable
  classified_at TEXT NOT NULL   -- RFC 3339
)
index (subject_type, subject_id)

deliveries(
  id TEXT PRIMARY KEY,
  classification_id TEXT NOT NULL,
  author_account TEXT NOT NULL,   -- the author whose preferences apply
  outcome TEXT NOT NULL,          -- 'delivered' | 'held' | 'withdrawn'
  created_at TEXT NOT NULL,
  resolved_at TEXT
)
index (author_account, outcome)

preference_changes(
  id TEXT PRIMARY KEY,
  account TEXT NOT NULL,
  changed_at TEXT NOT NULL,
  document TEXT NOT NULL          -- the preference snapshot, for audits
)
```

Notes:

- Append-only stores. A withdrawal is a **new** state on the delivery row
  (`outcome` transitions to `withdrawn`, timestamped) — history is never
  rewritten.
- No foreign keys to works/comments beyond `subject_id`: the classifier must
  not care which surface produced the text, which keeps it reusable by
  M12/M16/M17.
- Store confidence as basis points (`INTEGER`), not `REAL` — it binds as `i64`
  under the house binding rules and survives both dialects unchanged.

### 3.4 Domain module `crates/domain/src/positivity.rs`

Pure functions, unit tests in-file, no I/O:

- `classify(&str, &ClassificationContext) -> Classification` with
  `Classification { class: Class, confidence_bp: i64, signals: Vec<Signal> }`.
  Start **rule-based** (category pattern lists tuned on a fixture corpus), not
  ML: the spec requires inspectability, and a ruleset you can print is
  inspectable. Signals name the matched category — they never quote the user's
  text.
- `resolve_delivery(Class, &FeedbackPreferences) -> Outcome` — the single
  function encoding §12.2's matrix: positive → deliver; constructive → deliver
  only if the author opted in, otherwise hold silently; other → refuse storage
  outright with a generic message.
- The constructive-framing constructor: constructive feedback is presented the
  way the author asked (or the platform default) — a pure transformation over
  the stored body, testable without a database.

Build a fixture corpus first (`crates/domain/testdata/positivity/*.txt` or
in-file `const` tests): praise; praise with questions; constructive with
concrete suggestions; constructive that tips into cruelty; pure cruelty;

### 3.5 Repository `crates/db/src/positivity.rs`

- `record_classification`, `record_delivery`, `withdraw_critique`
- `feedback_preferences_for(account)` — reads the **existing** M3 preferences;
  do not duplicate that table
- `pending_deliveries_for_author(account)` — what the outbox pass renders
- `classification_for_subject(type, id)` — idempotency: re-submitting the same
  comment must not double-classify

Classification + comment insert is **one repository transaction** with both
dialect branches written out. If classification refuses (`other`), nothing is
written anywhere.

### 3.6 Routes (`crates/app/src/routes/feedback.rs`, wired in `build_router`)

```text
POST /api/v1/works/:id/comments   (existing route gains classification)
POST /api/v1/works/:id/reviews    (existing route gains classification)
GET  /api/v1/me/feedback-preferences
PUT  /api/v1/me/feedback-preferences
GET  /api/v1/me/feedback/inbox    (author-side received feedback)
POST /api/v1/feedback/:deliveryId/withdraw
```

- Classification runs **before** the comment row is stored (§8.6: preferences
  apply before comments are stored — M3's acceptance line becomes true
  end-to-end here). An `other` classification is refused with a generic
  message and nothing persisted.
- Sender-visible responses never reveal the author's preference state beyond
  what §12.5 allows: "sent" or platform silence, never the reason.
- These are `Write`-class routes: CSRF applies; per-account rate limits come
  from configuration.

### 3.7 Pages

- `frontend/src/routes/Feedback.svelte` — the author's feedback inbox:
  delivered items, a held-but-counted summary line (never held content),
  preferences link, withdrawal of their own critique threads. Loading, empty,
  error and success states (spec §1.1).
- The account settings page gains the feedback-preferences panel; it must show
  the **effective** policy ("you receive praise; constructive critique on") so
  a preference can never look like it took effect when it did not.
- The reader comment form gains the platform's framing text and the sender's
  receipt line.
- Decide and record: backfill classifications for pre-M9 comments (a doctor
  command or a migration data pass). Either is acceptable; an undocumented
  gap is not.

### 3.8 Hand journey (drive before writing tests)

1. Author A opts into constructive critique, default framing.
2. Reader B leaves a positive comment → appears in A's inbox.
3. Reader B leaves a critique → appears framed, withdrawable by B.
4. Reader C leaves a non-constructive negative → generic rejection, **nothing
   stored** (verify in the DB, not just the UI).
5. Author D (opted out) receives a critique attempt → held; C sees platform
   silence; D's inbox shows the held count, not the content.
6. B withdraws their critique on A → A's inbox reflects it.

### 3.9 Acceptance tests (`crates/app/tests/milestone_9.rs`)

One test per spec §12 acceptance line:

- positive comment stored and delivered to opted-in and default authors
- constructive comment delivered only to opted-in authors; held otherwise
- destructive comment never stored; sender gets a generic response
- withdrawal removes the critique from the author's view; the audit trail shows
  the withdrawal happened
- a preference change applies to **subsequent** items only (no retroactive
  reclassification)
- classification is deterministic for the same input (fixture corpus)
- the sender-visible payload contains no reason field (assert on the JSON)
- re-submitting the same comment does not double-classify

Frontend: `Feedback.test.ts` covers the empty/loading/error states and the
held-count line; the preferences panel test asserts the effective-policy line
renders.

### 3.10 Pitfalls

- **Do not** filter after insert. "Preferences apply before comments are
  stored" is the exact defect class this milestone closes; classify, then
  store, in one transaction.
- **Do not** put the classifier behind a network call or a job. It is a pure
  domain function; the job queue is for *delivery*, not classification — that
  keeps the request path deterministic and testable.
- **Do not** leak the author's preference state to the sender's UI. Assert the
  sender payload has no reason field.
- Dialect trap: `confidence_bp` and the `signals` JSON column must survive the
  SQLite/PostgreSQL split — bind as `String`/`i64` only (ADR 0004).

---

## 4. Milestone 10 (repo) — Structured taxonomy, body search, query language

**Read first:** spec §15 in full, plus the M4 re-scope note in
`docs/verification.md` (search-within-work lives here as `M10-03`).

### 4.1 Why this is next

Discovery (M11) reads two artefacts this milestone builds: the inverted index
and the tag graph. Nothing in M11 can be written honestly before they exist.
It also closes `M9-01`'s sibling debt: search-within-a-work was re-scoped from
M4 to here because it is the same index a second client-side scanner would
duplicate.

### 4.2 Ledger rows

- `M10-01` structured taxonomy: fandoms, relationships, characters, tags,
  warnings — with aliases and canonicalisation (spec §15.1–15.3)
- `M10-02` advanced search over the taxonomy with boolean and facet filters
  (spec §15.4)
- `M10-03` search within the current work (re-scoped from M4, 2026-09-10)
- `M10-04` query language with saved queries (spec §15.5–15.6)
- `M10-05` fuzzy matching with a documented similarity floor (spec §15.7)
- `M10-06` mood search (spec §15.8)

### 4.3 Migration `0011_taxonomy.sql` (both dialects)

```text
taxonomy_nodes(
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,        -- 'fandom' | 'ship' | 'character' | 'tag' | 'mood'
  canonical TEXT NOT NULL,
  norm TEXT NOT NULL,        -- normalised lookup form (lowercase, trimmed)
  created_at TEXT NOT NULL
)
unique (kind, norm)

taxonomy_aliases(
  alias TEXT NOT NULL,
  norm TEXT NOT NULL,
  node_id TEXT NOT NULL,
  source TEXT NOT NULL       -- 'author' | 'import' | 'operator'
)
index (norm)

work_tags(
  work_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  weight INTEGER NOT NULL DEFAULT 0,   -- e.g. main vs side character
  added_at TEXT NOT NULL
)
primary key (work_id, node_id)

works_index(
  work_id TEXT PRIMARY KEY,
  body_text TEXT NOT NULL     -- sanitised text stripped of markup, for body search
)
index on body_text via a keyword table:

works_index_terms(
  work_id TEXT NOT NULL,
  term TEXT NOT NULL,          -- lowercased token
  pos INTEGER NOT NULL         -- ordinal token position
)
index (term, work_id)
index (work_id, pos)

moods(
  node_id TEXT PRIMARY KEY,
  axes TEXT NOT NULL           -- JSON vector, e.g. {"warmth":0.8,"tension":0.3}
)

work_moods(
  work_id TEXT NOT NULL,
  node_id TEXT NOT NULL,
  score INTEGER NOT NULL       -- 0..100
)
primary key (work_id, node_id)
```

Notes:

- `works_index` and `works_index_terms` are **derived data**: they are rebuilt
  from `content_revisions` by a job, not written by hand. The job kind already
  exists (M5); add `JobKind::IndexWork`.
- The taxonomy is **site-level**, not per-pseud — the reader-private layer is
  M8's private tags, which already exist and stay private (M8's own test
  `a_private_tag_is_not_a_public_tag` pins this; do not merge the tables).

### 4.4 Domain modules

- `crates/domain/src/taxonomy.rs`: node kinds, normalisation
  (`canonical_form`), alias resolution returning the node or "no such alias";
  merge policy (two nodes merge behind one canonical id, aliases point at the
  survivor).
- `crates/domain/src/query.rs`: the query language — a small grammar
  (`fandom:`, `ship:`, `tag:`, `word_count:>10k`, `updated:>2026-01-01`, free
  text), parsed to an AST, rendered to both a SQL fragment pair (SQLite and
  PostgreSQL) and a saved-query JSON document. The AST is also what saved views
  store (M8's saved views already store a versioned JSON query document —
  reuse `SavedView::needs_repair` semantics: an unparseable stored query is
  repairable, never misread).

### 4.5 Repository `crates/db/src/search.rs`

- `rebuild_work_index(work_id)` — tokenise the sanitised body into
  `works_index_terms`; called by the `IndexWork` job.
- `search_works(query: &QueryAst, viewer: Option<Viewer>) -> Cursor<WorkSummary>`
  — joins terms and taxonomy through the AST; unpublished work is invisible to
  anonymous viewers (the visibility rule already lives in content.rs; reuse it,
  do not restate it).
- `search_in_work(work_id, needle)` — positional lookup: find term positions
  and return paragraph anchors so the reader can jump. This is why `pos`
  exists.
- `resolve_alias(norm) -> Option<node_id>`
- `merge_nodes(survivor, absorbed)` — one transaction; aliases redirect.

### 4.6 Routes

```text
GET  /api/v1/search?q=...&cursor=...    (AST or free text; the AST parser
                                        accepts the query language)
GET  /api/v1/search/in-work/:id?needle=...
GET  /api/v1/taxonomy?kind=...&prefix=...   (autocomplete)
GET  /api/v1/taxonomy/:id
POST /api/v1/taxonomy/aliases            (operator; audited)
```

The query parser errors are `422` with the character offset named, so the UI
can point at the mistake instead of a bare refusal.

### 4.7 Pages

- `frontend/src/routes/Search.svelte` — facets from the taxonomy, boolean
  forms, saved queries, results with the cursor envelope. Empty query → the
  page renders help, not a spinner.
- Reader in-work search: extend `Reader.svelte` (the re-scoped M4 row): a
  needle, match list with paragraph anchors, jump-to-match. Long works must
  not require rendering every paragraph at once (spec §9.2) — the jump uses
  the anchor mechanism the reader already has.

### 4.8 Hand journey

1. Tag a work with fandom/ship/character/tags via WorkEditor.
2. Search `fandom:x tag:y` → the work appears.
3. Alias: search a known alias → canonical node; merge two nodes → old alias
   redirects.
4. Save a query; load it from the library's saved-views list.
5. In-work search for a word mid-book → jump lands on the right paragraph.

### 4.9 Acceptance tests

- taxonomy normalisation: the same tag typed in three casings lands on one node
- alias resolution returns the canonical node or none
- merging two nodes redirects their aliases; the absorbed node's work tags move
  to the survivor in one transaction
- search honours visibility: unpublished work is invisible to anonymous viewers
- results use the cursor envelope with keyset pagination
- in-work search returns paragraph anchors the reader can jump to
- a saved query round-trips; an unparseable stored query is repairable, never
  misread (`needs_repair`, listed with a repair affordance)

### 4.10 Pitfalls

- Do not invent a second index implementation for in-work search. One term
  table serves both site search and in-work search — the M4 re-scope exists
  precisely to prevent the second implementation.
- Rebuilding the index must be idempotent per work and crash-safe: a rebuild
  that dies halfway leaves the previous index intact (stage the new term set,
  then swap inside one transaction).
- The fuzzy-match similarity floor is configuration with a documented default,
  not a constant in code.
- Tokenisation happens on the **sanitised** text (strip markup first), or
  search results will surface markup artefacts.

---

## 5. Milestone 11 (repo) — Discovery, private taste influence, recipes, dashboards

**Read first:** spec §16 in full, plus §0.2 (priority 4) and §0.3 (the admin's
taste profile must never be visible, inferable, or hinted at).

### 5.1 Why this is next

M10 built the index and tag graph; two of the recommendation engines read them
directly. This milestone completes the reader loop (search → read → be offered
the next thing) and establishes the influence mechanism every later ranking
surface (M13 events, M14 trust) must reuse rather than reinvent.

### 5.2 Ledger rows

- `M11-01` recommendations from multiple engines, blended, each result
  explainable at engine level (spec §16.1)
- `M11-02` private taste profile derived from the reader's own behaviour,
  inspectable and clearable by that reader, never visible to anyone else
  (spec §16.2)
- `M11-03` operator taste influence: private work affinities applied as ranking
  multipliers, audit-logged server-side, never surfaced in any API response,
  label, or credit breakdown (spec §16.3–16.4)
- `M11-04` diversity mechanisms: exploration slots and per-fandom caps so
  influence cannot make the site monothematic (spec §16.4)
- `M11-05` recipes: shareable recommendation formulas, sandboxed to the saved-
  query subset, honouring every reader's own opt-outs (spec §16.5)
- `M11-06` dashboards: personal home assembly from widgets in the M1 slots
  (spec §16.6)

### 5.3 Migration `0012_discovery.sql` (both dialects)

```text
taste_profiles(
  account TEXT PRIMARY KEY,
  signals TEXT NOT NULL,      -- JSON: {fandom:{id:bp}, tag:{...}, mood:{...}}
  computed_at TEXT NOT NULL
)

operator_affinities(
  work_id TEXT PRIMARY KEY,
  affinity_bp INTEGER NOT NULL,   -- -5000..10000
  operator TEXT NOT NULL,         -- who set it, for the audit trail only
  rationale TEXT NOT NULL,        -- operator-private, never rendered
  set_at TEXT NOT NULL
)

recipes(
  id TEXT PRIMARY KEY,
  owner TEXT NOT NULL,
  name TEXT NOT NULL,
  document TEXT NOT NULL,         -- versioned JSON: filters + weights
  is_public INTEGER NOT NULL DEFAULT 0,
  created_at TEXT NOT NULL
)

dashboard_layouts(
  account TEXT PRIMARY KEY,
  slots TEXT NOT NULL,            -- JSON: slot name -> widget id + params
  updated_at TEXT NOT NULL
)
```

Weights, caps and exploration rates are **configuration** with documented
defaults (`discovery.*` in `lorehaven.toml.example`), not rows and not
constants.

### 5.4 Domain modules

- `crates/domain/src/discovery.rs`: an engine is a **pure scoring function**
  `(signals, candidates) -> Vec<(candidate, score, reason)>`. Ship three:
  `MoreLikeThis` (index terms + tags), `SameFandomFresh` (taxonomy + recency),
  `ReaderHistory` (the reader's own history/notes). Blending is deterministic
  and capped; the reason is engine-level only.
- `crates/domain/src/influence.rs`: applies `operator_affinities` as a
  multiplier inside the blend. The rule that has teeth: the parameter's name in
  code and config is neutral (`affinity_weight`), and no response field, log
  line the client can read, or UI label may distinguish an influenced result
  from an organic one. Tests assert response-shape equality between influenced
  and uninfluenced runs.
- `crates/domain/src/recipe.rs`: a recipe document is the M10 query AST plus
  engine weights; it validates against a fixed schema version and carries no
  code. A recipe cannot override another reader's opt-outs or visibility.

### 5.5 Repository `crates/db/src/discovery.rs`

- `recompute_taste_profile(account)` — derived from history, ratings, notes
  and bookmarks (all already exist); scheduled as a job, bounded input.
- `candidates_for(engine, profile, limit)` — bounded candidate SQL per engine;
  cap *before* scoring, scoring in the domain, not SQL.
- `affinities_for(work_ids)`, `save_recipe`, `public_recipes(cursor)`,
  `dashboard_layout_for`, `save_dashboard_layout`.
- Taste profiles are read only by their owner: no repository function takes
  "another account's profile" as an argument.

### 5.6 Routes

```text
GET  /api/v1/discovery                    (blended feed, cursor envelope)
GET  /api/v1/discovery/why/:workId        (engine-level reason only)
GET|PUT|DELETE /api/v1/me/taste-profile   (inspect; clear)
GET|POST /api/v1/recipes  GET /api/v1/recipes/:id
GET|PUT  /api/v1/me/dashboard             (slot layout)
POST /api/v1/operator/affinities          (operator role; audited; never
                                          linked from any public page)
```

### 5.7 Pages

- `frontend/src/routes/Discovery.svelte` — the feed, a "why am I seeing this"
  affordance per card (engine reason only), and the recipe switcher.
- `frontend/src/routes/Dashboard.svelte` — assemble widgets into the M1 slots;
  unknown/deprecated widget ids degrade to an empty slot, never an error page.
- Recipe editor as a section of Discovery or its own route; sharing a recipe
  makes it public and versioned.

### 5.8 Hand journey

1. Read two chapters of a fic; open Discovery → the feed reflects reading
   without a full reload.
2. The "why" affordance names the engine, never a multiplier.
3. Clear the taste profile → feed falls back to neutral/popular; profile read
   returns empty; history rows are untouched.
4. As operator, set an affinity on a low-read work; confirm: rankings move,
   and *no* visible field, label or reason changes shape.
5. Save a recipe, share it, load it as a second account with different
   opt-outs → it cannot show anything that account opted out of.

### 5.9 Acceptance tests

- no route returns another account's profile; a second account's feed is
  byte-identical whether or not the first has a profile (no shape leak)
- influence invisibility: influenced vs uninfluenced responses differ only in
  result order, never in field presence or naming
- diversity: a feed window respects the per-fandom cap and includes exploration
  slots (configuration-driven; the test asserts the cap holds)
- recipes honour the *viewer's* opt-outs and visibility
- dashboards with unknown widget ids render empty slots, not errors
- a cleared profile zeroes out without deleting the reader's history rows

### 5.10 Pitfalls

- **Silent means silent.** Spec §20's "author demand multiplier (silent)" rule
  originates here: if an influence is detectable from the response, the
  implementation is wrong. Test by diffing field sets, not by reading code.
- Credits (M15) must never write `operator_affinities` or ranking weights —
  when M15 lands, add the negative test there too.
- The profile is derived from behaviour the reader already controls; clearing
  clears the profile only, and says so. It never deletes history rows.
- Bound everything: candidate cap before scoring, cursor page size, bounded
  recompute jobs. A runaway feed query is a production incident on a
  self-hosted box.

---

## 6. Milestone 12 (repo) — Comments, forums, groups, messaging, presence

**Read first:** spec §17 in full, plus §0.3 (blocks must be honoured
everywhere) and the debt register (M2-06 lands here).

### 6.1 Why this is next

This milestone is the platform's social spine and the first milestone with a
**real-time** surface (presence), so it also introduces the SSE/WebSocket
pattern. M9's classifier becomes the gate for every new comment-shaped
surface built here.

### 6.2 Ledger rows

- `M12-01` comments on works and chapters through the positivity gate; threads,
  per-thread reply, soft delete, pseud-only posting (spec §17.1)
- `M12-02` forums: categories, topics, replies; moderated per trust level
  (spec §17.2–17.3)
- `M12-03` groups: membership, privacy (open/closed/hidden), roles, group
  forums (spec §17.4)
- `M12-04` messaging: 1:1 conversations, block-aware, report-capable
  (spec §17.5)
- `M12-05` presence: online indicators and typing states, opt-in, off by
  default (spec §17.6)
- `M12-06` block/mute become real on every path (spec §7.2.2; closes `M2-06`
  and the debt register)

### 6.3 Migration `0013_community.sql` (both dialects)

```text
comments(
  id TEXT PRIMARY KEY,
  subject_type TEXT NOT NULL,     -- 'work' | 'chapter' | 'topic' | 'user'
  subject_id TEXT NOT NULL,
  author_pseud TEXT NOT NULL,
  body TEXT NOT NULL,             -- raw text; render only via sanitiser
  body_version TEXT NOT NULL,
  created_at TEXT NOT NULL,
  edited_at TEXT,
  deleted_at TEXT                 -- soft delete; body retained for moderation
)
index (subject_type, subject_id, created_at)
index (author_pseud)

comment_threads(
  id TEXT PRIMARY KEY,
  subject_type TEXT NOT NULL,
  subject_id TEXT NOT NULL,
  root_comment TEXT NOT NULL,
  reply_count INTEGER NOT NULL DEFAULT 0
)

forum_categories(
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  position INTEGER NOT NULL,
  min_trust INTEGER NOT NULL DEFAULT 0
)

forum_topics(
  id TEXT PRIMARY KEY,
  category_id TEXT NOT NULL,
  author_pseud TEXT NOT NULL,
  title TEXT NOT NULL,
  created_at TEXT NOT NULL,
  last_post_at TEXT,
  locked INTEGER NOT NULL DEFAULT 0
)

forum_posts(
  id TEXT PRIMARY KEY,
  topic_id TEXT NOT NULL,
  author_pseud TEXT NOT NULL,
  body TEXT NOT NULL,
  created_at TEXT NOT NULL,
  deleted_at TEXT
)
index (topic_id, created_at)

groups(
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  privacy TEXT NOT NULL,          -- 'open' | 'closed' | 'hidden'
  owner TEXT NOT NULL,
  created_at TEXT NOT NULL
)

group_members(
  group_id TEXT NOT NULL,
  account TEXT NOT NULL,
  role TEXT NOT NULL,             -- 'owner' | 'moderator' | 'member'
  joined_at TEXT NOT NULL
)
primary key (group_id, account)

conversations(
  id TEXT PRIMARY KEY,
  created_at TEXT NOT NULL
)

conversation_participants(
  conversation_id TEXT NOT NULL,
  account TEXT NOT NULL,
  last_read_at TEXT,
  muted_until TEXT
)
primary key (conversation_id, account)

messages(
  id TEXT PRIMARY KEY,
  conversation_id TEXT NOT NULL,
  sender TEXT NOT NULL,
  body TEXT NOT NULL,
  sent_at TEXT NOT NULL,
  deleted_at TEXT
)
index (conversation_id, sent_at)

blocks(
  blocker TEXT NOT NULL,
  blocked TEXT NOT NULL,
  scope TEXT NOT NULL,            -- 'all' | 'messages' | 'comments'
  created_at TEXT NOT NULL,
  note TEXT
)
primary key (blocker, blocked, scope)

mutes(
  muter TEXT NOT NULL,
  muted TEXT NOT NULL,
  until TEXT,
  created_at TEXT NOT NULL
)
primary key (muter, muted)

presence(
  account TEXT PRIMARY KEY,
  last_seen_at TEXT NOT NULL,
  typing_until TEXT,
  enabled INTEGER NOT NULL DEFAULT 0   -- presence is opt-in
)
```

**DDL discipline note:** the sketch above is a field-meaning guide, not final
DDL — type the real DDL from spec §17's field definitions and ADR 0004's
storage rules, and keep the two dialects identical (the drift test catches a
miss).

### 6.4 Domain modules

- `crates/domain/src/community.rs`: thread shapes, edit windows, soft-delete
  semantics, posting rules (pseud-only), and the visibility matrix for
  open/closed/hidden groups (who may list, join, post, moderate).
- `crates/domain/src/blocking.rs`: the resolution function
  `blocked_between(viewer, subject, scope) -> bool` used by **every** social
  read/write path. Spec §7.2.2's rule has teeth here: a block that hides a
  comment but lets a mention through is not a block.

### 6.5 Repository `crates/db/src/community.rs`

- Comments: `insert_comment` (one transaction with the M9 classify call),
  `thread_for_subject` (cursor-paginated), `soft_delete_comment`,
  `edit_comment` (edit-window policy from the domain).
- Forums/groups/messaging: topic and post CRUD with trust gates from
  `crates/domain/src/trust.rs`'s thresholds (trust itself is built in M14 —
  until then the gate reads trust level 0 and the operator's manual role
  assignments), group membership transitions with the visibility matrix,
  conversation/message functions with `blocked_between` applied **before**
  send and **after** read.
- `blocked_between(blocker, blocked, scope)` and its mute twin; a **view** or
  a join guard, not scattered `WHERE` clauses — one query helper per surface.

### 6.6 Routes

```text
-- comments
GET  /api/v1/works/:id/comments            (cursor envelope; threaded)
POST /api/v1/works/:id/comments            (positivity gate)
POST /api/v1/comments/:id/delete           (author or subject owner)
-- forums
GET  /api/v1/forums  GET /api/v1/forums/:category/topics (cursor)
POST /api/v1/forums/:category/topics       (trust gate)
GET  /api/v1/topics/:id  POST /api/v1/topics/:id/replies
POST /api/v1/topics/:id/lock               (moderator)
-- groups
GET|POST /api/v1/groups  GET /api/v1/groups/:id
POST /api/v1/groups/:id/join|leave|role
-- messaging
GET  /api/v1/conversations  POST /api/v1/conversations
GET  /api/v1/conversations/:id/messages    (cursor; ascending)
POST /api/v1/conversations/:id/messages    (blocked_between first)
-- blocks and mutes
GET|POST|DELETE /api/v1/me/blocks  GET|DELETE /api/v1/me/mutes
-- presence
GET  /api/v1/presence/stream               (SSE; typing + online, opt-in)
```

### 6.7 Real-time pattern (first use; define it here, reuse later)

- **SSE over a single authenticated stream**, not WebSockets: one connection,
  server-pushed events (`typing`, `online`, `message`), auto-reconnect with
  `Last-Event-ID` resume from the client. No new daemon: the stream is an axum
  route reading an in-process broadcast channel; message fan-out also writes
  to `messages` so offline readers catch up by polling.
- Heartbeats every 30s; connections are dropped server-side after 2 missed
  beats. Stream handles are per-account, never per-pseud.
- All SSE payloads pass the same classification and privacy rules as REST;
  nothing over the stream that the REST surface would refuse.

### 6.8 Pages

- `Comments.svelte` (embedded in WorkPage/Reader): threads, reply, edit within
  the window, delete (tombstone), report affordance (M14 wires the backend).
- `Forums.svelte`, `ForumTopic.svelte`, `Groups.svelte`, `GroupPage.svelte`,
  `Messages.svelte` (list + thread pane), and a Settings section for blocks,
  mutes and the presence toggle.
- Every new page: loading, empty, error, success states; both `en` and `eo`
  labels; API types mirrored from server shapes.

### 6.9 Hand journey

1. Two accounts: A comments on B's work; B replies; A edits within the window;
   A deletes (tombstone shows for others).
2. A blocks B → B's comments vanish from A's views; B can still see their own;
   a message from B to A is refused at send with a **generic** error (never
   "you are blocked").
3. B @mentions A in a forum → no notification reaches A (block holds across
   paths).
4. Group: create closed group, second account requests, owner approves, member
   posts to the group forum; hidden group is invisible to non-members.
5. Presence: A opts in; B sees "typing" while A types; A opts out → B sees
   nothing, and the setting survives a reload.

### 6.10 Acceptance tests

- comment through the positivity gate: constructive held when the work's author
  opted out (the M9 rule applies to the new surface unchanged)
- block semantics across paths: comment hiding, message refusal, mention
  suppression, notification suppression — one block, every path
- mute expiry: after `until`, the muted account's content reappears
- group visibility matrix: hidden not listed, closed listed but join-gated,
  open joinable; non-member cannot read a hidden group's forum
- messaging pagination is stable under concurrent sends (cursor envelope)
- presence is opt-in: with `enabled=0`, no stream event names the account
- soft-deleted comment: hidden for others, retained for moderation, author sees
  their own tombstone

### 6.11 Pitfalls

- Do not run the positivity classifier again for content already classified and
  stored; store the classification id on the new row (surface change) instead.
- The block check is a **domain decision rendered as SQL**, not ad-hoc SQL per
  route: one helper per surface, tested once, reused everywhere. A route that
  hand-rolls its own block filter will be the one that leaks.
- Presence leaks via timestamps: a last-seen value is itself a presence signal.
  The opt-out must suppress derived signals too, not just the indicator.
- Mentions: parsing @pseud in bodies is a domain function with tests for
  lookalike names (unicode confusables) before any notification is queued.

---

## 7. Milestone 13 (repo) — Collections, challenges, requests, wishlists, events

**Read first:** spec §18 in full, plus §0.3 (a challenge entry is a work: the
positivity filter and pseud rules apply unchanged).

### 7.1 Why this is next

Challenge entries are works with a deadline; wishlist fulfilment is an import
or a write with a claimant. Both reuse the writing, publishing, importing and
positivity machinery that already exists instead of inventing parallel flows —
this milestone is mostly **composition**, which is why it follows M12 and not
the reverse.

### 7.2 Ledger rows

- `M13-01` collections: curated groupings with open/closed moderation and
  item-level inclusion policy (spec §18.1)
- `M13-02` challenges: prompts, schedules, open/closed entry windows,
  constraint checks (spec §18.2)
- `M13-03` requests/exchanges: claims, anonymous-until-reveal, assignment
  integrity (spec §18.3)
- `M13-04` wishlists: wanted stories with claims and fulfilment links (spec
  §18.4)
- `M13-05` writing events with a shared timeline and per-event rules (spec
  §18.5)

### 7.3 Migration `0014_events.sql` (both dialects)

```text
collections(
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  description TEXT,
  owner TEXT NOT NULL,
  item_policy TEXT NOT NULL,      -- 'owner_only' | 'open' | 'moderated'
  is_public INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL
)

collection_items(
  collection_id TEXT NOT NULL,
  work_id TEXT NOT NULL,
  added_by TEXT NOT NULL,
  added_at TEXT NOT NULL,
  note TEXT
)
primary key (collection_id, work_id)

challenges(
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  rules TEXT NOT NULL,            -- versioned JSON document (constraint list)
  schedule TEXT NOT NULL,         -- windows: opens/closes RFC 3339
  created_by TEXT NOT NULL,
  created_at TEXT NOT NULL
)

challenge_entries(
  challenge_id TEXT NOT NULL,
  work_id TEXT NOT NULL,
  entered_at TEXT NOT NULL,
  constraint_check TEXT NOT NULL  -- JSON: pass/fail per constraint, at entry
)
primary key (challenge_id, work_id)

requests(
  id TEXT PRIMARY KEY,
  requester TEXT NOT NULL,
  prompt TEXT NOT NULL,
  anonym_until TEXT,
  created_at TEXT NOT NULL
)

claims(
  request_id TEXT NOT NULL,
  claimant TEXT NOT NULL,
  claimed_at TEXT NOT NULL,
  fulfilled_by_work TEXT,
  fulfilled_at TEXT
)
primary key (request_id, claimant)

wishlists(
  account TEXT PRIMARY KEY,
  is_public INTEGER NOT NULL DEFAULT 0
)

wishlist_items(
  wishlist TEXT NOT NULL,
  node_id TEXT,                   -- taxonomy node (fandom/ship/tag) or work
  work_id TEXT,
  note TEXT,
  added_at TEXT NOT NULL
)

events(
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  document TEXT NOT NULL,         -- versioned JSON: rules, timeline, rewards
  created_by TEXT NOT NULL,
  created_at TEXT NOT NULL
)

event_participation(
  event_id TEXT NOT NULL,
  account TEXT NOT NULL,
  joined_at TEXT NOT NULL
)
primary key (event_id, account)
```

Constraint checks run at entry and again at close; a work edited after entry
re-checks (the M3 revision system supplies the trigger).

### 7.4 Domain modules

- `crates/domain/src/collections.rs`: item policy transitions (owner_only →
  moderated moves approval to the owner), the "add to collection" permission
  function, and the public-listing visibility rule (a private collection's
  existence is not confirmed to non-members).
- `crates/domain/src/challenges.rs`: constraint evaluation as data — a
  constraint is `{kind, params}`, evaluated against a work + taxonomy facts:
  word count bounds, required/forbidden tags, fandom, rating ceiling, deadline.
  No constraint type may execute code; evaluation is a pure match over kinds.
- `crates/domain/src/exchange.rs`: request/claim state machine
  (`open → claimed → fulfilled | expired`), assignment integrity (a request has
  at most one active claim), and the anonymity window.

### 7.5 Repository `crates/db/src/events.rs`

- Collection CRUD + `add_item` (policy-checked), `remove_item`, public listing
  with cursor envelope, private-collection existence guard (a read that would
  confirm a private collection's existence to a non-member returns the same
  404 as a nonexistent one).
- Challenge CRUD, `enter_work` (window + constraints in one transaction),
  re-check job at close (M5's `JobKind` extension), entry listing.
- Requests/claims: `claim_request` (at most one active claim — enforced in the
  write, not just a check-then-insert), `fulfil_with(work_id)`, `expire_claims`
  job.
- Wishlist functions honour `is_public`; a private wishlist's items are never
  in any public query path.

### 7.6 Routes

```text
GET|POST /api/v1/collections  GET|PUT|DELETE /api/v1/collections/:id
POST /api/v1/collections/:id/items  DELETE /api/v1/collections/:id/items/:workId
GET|POST /api/v1/challenges  GET|PUT /api/v1/challenges/:id
POST /api/v1/challenges/:id/entries
GET|POST /api/v1/requests  POST /api/v1/requests/:id/claims
POST /api/v1/claims/:id/fulfil
GET|POST /api/v1/wishlists/:account  (owner or public)
POST /api/v1/wishlist-items  DELETE /api/v1/wishlist-items/:id
GET|POST /api/v1/events  GET /api/v1/events/:id  POST /api/v1/events/:id/join
```

### 7.7 Pages

- `Collections.svelte`, `CollectionPage.svelte` — items, policy control,
  moderation queue for `moderated` collections.
- `Challenges.svelte`, `ChallengePage.svelte` — rules rendered from the
  versioned document, entry window state machine visible to the reader
  (opens in / open / closed), entry list.
- `Requests.svelte` (exchange board), `Wishlist.svelte`,
  `EventPage.svelte` (timeline, rules, participation).
- Reader-side affordances: "add to collection" on WorkPage; "fulfils" on a
  wishlist item linking to the work.

### 7.8 Hand journey

1. Create a moderated collection; a second account proposes an item; owner
   approves → item appears; a third account sees nothing pending.
2. Challenge with a word-count constraint: enter a compliant work (pass),
   enter a non-compliant work (refused with the failing constraint named),
   edit the work to break the constraint → re-check flags it at close.
3. Request with anonymous window: claimant writes and fulfils; requester's
   name hidden until reveal; after reveal both see identities.
4. Wishlist: add an item by taxonomy node; another reader claims it; fulfil
   with a new work; the wishlist owner receives the credit-relevant event
   (M15 consumes it later; M13 only emits it).
5. Event: join, see the timeline, participants list honours privacy settings.

### 7.9 Acceptance tests

- private collection: existence not confirmable by non-members (same 404 as
  nonexistent); membership grants read; leaving forfeits read
- moderated collection: proposer cannot self-approve; approval is the owner's
- challenge windows: entry refused outside the window with a timestamped
  reason; constraints evaluated at entry; re-check at close uses the revision
  current at close
- exchange: a request cannot hold two active claims; the same work cannot
  fulfil two claims from one claimant (anti-gaming, spec §18.3)
- wishlist: private wishlist items never appear in public queries
- event participation respects the reader's privacy settings on public lists

### 7.10 Pitfalls

- Constraint checks read the **taxonomy** snapshot current at entry; a later
  tag rename must not retroactively fail an entry (record the node ids used).
- Anonymous windows are enforced in the query layer, not the UI: the requester
  identity is absent from the response, not merely hidden.
- Anti-gaming rules (one work, one claim) are database constraints or
  transactions, not UI warnings.
- The "fulfils" link is a claim record, not free text: fulfilment without a
  claim is refused.

---

## 8. Milestone 14 (repo) — Trust, reports, quorum, appeals, sanctions

**Read first:** spec §19 in full, plus §0.2 (priorities 5–6) and §0.3 (no
purchased trust; credits can never touch this milestone's tables — M15 will
add the negative test).

### 8.1 Why this is next

M12's report affordances and M13's anti-gaming needs both point here; M15's
credits must never purchase trust, so the trust model must exist and be
tested before the economy mints anything. This milestone also lands the
`M6-10` preservation batches (debt register) behind a documented permission
basis.

### 8.2 Ledger rows

- `M14-01` trust levels TL0–TL6 with documented progression and regress
  criteria, derived from behaviour records, never purchases (spec §19.1)
- `M14-02` reports: any user-visible content reportable; queue with quorum
  review for account-level outcomes (spec §19.2)
- `M14-03` quorum: multiple reviewers, independence rules, audit trail, no
  self-review of own content (spec §19.3)
- `M14-04` appeals: one active appeal per sanction; evidence-based; resolved
  by reviewers who did not issue the sanction (spec §19.4)
- `M14-05` sanctions: staged (rate-limit → shadow → suspend), always
  time-bounded, with expiry and automatic regress (spec §19.5)
- `M14-06` process feedback: closed-loop feedback on moderation outcomes to
  refine written policy, not per-case overrides (spec §19.6)

### 8.3 Migration `0015_governance.sql` (both dialects)

```text
trust_levels(
  account TEXT PRIMARY KEY,
  level INTEGER NOT NULL,           -- 0..6
  computed_at TEXT NOT NULL,
  basis TEXT NOT NULL               -- JSON: behaviour records behind it
)

reports(
  id TEXT PRIMARY KEY,
  subject_type TEXT NOT NULL,       -- comment|review|work|message|profile|user
  subject_id TEXT NOT NULL,
  reporter TEXT NOT NULL,
  reason TEXT NOT NULL,             -- enum from config (+ free text field)
  created_at TEXT NOT NULL,
  state TEXT NOT NULL,              -- open|in_review|resolved|dismissed
  resolved_at TEXT,
  resolution TEXT NOT NULL
)
index (state, created_at)

review_tasks(
  id TEXT PRIMARY KEY,
  reviewer TEXT NOT NULL,
  report_id TEXT NOT NULL,
  assigned_at TEXT NOT NULL,
  decided_at TEXT,
  outcome TEXT NOT NULL             -- uphold|dismiss|escalate|recuse
)
index (report_id)

sanctions(
  id TEXT PRIMARY KEY,
  account TEXT NOT NULL,
  kind TEXT NOT NULL,               -- rate_limit|shadow|suspend
  reason_ref TEXT NOT NULL,         -- report or review task id
  starts_at TEXT NOT NULL,
  ends_at TEXT,                     -- NULL = until appeal or operator action
  issued_by TEXT NOT NULL,
  lifted_at TEXT,
  lifted_by TEXT
)
index (account, starts_at)

appeals(
  id TEXT PRIMARY KEY,
  sanction_id TEXT NOT NULL,
  appellant TEXT NOT NULL,
  statement TEXT NOT NULL,
  created_at TEXT NOT NULL,
  state TEXT NOT NULL,              -- open|decided
  decided_at TEXT,
  decision TEXT NOT NULL,           -- upheld|reduced|overturned
  decided_by TEXT NOT NULL          -- a reviewer who did not issue it
)

audit_log(
  id TEXT PRIMARY KEY,
  actor TEXT NOT NULL,
  action TEXT NOT NULL,
  subject_type TEXT NOT NULL,
  subject_id TEXT NOT NULL,
  document TEXT NOT NULL,
  created_at TEXT NOT NULL
)
index (subject_type, subject_id, created_at)

operator_role(
  account TEXT PRIMARY KEY,
  role TEXT NOT NULL,               -- 'operator' | 'preservation_officer'
  granted_at TEXT NOT NULL
)

preservation_batches(
  id TEXT PRIMARY KEY,
  source TEXT NOT NULL,
  query TEXT NOT NULL,
  destination TEXT NOT NULL,        -- public archive only (M6's guard holds)
  dry_run INTEGER NOT NULL,
  approval_basis TEXT NOT NULL,     -- permission basis + quorum record
  approved_by TEXT NOT NULL,
  ran_at TEXT,
  summary TEXT                      -- JSON: what a dry run found
)
```

### 8.4 Domain modules

- `crates/domain/src/trust.rs`: TL0–TL6 constants, progression function
  `(behaviour records) -> level` with **documented, configuration-specified**
  criteria; regress on sanctions; never a purchase input. Trust never grants
  permissions — only the ceilings spec §19 already fixes.
- `crates/domain/src/quorum.rs`: assignment with independence (no self-review,
  recusal for same-work conflicts, no sanction-issuer deciding its appeal),
  quorum size from configuration, and the "no quorum available" degraded
  state that parks the report for the operator instead of letting one
  reviewer decide alone.

### 8.5 Repository `crates/db/src/governance.rs`

- Trust: `trust_for(account)`, `recompute_trust(account)` (job, bounded), a
  behaviour-records read that never joins against purchases.
- Reports/review tasks: `open_report` (idempotent per reporter+subject while
  open), `assign_task` (independence-checked), `decide_task`, quorum tally.
- Sanctions: `issue_sanction` (staged kinds only; ends_at required except
  operator-held suspends), `lift_sanction`, `expire_sanctions` job with
  automatic trust regress.
- Appeals: `open_appeal` (one active per sanction), `decide_appeal`
  (independence enforced in the write).
- `audit_log` append-only; **every** governance mutation writes one row in
  the same transaction.
- Preservation: `create_batch` (dry-run first), `record_dry_run`,
  `approve_batch` (operator role + approval basis document required),
  `run_batch` (reuses M6's import machinery; destination guard unchanged).

### 8.6 Routes

```text
POST /api/v1/reports                        (any user-visible content)
GET  /api/v1/moderation/queue               (reviewer role)
POST /api/v1/moderation/reviews/:taskId/decide|recuse
GET  /api/v1/moderation/reports/:id         (reviewer role)
POST /api/v1/sanctions  POST /api/v1/sanctions/:id/lift
POST /api/v1/appeals  GET /api/v1/me/appeals
GET  /api/v1/me/trust                       (own level + the written criteria)
POST /api/v1/operator/preservation-batches  (operator role; dry-run first)
POST /api/v1/operator/preservation-batches/:id/approve|run
```

### 8.7 Pages

- `Report.svelte` (or an embedded dialog on every reportable surface —
  WorkPage, Comments, Messages, Profiles).
- `Moderation.svelte` — the reviewer queue with task detail, quorum state,
  decision/recuse affordances; a supervisor view for the operator showing
  audit log entries (read-only).
- `Sanctions.svelte`/appeals pages under settings; `Trust.svelte` — the
  reader's own level and the written criteria, always visible, no hidden
  thresholds.
- Operator: preservation batch console (dry run → review summary → approve →
  run), surfacing M6's dry-run report format.

### 8.8 Hand journey

1. Reader reports a comment; two reviewers pick it up; quorum upholds →
   sanction issues automatically at the staged floor; audit rows exist for
   every step.
2. The sanctioned author appeals; a reviewer who did not issue decides; the
   sanction is reduced; trust regresses less than a suspension would.
3. A reviewer attempts to review a report on their own work → the UI refuses
   and the server refuses (`recuse` is the only path).
4. Operator runs a preservation batch: dry run (summary shows what would be
   imported), approve with basis document, run, items land in the public
   archive; the audit trail names the basis.
5. Sanction expiry: a time-bounded rate-limit ends; the account's effective
   limits return without operator action.

### 8.9 Acceptance tests

- quorum: an account-level outcome requires N independent reviewers;
  a single reviewer cannot decide alone (the write refuses)
- independence: self-review refused; sanction issuer cannot decide its appeal
- sanctions are always time-bounded except operator-held; expiry job restores
  prior limits; trust regress applies and is recorded in `basis`
- one active appeal per sanction; a decided appeal cannot reopen
- every governance mutation produced exactly one audit row (count them)
- preservation batch: run without dry-run summary or without approval basis
  is refused; destination is the public archive only
- reports are idempotent per reporter+subject while open

### 8.10 Pitfalls

- Quorum size 1 is not quorum. Configure minimums in config and refuse to
  start with impossible values (`doctor`).
- The audit log is append-only: no UPDATE path exists for it, in code or in
  migrations. If a test can update a row, the schema is wrong.
- Never display quorum reviewer identities to the reported account, and never
  show the reporter's identity to the reported account. Anonymity of process
  protects everyone; record it in `audit_log`, not in user-facing responses.
- Trust recomputation must read behaviour records only; a join to credits
  (M15) is the one invariant M15's negative tests will check — keep the
  schemas separate so the join cannot even be written naturally.

---

## 9. Milestone 15 (repo) — Credits, fair queues, bounties, billing

**Read first:** spec §20 in full (the credit economy table and the quote →
reserve → submit → complete → capture flow), plus §0.3 (credits never purchase
trust, ranking, or moderation authority) and §19.5.

### 9.1 Why this is next

Priority jobs (imports, conversions, translation, AI) already exist behind M5's
job queue and M7's exports; M17 and M18 will price translation and AI through
this milestone's quote flow. The economy also powers event rewards (M13
emitted the events) — but the founding invariant is negative: **the economy
changes ceilings, never permissions** (spec §19.5).

### 9.2 Ledger rows

- `M15-01` double-entry credit ledger with idempotency keys and balanced
  transactions (spec §20.1)
- `M15-02` job charging: quote → reserve → submit → complete → capture actual,
  with release or documented partial charge on failure (spec §20.2)
- `M15-03` the initial credit economy table wired to real actions with per-
  action daily caps (spec §20.3)
- `M15-04` daily action caps by tier (50/75/100) and author caps (100/work/day,
  300/author/day, 5,000/author/month) (spec §20.3)
- `M15-05` fair queues: priority is paid, order within a class stays
  first-come, queue position is observable (spec §20.4)
- `M15-06` bounties: escrow on a job's outcome, released on completion
  (spec §20.5)
- `M15-07` subscriptions and billing providers behind one interface, with
  grants that change ceilings only (spec §20.6–20.7)

### 9.3 Migration `0016_economy.sql` (both dialects)

```text
credit_transactions(
  id TEXT PRIMARY KEY,
  type TEXT NOT NULL,             -- earn|spend|grant|purchase|hold|release|capture
  idempotency_key TEXT NOT NULL,  -- unique; the client supplies it
  reference TEXT NOT NULL,        -- job id, work id, event id...
  created_at TEXT NOT NULL
)
unique (idempotency_key)

credit_entries(
  transaction_id TEXT NOT NULL,
  account TEXT NOT NULL,
  bucket TEXT NOT NULL,           -- earned|granted|purchased|held
  amount_bp INTEGER NOT NULL,     -- signed; debits negative; balances check
  created_at TEXT NOT NULL
)
index (account, created_at)

credit_holds(
  id TEXT PRIMARY KEY,
  account TEXT NOT NULL,
  amount INTEGER NOT NULL,
  job_id TEXT NOT NULL,
  expires_at TEXT NOT NULL,       -- reserve TTL; release on expiry
  released_at TEXT,
  captured_at TEXT
)
index (job_id)

queue_slots(
  job_id TEXT PRIMARY KEY,
  priority_class TEXT NOT NULL,   -- free|priority|subscription
  position INTEGER NOT NULL,      -- monotonic within class
  enqueued_at TEXT NOT NULL
)

bounties(
  id TEXT PRIMARY KEY,
  job_kind TEXT NOT NULL,
  terms TEXT NOT NULL,            -- versioned JSON
  escrow_transaction TEXT NOT NULL,
  state TEXT NOT NULL,            -- open|claimed|paid|expired
  claimant TEXT,
  created_by TEXT NOT NULL,
  created_at TEXT NOT NULL
)

subscriptions(
  account TEXT NOT NULL,
  tier TEXT NOT NULL,             -- reader|author|curator
  state TEXT NOT NULL,            -- active|past_due|canceled
  period_end TEXT NOT NULL,
  provider TEXT,                  -- billing provider id, nullable in dev
  external_ref TEXT
)
primary key (account, tier)

usage_counters(
  account TEXT NOT NULL,
  action TEXT NOT NULL,           -- the capped action key
  day TEXT NOT NULL,              -- UTC date
  count INTEGER NOT NULL DEFAULT 0
)
primary key (account, action, day)
```

The economy values in spec §20.3's table are **configuration** (`economy.*`
in `lorehaven.toml.example`) with the spec values as documented defaults —
the operator tunes, the code reads config, nothing is hardcoded.

### 9.4 Domain modules

- `crates/domain/src/ledger.rs`: transaction/entry invariants — a transaction
  is balanced across buckets (sum of entries = 0, holds excluded), an
  idempotency key replays the **same** transaction (replay with a different
  payload is an error, not a second write), bucket precedence for spending
  (held → earned → granted → purchased, expiring first where applicable).
- `crates/domain/src/charging.rs`: the quote flow state machine
  (`quoted → reserved → submitted → completed|failed`) with the capture rule
  (actual charge may be less; more requires a new quote) and the failure rule
  (release the hold, or apply the documented partial charge).
- `crates/domain/src/fairqueue.rs`: position assignment per class; a paid job
  never jumps ahead of an enqueued paid job in the same class; position is
  observable to the job's owner. The queue is fair *within* classes; classes
  differ only in relative order, which the config publishes.
- `crates/domain/src/caps.rs`: the daily/monthly cap evaluation over
  `usage_counters`, including the "credits show the cap and the effective
  value" semantics — the interface must let a preference (or cap) never look
  like it took effect when it did not.

### 9.5 Repository `crates/db/src/economy.rs`

- Ledger: `post_transaction(entries, idempotency_key) -> TxnId` — one
  transaction, both dialects; replay-safe via the unique key; balanced-entry
  check in the same transaction.
- Holds: `reserve(account, job, quote)`, `capture(hold, actual)`, `release(hold)`,
  `expire_holds` job (TTL).
- Caps: `bump_counter(account, action) -> (count, cap)` — atomic increment
  with the cap check inside the statement pair; `usage_for(account, day)`.
- Queue: `enqueue_classed(job, class)`, `next_in_class(class, worker)`,
  `position_of(job_id)` — position assignment in the same transaction as
  enqueue.
- Subscriptions: `active_tiers(account)`, `grant_periodic` (job), provider
  callbacks recorded through the idempotency key.
- Every economy write goes through the ledger module — no direct table writes
  from routes or workers.

### 9.6 Routes

```text
GET  /api/v1/me/credits               (balances by bucket + recent ledger)
GET  /api/v1/me/credits/quote?kind=...&params=...   (job quote)
POST /api/v1/me/credits/reserve       (hold for a job)
GET  /api/v1/jobs/:id/queue-position  (observable position)
GET  /api/v1/me/usage                 (cap + effective count per action)
GET|POST /api/v1/bounties  POST /api/v1/bounties/:id/claim|pay
GET|POST /api/v1/me/subscription      (status; provider checkout handled by
                                       the provider adapter, not this route)
```

Job submission paths (imports, exports, translation, AI) call
`charging::quote_flow` server-side; the quote is an API response the client
shows *before* submit, per §20.2's consent rule.

### 9.7 Pages

- `Credits.svelte` — balances by bucket, recent transactions, usage vs caps
  (the cap and the effective value, always both).
- Quote-and-reserve flow on the existing Jobs/Import/Exports pages: show the
  quote, get consent, submit, show queue position, show capture receipt.
- `Bounties.svelte` — open bounties, claim, pay on completion.
- Subscription page: tiers, state, period end; the provider checkout lives in
  the provider's hosted flow — the app only records the outcome.

### 9.8 Hand journey

1. Earn: daily login grant; read a chapter (capped); react (capped) — the
   ledger shows balanced transactions, replaying the same idempotency key does
   not double-credit.
2. Spend: request a priority import quote → consent → reserve → job runs →
   capture actual ≤ quote; receipt matches the ledger.
3. Failure: kill the worker mid-job → hold released (or documented partial
   charge), never a silent loss.
4. Fair queue: enqueue 3 free jobs and 1 priority job; the priority job runs
   first, the free jobs keep their arrival order; positions observable.
5. Caps: exhaust a daily cap; the action refuses with cap and effective count
   shown, and the refusal is honest (no partial credit taken).
6. Bounty: escrow, claim, complete job, pay; expiry returns escrow.

### 9.9 Acceptance tests

- ledger invariants: every transaction balances; idempotency replay returns
  the same result; a replay with a different payload is refused
- holds: capture ≤ reserve; release returns the full hold; expiry releases
- caps: per-action daily caps enforced server-side; tier ceilings (50/75/100)
  and author caps enforced; counters roll over at UTC midnight
- fair queue: paid never jumps a same-class peer; free jobs preserve arrival
  order; position observable and monotonic
- bounty escrow released on expiry, paid exactly once on completion
- subscription grants change ceilings only: no new permission appears with a
  tier change (test the permission matrix before/after)
- **negative invariant (the founding one):** no code path — purchase, bounty,
  subscription or leaderboard — writes to `trust_levels`, `operator_role`,
  taxonomy ranking weights, or `operator_affinities`. Write the test that
  proves the join doesn't exist.

### 9.10 Pitfalls

- Money-like tables get money-like discipline: every mutation in one
  transaction, idempotency keys on every external callback, no "adjust later"
  writes. A ledger you can't reconcile is a ledger you can't refund from.
- **No raw balance column.** Balances are derived from entries (or cached
  totals verified against them); a mutable balance column invites drift.
- The purchase provider is behind one trait (`BillingProvider`) with a dev
  implementation (manual grant) and a stub for the real one; a paid feature
  must never call the provider directly.
- Leaderboard rewards (spec §20.3) are periodic jobs writing earned credits —
  they must not write ranking weights either. Same negative test covers it.

---

## 10. Milestone 16 (repo) — Marketplace, extension isolation, webhooks, gallery

**Read first:** spec §21 in full, plus the M16 memory-tier budgets referenced
from §19 (trust grants no exemptions; the isolation budget is per-extension).

### 10.1 Why this is next

The marketplace monetises the craft surfaces M9–M15 built (paid works,
commission listings) and opens the platform to third-party code — which is
exactly why it comes after the trust and economy invariants exist to constrain
it. Extensions are the largest new attack surface in the build; the isolation
budget is the milestone's real deliverable, the storefront is a catalog over
existing flows.

### 10.2 Ledger rows

- `M16-01` marketplace listings: paid works, commissions with a quote and
  state machine, billed only through the M15 ledger (spec §21.1–21.2)
- `M16-02` reader community marketplace: listings, ask/response, moderation
  via the M14 report path (spec §21.3)
- `M16-03` extensions: versioned manifests, capability grants, explicit
  reader consent, revocation (spec §21.4)
- `M16-04` extension isolation: dedicated workers, cgroup/seccomp profile,
  memory tiers (150/300/600 MiB), no ambient network, W3C-style origin
  separation, kill-on-overrun (spec §21.4)
- `M16-05` webhooks: HMAC-signed, bounded payload, user-installed, replay
  protection (spec §21.5)
- `M16-06` gallery pages: embedded rich content via the sanitiser's gallery
  extension only, reader-blocking honoured (spec §21.6)

### 10.3 Migration `0017_marketplace.sql` (both dialects)

```text
listings(
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,             -- paid_work | commission | ask
  owner TEXT NOT NULL,
  work_id TEXT,                   -- paid_work
  terms TEXT NOT NULL,            -- versioned JSON: price points, turnaround
  state TEXT NOT NULL,            -- draft|active|paused|closed
  created_at TEXT NOT NULL
)
index (kind, state, created_at)

commissions(
  id TEXT PRIMARY KEY,
  listing_id TEXT NOT NULL,
  client TEXT NOT NULL,
  state TEXT NOT NULL,            -- quoted|accepted|in_progress|delivered|
                                  -- accepted_final|refunded|disputed
  quote_transaction TEXT,         -- M15 ledger reference
  delivery_work TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
)

extension_manifests(
  id TEXT PRIMARY KEY,            -- slug
  version TEXT NOT NULL,
  document TEXT NOT NULL,         -- the manifest, versioned, signed hash
  submitted_by TEXT NOT NULL,
  state TEXT NOT NULL,            -- pending|approved|rejected|revoked
  created_at TEXT NOT NULL
)
index (id, version)

extension_grants(
  account TEXT NOT NULL,
  manifest_id TEXT NOT NULL,
  version TEXT NOT NULL,
  capabilities TEXT NOT NULL,     -- JSON: the granted capability list
  granted_at TEXT NOT NULL,
  revoked_at TEXT
)
primary key (account, manifest_id)

webhook_endpoints(
  id TEXT PRIMARY KEY,
  owner TEXT NOT NULL,
  url TEXT NOT NULL,
  secret TEXT NOT NULL,           -- HMAC key; store like a password
  events TEXT NOT NULL,           -- JSON list of subscribed event types
  active INTEGER NOT NULL DEFAULT 1,
  created_at TEXT NOT NULL
)

webhook_deliveries(
  id TEXT PRIMARY KEY,
  endpoint_id TEXT NOT NULL,
  event_id TEXT NOT NULL,
  payload TEXT NOT NULL,          -- bounded; the bound is configuration
  signature TEXT NOT NULL,
  attempted_at TEXT NOT NULL,
  status TEXT NOT NULL,           -- ok|retrying|failed
  attempts INTEGER NOT NULL DEFAULT 0
)
index (endpoint_id, attempted_at)

gallery_items(
  id TEXT PRIMARY KEY,
  work_id TEXT NOT NULL,
  owner TEXT NOT NULL,
  media_type TEXT NOT NULL,
  storage_key TEXT NOT NULL,      -- M5 object storage; presigned only
  alt_text TEXT NOT NULL,
  sanitized_document TEXT NOT NULL,  -- the sanitiser's gallery output
  created_at TEXT NOT NULL
)
index (work_id)
```

### 10.4 Domain modules

- `crates/domain/src/marketplace.rs`: listing/commission state machines with
  the rule that **money moves only through the M15 ledger** — a commission
  transition that implies payment names its `quote_transaction`; the refund
  path references the original capture.
- `crates/domain/src/extension.rs`: manifest schema (versioned), capability
  vocabulary (`storage.read`, `work.read`, `webhook.send`, …), the grant
  subset rule (a grant can only narrow a manifest's requested capabilities),
  consent record, revocation. The manifest **cannot** request ambient network
  access or arbitrary process control — the vocabulary has no such capability,
  so the request cannot parse.
- `crates/domain/src/webhook.rs`: event envelope (type, id, created_at,
  payload), HMAC signing over the serialised envelope, the bounded-payload
  rule (bound from configuration), and replay protection via `event_id` +
  timestamp window.

### 10.5 Repository `crates/db/src/marketplace.rs`

- Listings/commissions CRUD + state transitions; each transition that implies
  money movement requires and records the ledger reference (the write refuses
  without one).
- Extensions: manifest CRUD, grants with the subset rule checked in the
  write, revocation (revocation must **disable** the extension's scheduled
  jobs and webhooks in the same transaction).
- Webhooks: endpoint CRUD, `record_delivery`, `pending_deliveries` (the
  webhook worker's input), retry accounting.
- Gallery: `add_gallery_item` (sanitised document required; the sanitiser's
  gallery extension output is the only path), presigned URL issuance at read.

### 10.6 Extension isolation (the real deliverable)

- A dedicated worker binary (`crates/app/src/bin/extension_worker.rs`) spawned
  per job with: its own cgroup (memory tier from configuration: 150 / 300 /
  600 MiB), seccomp profile (no exec, no new namespaces, no raw sockets),
  no ambient network namespace (network only via the host's capability
  broker), its own tmpfs, wall-clock kill timer.
- The capability broker is an in-process channel to the host with **no
  ambient authority**: an extension must present a grant token per call; the
  broker validates the grant subset before any host call.
- Memory tier selection: manifest-declared tier, capped by configuration;
  overrun → kill + event to the owner + trust-relevant record for M14 (the
  kill is not a sanction; repeated overruns are).
- Kill-on-overrun must leave no partial writes: the broker serialises host
  mutations so a killed extension's transaction aborts.
- OSS note (spec §0.5.2): Landlock restricts filesystem reach; seccomp
  restricts syscalls; cgroups bound memory. On the dev machine without
  Landlock, the tests still pass — the profile is **best-effort documented
  and tested where the kernel allows**, never silently absent.

### 10.7 Routes

```text
GET|POST /api/v1/listings  GET|PUT|DELETE /api/v1/listings/:id
POST /api/v1/listings/:id/commissions  POST /api/v1/commissions/:id/accept|deliver|accept-final|refund
GET  /api/v1/extensions  GET /api/v1/extensions/:slug
POST /api/v1/extensions/:slug/grant  POST /api/v1/extensions/:slug/revoke
GET  /api/v1/me/extension-grants
GET|POST|DELETE /api/v1/me/webhooks  (bounded payload; secret shown once)
GET  /api/v1/works/:id/gallery  POST /api/v1/works/:id/gallery
GET  /api/v1/gallery-items/:id/media   (presigned, short TTL)
```

### 10.8 Pages

- `Marketplace.svelte` (listings), `ListingPage.svelte` (quote → accept
  state machine visible), commission thread view with receipt links into
  `Credits.svelte`.
- `Extensions.svelte` (gallery of extensions, manifest view, consent screen
  naming each capability in plain language), `MyExtensions.svelte` (grants,
  revoke).
- `Webhooks.svelte` (endpoints, delivery log, redelivery affordance).
- `Gallery.svelte` sections on WorkPage: sanitiser-rendered rich items,
  reader-side block-aware (a blocked owner's gallery items do not render).

### 10.9 Hand journey

1. Author lists a commission with terms; reader requests quote → accept →
   deliver → accept-final; the ledger shows exactly two balanced transactions
   (charge and payout); the receipt links resolve.
2. Refund path: dispute → refund; the refund transaction references the
   original capture; balances reconcile.
3. Install an extension: consent screen lists capabilities; install; run a
   job; revoke — scheduled jobs and webhooks stop in the same transaction.
4. Overrun: an extension exceeding its memory tier is killed; the owner sees
   the event; nothing partial is stored.
5. Webhook: install an endpoint, trigger an event, verify the HMAC signature
   with the stored secret, replay the same delivery → refused by `event_id`.
6. Gallery: embed an image with alt text; a reader who blocked the owner
   sees the gallery items suppressed.

### 10.10 Acceptance tests

- commission money movements are ledger transactions (no transition stores a
  price without a ledger reference; refund references the capture)
- extension grant subset rule: a grant cannot exceed the manifest request
- revocation stops scheduled jobs and webhooks atomically
- extension worker: memory overrun kills the process, emits the owner event,
  aborts partial writes; seccomp/cgroup profile applied where the kernel
  allows, and the deviation is **documented** in `docs/verification.md`
- webhooks: signature verifies; payload bound enforced; replay refused;
  failed delivery retries with backoff and then parks as `failed`
- gallery renders only sanitiser output; blocked owner's items suppressed
- marketplace surfaces are reportable and the report flows into M14's queue

### 10.11 Pitfalls

- The extension runtime is the risk: prototype the worker **first**, with a
  "hello" extension under each memory tier, before building any storefront
  UI. If the isolation story cannot be demonstrated, stop and escalate rather
  than shipping a weaker sandbox quietly.
- Never sign webhooks over a payload the bound would reject — truncate at the
  boundary and document it, don't silently drop events.
- Grant tokens are per-call and short-lived; a long-lived token in an
  extension's hands is an ambient authority by another name.
- Commission "delivery" is a work reference (M3), not an upload into the
  commission row; the delivery flow publishes a draft like any other work.

---

## 11. Milestone 17 (repo) — Translation pipeline

**Read first:** spec §22 in full, plus §12's classifier (translation reviews
pass through it) and §20's quote flow (priced via M15).

### 11.1 Why this is next

Translation is a **job pipeline** over content that already exists: chapters
(M3), jobs (M5), the positivity gate (M9), the taxonomy for language facets
(M10), credits (M15) and the public API surface (M18 consumes it). It is
stacked here, before the API milestone, so M18 can expose translation state
without re-opening content routes.

### 11.2 Ledger rows

- `M17-01` translation pipeline: language detection, translation jobs, memory
  and glossary, review gates, publication as sibling works (spec §22.1–22.3)
- `M17-02` translation memory: paragraph-level reuse across works; the author
  controls inclusion of their own memory (spec §22.2)
- `M17-03` review gates: each translation passes reviewer gates with the
  positivity classifier applied to review text (spec §22.3)
- `M17-04` provenance: machine vs human, model/version recorded, shown to
  readers per §21's disclosure rules (spec §22.4)
- `M17-05` priced via the M15 quote flow with consent before submit
  (spec §22.5)

### 11.3 Migration `0018_translation.sql` (both dialects)

```text
translation_jobs(
  id TEXT PRIMARY KEY,
  source_work TEXT NOT NULL,
  source_lang TEXT NOT NULL,       -- BCP-47
  target_lang TEXT NOT NULL,
  provider TEXT NOT NULL,          -- 'human' | 'machine:<provider>:<version>'
  quote_transaction TEXT,          -- M15 ledger (machine jobs)
  state TEXT NOT NULL,             -- quoted|reserved|in_progress|in_review|
                                   -- approved|published|failed|cancelled
  created_by TEXT NOT NULL,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL
)
index (source_work, target_lang)

translation_units(
  id TEXT PRIMARY KEY,
  job_id TEXT NOT NULL,
  chapter_id TEXT NOT NULL,
  paragraph_index INTEGER NOT NULL,
  source_text TEXT NOT NULL,
  target_text TEXT,
  state TEXT NOT NULL,             -- pending|translated|reviewed|approved
  memory_hit TEXT                  -- the memory entry id reused, if any
)
index (job_id, chapter_id, paragraph_index)

translation_memory(
  id TEXT PRIMARY KEY,
  owner TEXT NOT NULL,             -- the author whose memory this is
  source_lang TEXT NOT NULL,
  target_lang TEXT NOT NULL,
  source_hash TEXT NOT NULL,       -- normalised paragraph hash
  source_text TEXT NOT NULL,
  target_text TEXT NOT NULL,
  quality_bp INTEGER NOT NULL,     -- reviewer-assessed, 0..10000
  created_at TEXT NOT NULL
)
index (owner, source_lang, target_lang, source_hash)

translation_glossaries(
  id TEXT PRIMARY KEY,
  owner TEXT NOT NULL,             -- work owner or group
  work_id TEXT,                    -- scoped to a work when present
  source_lang TEXT NOT NULL,
  target_lang TEXT NOT NULL,
  term TEXT NOT NULL,
  translation TEXT NOT NULL,
  case_sensitive INTEGER NOT NULL DEFAULT 0
)
index (owner, source_lang, target_lang)

translation_reviews(
  id TEXT PRIMARY KEY,
  job_id TEXT NOT NULL,
  reviewer TEXT NOT NULL,
  gate TEXT NOT NULL,              -- 'linguistic' | 'cultural' | 'final'
  state TEXT NOT NULL,             -- pending|approved|changes_requested
  notes TEXT,
  created_at TEXT NOT NULL,
  decided_at TEXT
)

translation_publications(
  id TEXT PRIMARY KEY,
  job_id TEXT NOT NULL,
  work_id TEXT NOT NULL,           -- the sibling work created by M3 flows
  published_at TEXT NOT NULL
)
```

### 11.4 Domain modules

- `crates/domain/src/translation.rs`: job state machine; unit segmentation
  (paragraph boundaries from M3's chapter model, stable indices so a re-run
  reuses unchanged paragraphs); memory lookup (exact hash, then fuzzy with the
  M10 similarity floor); glossary application order (exact → case-insensitive
  → longest match); quality thresholds per gate from configuration.
- `crates/domain/src/translation_policy.rs`: who may request a translation of
  a work (the author, or an authorised translator via M3's collaboration
  model), whose memory may be consulted (only the author's, or opted-in
  shared), and the disclosure rule — the published sibling work carries
  provenance (machine/human, model, version) in its front matter, rendered by
  WorkPage per §21's disclosure requirements.

### 11.5 Repository `crates/db/src/translation.rs`

- Jobs: `create_job` (quote attached for machine jobs), `transition_job`
  (state machine in the write), `units_for_job`, `upsert_unit`,
  `publish_job` (creates the sibling work through M3's normal publish path —
  never a direct content-table write).
- Memory: `lookup_memory(owner, langs, hash)`, `fuzzy_memory(...)`, `add_memory`
  (quality from review), `memory_opt_in(owner, share)` — inclusion in others'
  lookups is the owner's choice.
- Glossaries: CRUD + `apply_glossary(text, direction)`.
- Reviews: `open_review_gate`, `decide_review_gate` (notes pass the M9
  classifier before storage, like any comment).

### 11.6 Routes

```text
POST /api/v1/works/:id/translations            (quote → consent → job)
GET  /api/v1/translations/:jobId               (state, units progress)
GET  /api/v1/translations/:jobId/reviews       (reviewer role)
POST /api/v1/translations/:jobId/reviews/:gate/decide
POST /api/v1/translations/:jobId/publish
GET|PUT /api/v1/me/translation-memory          (inspect, opt-in/out, clear)
GET|POST|DELETE /api/v1/me/translation-glossaries
GET  /api/v1/works/:id/translations            (public: published siblings)
```

### 11.7 Pages

- `Translate.svelte` (author/translator): request a translation with quote
  consent, watch unit progress, per-gate review queue with diff view
  (source/target paragraph pairs), publish with provenance preview.
- WorkPage: published translations listed as siblings with the provenance
  badge (machine/human + model/version per §21 disclosure).
- `TranslationMemory.svelte` (settings): opt-in/out, inspect entries, clear.

### 11.8 Hand journey

1. Author requests a machine translation of a 3-chapter work → quote shown →
   consent → job runs; unchanged paragraphs on re-run reuse memory (verify
   `memory_hit`).
2. Glossary: add a term; re-run; the term's translation appears verbatim.
3. Review gates: linguistic reviewer requests changes; notes go through the
   classifier; author updates; final gate approves; publish creates a sibling
   work with the provenance badge.
4. Memory opt-out: with sharing off, a second author's job does not hit the
   first author's memory.
5. Failure: machine job fails mid-way → hold released, units retain
   `translated` state so a retry is incremental.

### 11.9 Acceptance tests

- quote/consent precedes any machine job (no job without a quote transaction)
- re-run reuses unchanged paragraphs (stable indices; `memory_hit` recorded)
- glossary precedence: exact > case-insensitive > longest
- memory sharing is opt-in: default is private; opted-out memory never appears
  in another account's lookups
- review notes pass the positivity classifier before storage
- published sibling works carry provenance in front matter; WorkPage renders
  the badge; the API exposes it per §21
- a failed machine job releases the hold and preserves completed units

### 11.10 Pitfalls

- **Never** write translated chapters directly into content tables — always
  through M3's publish path so revisions, versioning and positivity apply
  unchanged.
- Paragraph indices are contract: re-segmentation on re-run must not renumber
  existing units (append-only units; deletions tombstone).
- Provenance is data, not decoration: if the model/version is not in the
  front matter, the disclosure rule is violated even if the UI shows a badge.
- Machine providers are behind a trait (`TranslationProvider`) with a dev
  implementation (identity/echo) — the pipeline is testable without network.

---

## 12. Milestone 18 (repo) — Public API, bots, feeds, push, federation, AI providers

**Read first:** spec §23 in full, plus §0.3 (the public API is not a bypass:
every rule the web app applies applies here) and the M7-03 debt row (device
delivery lands here when a mail transport exists).

### 12.1 Why this is next

The public API is a **projection** of everything already built: the same
content rules, the same visibility, the same positivity gate, the same caps.
It comes after the feature surface exists so that M19's admin and abuse
tooling can see the whole external surface it must defend. AI provider access
is last here because it is the most sensitive external dependency.

### 12.2 Ledger rows

- `M18-01` public REST API: read endpoints with visibility and rate limits,
  write endpoints for account holders with CSRF-free token auth (spec §23.1)
- `M18-02` bots: registered API agents with scoped tokens, audited, owner-
  accountable, revocable (spec §23.2)
- `M18-03` feeds: RSS/Atom for works, series, tags; podcast for audio
  chapters (spec §23.3)
- `M18-04` web push: opt-in, per-device, quiet hours from the reader's
  settings (spec §23.4)
- `M18-05` federation: instance blocklist/allowlist, trust propagation as
  bounded data, content moderation on inbound content (spec §23.5)
- `M18-06` AI provider access: consented, scoped, per-work consent records,
  rate-limited, paid through M15 where priced (spec §23.6)
- `M18-07` device delivery (Kindle/email): refused-by-default until a mail
  transport is configured; when enabled, priced through the M15 quote flow
  (closes `M7-03`; spec §13.4)

### 12.3 Migration `0019_external.sql` (both dialects)

```text
api_tokens(
  id TEXT PRIMARY KEY,
  account TEXT NOT NULL,
  kind TEXT NOT NULL,             -- personal | bot
  name TEXT NOT NULL,
  token_hash TEXT NOT NULL,       -- store the hash, show the token once
  scopes TEXT NOT NULL,           -- JSON scope list, subset of the vocabulary
  created_at TEXT NOT NULL,
  last_used_at TEXT,
  revoked_at TEXT
)
index (account)

bot_registrations(
  id TEXT PRIMARY KEY,
  token_id TEXT NOT NULL,
  owner TEXT NOT NULL,            -- the accountable human
  contact TEXT NOT NULL,
  user_agent TEXT NOT NULL,
  state TEXT NOT NULL,            -- active|suspended|revoked
  registered_at TEXT NOT NULL
)

feed_handles(
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,             -- work|series|tag|user
  subject TEXT NOT NULL,
  handle TEXT NOT NULL UNIQUE,    -- the stable feed path segment
  created_at TEXT NOT NULL
)

push_subscriptions(
  id TEXT PRIMARY KEY,
  account TEXT NOT NULL,
  endpoint TEXT NOT NULL,
  keys TEXT NOT NULL,             -- p256dh + auth
  device_name TEXT,
  created_at TEXT NOT NULL,
  revoked_at TEXT
)
index (account)

federation_peers(
  id TEXT PRIMARY KEY,
  host TEXT NOT NULL,
  direction TEXT NOT NULL,        -- allow | block
  reason TEXT,
  set_by TEXT NOT NULL,
  set_at TEXT NOT NULL
)
unique (host)

federation_inbound(
  id TEXT PRIMARY KEY,
  peer_host TEXT NOT NULL,
  object_type TEXT NOT NULL,
  object_id TEXT NOT NULL,
  received_at TEXT NOT NULL,
  state TEXT NOT NULL,            -- accepted|quarantined|rejected
  note TEXT
)
index (peer_host, received_at)

ai_consents(
  id TEXT PRIMARY KEY,
  work_id TEXT NOT NULL,
  provider TEXT NOT NULL,         -- provider id from configuration
  consent TEXT NOT NULL,          -- versioned JSON: scope, expiry, revocation
  granted_by TEXT NOT NULL,
  created_at TEXT NOT NULL,
  revoked_at TEXT
)
unique (work_id, provider)

ai_requests(
  id TEXT PRIMARY KEY,
  work_id TEXT NOT NULL,
  provider TEXT NOT NULL,
  account TEXT,                   -- the requesting account, when known
  purpose TEXT NOT NULL,
  charged_transaction TEXT,       -- M15 ledger when priced
  requested_at TEXT NOT NULL,
  served_at TEXT
)
index (work_id, requested_at)
```

### 12.4 Domain modules

- `crates/domain/src/api_scopes.rs`: the scope vocabulary (`content.read`,
  `content.write`, `library.read`, `comments.write`, `translation.read`, …),
  token validation, scope subset rule (a bot's token cannot exceed its
  registration's requested scopes).
- `crates/domain/src/feeds.rs`: feed document builders (RSS 2.0 and Atom)
  from the same internal model, escaping via the sanitiser's escaping rules,
  podcast enclosures for audio chapters, stable IDs (tag URIs with the
  instance base URL from configuration).
- `crates/domain/src/push.rs`: payload builder (no content beyond what the
  reader opted into), quiet-hours evaluation from the reader's settings
  (quiet hours suppress delivery, not storage), device revocation.
- `crates/domain/src/federation.rs`: inbound pipeline stages (peer check →
  payload bound → classification → quarantine on failure), outbound rate
  limits per peer, trust propagation as **bounded data** (numbers with
  defined meaning, never strings of policy).
- `crates/domain/src/ai_gate.rs`: the consent check (`ai_consents` valid for
  work + provider + scope + expiry), the rate-limit check, the charge
  decision (paid providers route through `charging::quote_flow`), and the
  refusal that names nothing beyond "not consented".

### 12.5 Repository `crates/db/src/external.rs`

- Tokens: `issue_token` (hash, scopes), `resolve_token` (hash → scopes,
  updates `last_used_at`), `revoke_token`.
- Bots: registration CRUD, suspend/revive, the owner-accountability join for
  abuse lookups.
- Feeds: `feed_handle_for(kind, subject)`, `upsert_feed_handle`.
- Push: `register_subscription`, `subscriptions_for(account)` (the send job's
  input), `revoke_subscription`.
- Federation: `peer_policy(host)`, `record_inbound`, `quarantine`,
  `reject_object`.
- AI: `consent_for(work, provider)`, `record_request`, `serve_request`.
- Device delivery: reuses M7's export targets; the route stays **refused by
  default** with a distinct error until mail configuration exists (the same
  refusal the M7 tests pin).

### 12.6 Routes

```text
-- public read API (token or anonymous; visibility + rate limits apply)
GET /api/v1/public/works/:id
GET /api/v1/public/search?q=...            (the M10 AST, read scopes)
GET /api/v1/public/taxonomy?...
-- token management (session; CSRF applies)
GET|POST /api/v1/me/tokens  DELETE /api/v1/me/tokens/:id
POST /api/v1/me/bots                       (registration + token together)
-- feeds (public, no auth)
GET /feeds/:handle.xml  GET /feeds/:handle.atom
GET /feeds/:handle/podcast.xml             (audio chapters)
-- push
POST /api/v1/me/push/subscribe  POST /api/v1/me/push/:id/revoke
-- federation (inbound under a dedicated mount, signed)
POST /federation/inbox  GET /federation/actor/:slug
-- AI provider access (token with `ai.read` + consent + rate limit)
GET /api/v1/ai/works/:id?purpose=...       (serves only with valid consent)
-- device delivery (refused by default; closes M7-03)
POST /api/v1/exports/:id/deliver           (requires mail transport config)
```

### 12.7 Pages

- `ApiTokens.svelte`, `Bots.svelte` (settings): issue/revoke, scope picker
  with plain-language descriptions, last-used column, bot contact field.
- `Notifications.svelte`: the push opt-in flow (browser permission prompt
  only after the reader clicks opt-in), device list, quiet hours.
- WorkPage: subscribe affordances (RSS/podcast icons linking feed handles).
- Operator: federation peer console (allow/block with reason; inbound
  quarantine list with accept/reject).

### 12.8 Hand journey

1. Issue a personal token; read a public work anonymously and with the token
   (rate limits differ); a private work 404s anonymously and with a token
   lacking scope.
2. Bot: register with scopes `content.read library.read`; the bot reads;
   a write attempt with a read-scoped token is refused; owner revokes →
   the bot's next request 401s.
3. Feeds: subscribe to a work feed in a reader; publish a chapter; the feed
   updates; the podcast feed lists the audio chapter enclosure.
4. Push: opt in on one device; a followed work updates; delivery respects
   quiet hours; revoke the device.
5. Federation: block a peer → inbound quarantines; unblock → accepted; a
   poisoned payload (oversized, malformed) → rejected with the reason
   recorded.
6. AI: grant a provider consent on one work; the provider reads it; a second
   work without consent → refusal; revoke → next request refuses.
7. Device delivery: without mail config the endpoint answers the documented
   refusal; configure a dev transport → the export delivers and charges
   through the quote flow.

### 12.9 Acceptance tests

- API visibility equals web visibility (same work matrix, same positivity
  rules, same block rules) — the same fixture suite runs against both
- token auth: hash-only storage; scope enforcement per endpoint; revocation
  immediate; `last_used_at` updated
- bot accountability: every bot action resolves to an owner account
- feeds: valid XML (validate against the RSS/Atom schemas), stable IDs,
  unchanged content → unchanged feed (conditional-GET friendly)
- push: quiet hours suppress delivery but not storage; revoked device stops
  receiving; payload contains nothing beyond the opt-in
- federation: blocked peer quarantined; inbound content passes the same
  classifier; trust propagation values bounded (reject out-of-range)
- AI: no consent → refusal; expired consent → refusal; charged requests
  reference the ledger; rate limits per provider
- device delivery refused without mail config (the pinned M7-03 refusal)

### 12.10 Pitfalls

- The public API must not bypass a single rule the web app enforces. Where a
  rule lives only in a route handler, lift it into the domain first — this is
  the milestone's hidden cost, and it is worth it.
- Rate limits are per-token and per-IP, from configuration; anonymous limits
  are lower than authenticated ones and the **docs say so**.
- Feed handles are stable: a feed URL that changes breaks subscribers — the
  handle table exists so handles never encode internal ids.
- AI consent is per work and per provider with expiry; a site-wide "AI
  allowed" switch does not exist in the spec — do not invent one.
- Federation trust propagation is numbers with defined meaning only; free
  text "reputation" imports are refused at parse time.

---

## 13. Milestone 19 (repo) — Administration, statistics, abuse defence, privacy, operations

**Read first:** spec §24 in full, plus the debt register (`M17-01` — admin
beyond `doctor` — is this milestone) and §19 (the operator's role in
governance).

### 13.1 Why this is next

Every milestone so far left its operational surface implicit (jobs, config,
logs). This milestone makes the **operator a first-class user**: statistics
that answer real questions, privacy requests that complete legally-required
flows, abuse defence that acts before a small self-hosted box drowns, and
administrative tools that don't require hand-written SQL. It is last before
hardening because it needs the whole surface to exist to defend it.

### 13.2 Ledger rows

- `M19-01` admin console: role-gated (operator, preservation officer),
  every action audited through M14's `audit_log` (spec §24.1)
- `M19-02` statistics: reading, posting, engagement, discovery — derived,
  privacy-preserving, honest about gaps (spec §24.2)
- `M19-03` abuse defence: per-IP and per-account circuit breakers, signup
  controls, challenge gates, emergency rate config without redeploy (spec
  §24.3)
- `M19-04` privacy: data export (the reader's own, complete), account deletion
  with the documented cascade, removal of derivatives, the provider data map
  (spec §24.4)
- `M19-05` operations: backup/restore drills, migrations run and verified,
  the `doctor` command grown into the ops surface, log hygiene (spec §24.5)

### 13.3 Migration `0020_admin.sql` (both dialects)

```text
admin_actions(
  id TEXT PRIMARY KEY,
  actor TEXT NOT NULL,             -- operator account
  action TEXT NOT NULL,            -- the admin verb
  subject_type TEXT NOT NULL,
  subject_id TEXT NOT NULL,
  document TEXT NOT NULL,          -- parameters + result summary
  created_at TEXT NOT NULL
)
index (actor, created_at)

feature_flags(
  key TEXT PRIMARY KEY,
  state TEXT NOT NULL,             -- off | on | rollout
  rollout_bp INTEGER NOT NULL DEFAULT 0,   -- for gradual rollout
  note TEXT NOT NULL,
  updated_by TEXT NOT NULL,
  updated_at TEXT NOT NULL
)

announcements(
  id TEXT PRIMARY KEY,
  body TEXT NOT NULL,              -- sanitised document
  level TEXT NOT NULL,             -- info | warning | maintenance
  starts_at TEXT NOT NULL,
  ends_at TEXT,
  created_by TEXT NOT NULL
)

stat_snapshots(
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,              -- reading|posting|engagement|discovery
  period TEXT NOT NULL,            -- the bucket (day/week)
  document TEXT NOT NULL,          -- aggregated, k-anonymised numbers
  computed_at TEXT NOT NULL
)
unique (kind, period)

privacy_requests(
  id TEXT PRIMARY KEY,
  account TEXT NOT NULL,
  kind TEXT NOT NULL,              -- export | delete | derivative_removal
  state TEXT NOT NULL,             -- pending|processing|done|failed
  requested_at TEXT NOT NULL,
  completed_at TEXT,
  result_ref TEXT                  -- storage key of the export archive
)

abuse_counters(
  key TEXT NOT NULL,               -- 'ip:1.2.3.4' | 'account:x' | 'global:x'
  window TEXT NOT NULL,            -- the bucket key
  count INTEGER NOT NULL DEFAULT 0,
  blocked_until TEXT
)
primary key (key, window)

ip_policy(
  ip TEXT PRIMARY KEY,
  state TEXT NOT NULL,             -- allow | challenge | block
  reason TEXT,
  set_by TEXT NOT NULL,
  set_at TEXT NOT NULL
)

signup_controls(
  id TEXT PRIMARY KEY CHECK (id = 'singleton'),
  mode TEXT NOT NULL,              -- open | invite | closed
  challenge INTEGER NOT NULL DEFAULT 0,   -- require a challenge gate
  updated_by TEXT NOT NULL,
  updated_at TEXT NOT NULL
)
```

### 13.4 Domain modules

- `crates/domain/src/stats.rs`: the aggregation definitions (what counts as a
  read, how buckets roll up), k-anonymity floor for any per-entity stat
  (numbers below the floor aggregate into "other"), and the honesty rule —
  a statistic that could not be computed reports its gap, never a zero.
- `crates/domain/src/abuse.rs`: circuit-breaker policy (windows, thresholds,
  blocked_until transitions from configuration), the challenge decision, and
  the emergency-ladder semantics (raising limits is a config change, not a
  redeploy; the operator console writes config overrides with audit rows).
- `crates/domain/src/privacy.rs`: the export manifest (every table that
  carries the account's data, with the column list), the deletion cascade
  order (what is anonymised, what is hard-deleted, what is retained for
  legal/audit with the retention window), and the derivative-removal rule
  (translation publications, gallery derivatives, AI served responses).

### 13.5 Repository `crates/db/src/admin.rs`

- `record_admin_action` (in-transaction with every admin mutation), admin
  queries for listings/lookups the console needs, `feature_flags` CRUD,
  announcements CRUD, `stat_snapshots` upsert (a recompute job per kind).
- Privacy: `open_privacy_request`, `complete_privacy_request`; the export job
  walks the manifest and writes the archive to M5 storage; the delete job
  runs the documented cascade in **one transaction per table group**, with
  the retention-window table list derived from `crates/domain/src/privacy.rs`.
- Abuse: `bump_abuse_counter(key, window) -> (count, blocked_until)`,
  `blocked(key)`, `ip_policy` CRUD, `signup_controls` read/write.
- `doctor` grows subcommands: `doctor stats`, `doctor privacy-audit`,
  `doctor backup-check`, `doctor abuse-config` — each prints what it
  verified and exits non-zero on failure.

### 13.6 Routes

```text
GET  /api/v1/admin/stats?kind=...&period=...       (operator)
GET|POST /api/v1/admin/feature-flags               (operator; audited)
GET|POST /api/v1/admin/announcements               (operator; audited)
GET  /api/v1/admin/abuse            (counters, blocked keys, ip policy)
POST /api/v1/admin/abuse/ip-policy  POST /api/v1/admin/abuse/overrides
POST /api/v1/admin/signup-controls
GET|POST /api/v1/admin/privacy-requests  POST /api/v1/admin/privacy-requests/:id/run
GET  /api/v1/me/privacy/export  POST /api/v1/me/privacy/delete
GET  /api/v1/admin/audit-log?q=...                  (read-only)
```

### 13.7 Pages

- `Admin/Stats.svelte` — snapshots by kind/period with honesty gaps shown;
  no per-user drill-down below the k-anonymity floor.
- `Admin/Console.svelte` — flags, announcements, signup controls; each
  mutation shows the audit-log entry it produced.
- `Admin/Abuse.svelte` — live counters, blocked keys with expiry, IP policy
  editor, the emergency ladder (config overrides) with before/after diff.
- `Admin/Privacy.svelte` — the request queue; run/complete with result
  links; the data map (every table, every retention window) rendered from
  `crates/domain/src/privacy.rs`'s manifest so docs and code can't diverge.
- Reader-facing: `Privacy.svelte` (export download, delete with cascade
  preview and confirmation typing), notifications for completion.

### 13.8 Hand journey

1. Operator flips a feature flag with a rollout percentage; a reader session
   below the threshold sees the flag off; above, on; the audit row exists.
2. Statistics: generate activity across accounts; snapshots compute; numbers
   below the k floor aggregate into "other"; a gap reports as a gap.
3. Abuse: hammer an endpoint from one IP → the breaker opens (`blocked_until`),
   the response is the documented 429-with-retry-after; the operator raises
   the threshold from the console (config override, no redeploy) and the
   breaker honours it on the next window.
4. Privacy export: request → job → archive in storage → download link (the
   archive contents match the manifest, table by table).
5. Delete: request with typed confirmation → cascade runs → the account's
   works are anonymised (attribution removed), sessions revoked, audit rows
   retained within their window; a second delete request 404s.
6. Signup: switch to invite mode → registration requires an invite; the
   challenge gate trips for flagged IPs.

### 13.9 Acceptance tests

- every admin mutation writes `admin_actions` **and** `audit_log` (count both)
- feature flag rollout is stable for a given account (hash-based, not random
  per request)
- stats: k-anonymity floor holds; gaps report as gaps; snapshots are
  idempotent per kind+period
- circuit breaker opens on threshold, honours `blocked_until`, and the
  config override applies without redeploy
- privacy export archive contents match the manifest exactly (table names and
  column lists compared)
- delete cascade: attribution removed, derivatives removed or anonymised per
  the manifest, audit retained for its window; a deleted account's requests
  404 afterwards
- signup controls: invite mode refuses open registration; challenge gate
  trips for policy-flagged IPs
- `doctor stats|privacy-audit|backup-check` exit non-zero on failure (test
  the failure path by breaking a fixture)

### 13.10 Pitfalls

- The privacy export must include **everything** the manifest lists; a table
  added later must update the manifest in the same commit (add a test that
  fails when a new account-carrying table is missing from it — search the
  migrations for the account column and diff against the manifest).
- Deletion cascades run in the documented order; a cascade that orphans a
  translation publication or gallery derivative violates §24.4 — the
  derivative-removal rule is a test, not a comment.
- Statistics are not surveillance: no per-user drill-down below the k floor,
  and the operator UI says so. If a chart needs individual rows, it is the
  wrong chart.
- Circuit breakers must fail **closed** for writes and **open** for reads
  sensibly: a breaker that locks readers out of the site entirely is worse
  than the abuse it stops — the ladder's first rungs protect the box, the
  last rungs protect the community.

---

## 14. Milestone 20 (repo) — Hardening and release

**Read first:** spec §25 in full, plus `docs/verification.md` end to end —
this milestone's definition of done is **the spec's**, not a reduced one.

### 14.1 Why this is next

Everything is built; nothing is trusted until it is exercised as a whole.
M20 is not a feature milestone: it is the pass that makes the site releasable
per the spec's definition of release, with the verification matrix full and
the honest status vocabulary intact.

### 14.2 Ledger rows

- `M20-01` cross-milestone regression sweep: the full verification matrix
  executed, gaps documented (spec §25.1)
- `M20-02` performance and bounds: the documented bounds hold at 2× the
  defaults on the reference self-hosted box (spec §25.2)
- `M20-03` security pass: threat model walkthrough, permissions matrix
  verified, secrets hygiene, dependency audit (spec §25.3)
- `M20-04` accessibility and i18n: a11y audit per §6, every user-facing
  string in both `en` and `eo` (spec §25.4)
- `M20-05` release: docs, tutorial, tag, migration story for operators
  upgrading from `v0.09-library` (spec §25.5)

### 14.3 Work breakdown (no new tables; the migration counter stops at `0020`)

**14.3.1 Verification sweep (M20-01).** Walk `docs/requirements.csv` row by
row; every row is either evidenced, re-tested now, or marked with an honest
gap. Any row citing "implemented but not executed" gets executed or the gap
becomes a release blocker discussion. Re-run every `milestone_*.rs` suite; a
flaky test is a bug — fix it, don't retry it.

**14.3.2 Performance and bounds (M20-02).** Script the bounds from the spec:
a 2× dataset (double the defaults), the reference self-hosted profile, and
the documented commands (import a large work, run search, render the reader
with 1,000-paragraph chapters, run discovery with 2× candidates, queue 2×
jobs). Record numbers in `docs/verification.md`. A bound that fails gets
either fixed or becomes a documented limitation with the failing number.

**14.3.3 Security pass (M20-03).** Walk the permissions matrix (spec §5.4)
end to end: every route × every role (anonymous, reader, author, reviewer,
operator) → expected vs actual. Then: secrets hygiene (no secret in logs, in
errors, in client payloads — grep the codebase), dependency audit
(`cargo audit`, `npm audit`), the extension worker profile verified on the
reference box, webhook HMAC verification from the outside, CSRF coverage on
every cookie-authenticated `Write` route (list them all, tick them all).

**14.3.4 Accessibility and i18n (M20-04).** Keyboard-walk every page
(WorkPage, Reader, forms, admin console); focus order and labels verified;
`labels.ts` audited: a `grep` for raw strings in templates must return only
sanctioned literals (numbers, punctuation). Both `en` and `eo` complete for
every new string since M9.

**14.3.5 Release (M20-05).** Update `docs/tutorial/README.md` (the chapter
map to the M-numbering used here), regenerate any spec cross-references,
write the operator upgrade note (migrations `0010`–`0020` with the dialect
notes), tag `v0.20-release` (per docs/tutorial/README.md's scheme), and only
then run the full `just check` one final time.

### 14.4 Hand journey (the release rehearsal)

Run the **whole site** as one story on the reference box, in one sitting:

1. Fresh install → `doctor` → sign up (reader) → import a fic (author) →
   publish (editor, revision, chapter reorder via the UI now that M12 opened
   it) → read with a second account → rate/react → comment through the
   positivity gate → follow → library updates → discovery feed → search with
   the query language → save a view.
2. Translation: request, review, publish, provenance badge.
3. Economy: earn, quote, reserve, priority import, capture, receipts.
4. Community: forum thread, group, message, block across paths, presence.
5. Governance: report, quorum, sanction, appeal, expiry.
6. Extensions: install with consent, run, overrun, revoke; webhook verify.
7. Admin: flag rollout, stats, abuse breaker + override, privacy export and
   delete.
8. API: token, bot, feeds, push, federation blocklist, AI consent refusal.

Each step: recorded in `docs/verification.md` as it happens, with the
command or click path. A step that cannot be completed is a release blocker
or a documented limitation — decided explicitly, never silently.

### 14.5 Acceptance tests (release gate)

- the full matrix: every CSV row evidenced or an open, documented gap
- `just check` green on the reference box, twice in a row (flakiness check)
- both dialects: the migration sets identical (the drift test), and a
  **fresh install on PostgreSQL** from `v0.09-library` → head succeeds
- bounds: the 2× script's numbers recorded; every bound either holds or has
  a documented limitation with the number that failed
- no raw user-facing strings in templates outside `labels.ts` (the grep is
  part of CI now)
- secrets hygiene greps clean; dependency audit clean or exceptions
  documented with reasons

### 14.6 Pitfalls

- Do not "fix" a flaky test by retrying or by sleeping; find the race or the
  clock dependency. The release gate runs the suite **twice**.
- The bounds script is part of the repo (`docs/scripts/bounds.sh` or similar)
  so the next person can re-run it — a number without a reproduction is a
  rumour.
- Release ordering matters: docs and tutorial **before** the tag, so the tag
  points at a commit where the docs are already true.
- Do not silently re-scope M20's rows. If the sweep finds unverifiable rows,
  they stay visible in `docs/verification.md` with their gap statement —
  honesty is the release gate, not a nice-to-have.

---

## 15. Milestone 21 (repo) — Spec-revision skeleton: monetization, subscriptions, alerts, gifts

**Read first:** spec §20.9 (work monetization), §23.3 (subscriptions),
§14.2 (saved-search alerts), §18.10 (gifts), §24.14 (AI-crawler posture),
§6.7/§9.2/§15.4–15.5/§16.2–16.9 (the customization-first changes), and the
ADRs `docs/adr/0017` and `0018`. The skeleton (migration 0022, the contract
routes, the domain rule modules, and `milestone_21.rs`) already exists and
passes; this milestone is **filling the bodies** without reshaping them.

### 15.1 What is already in place (the skeleton, this commit)

- Migration `0022_spec_revision.sql` in **both** dialects: `work_pricing`,
  `work_entitlements`, `author_earnings_ledger`, `payouts`,
  `monetization_assertions`, `work_gifts`, `content_subscriptions`,
  `search_alerts`, and `works.ai_training`.
- Domain rule modules `crates/domain/src/monetization.rs` and
  `subscriptions.rs`: the §20.9.3 invariants (separate ledgers, no
  credit→money conversion, early-access as scheduled unlock, no ranking
  boost, self-dealing refusal, 85/15 split; subscriber lists never visible,
  alert frequency bounds) as pure, tested functions.
- Contract routes in `crates/app/src/routes/monetization.rs` and
  `subscriptions.rs`, mounted in `server.rs`, returning `501
  NOT_IMPLEMENTED` with pinned request shapes.
- `crates/app/tests/milestone_21.rs`: 14 tests pinning the migration, the
  domain invariants, and the route contracts (401 anonymous / 501 authed).

### 15.2 Ledger rows

- `M21-01` monetization eligibility + assertions: instance setting,
  imported-works rule, re-assertion on price change (spec §20.9.1)
- `M21-02` pricing + purchase + entitlements: durable, server-side checks,
  survive pseud switching (spec §20.9.2–20.9.3)
- `M21-03` tips: credit tips via the credit ledger, money tips via the
  earnings ledger, never mixed (spec §20.9.2)
- `M21-04` payouts + platform split: 85/15 default, processor flow
  (spec §20.9.3)
- `M21-05` gifts and dedications: recipient listing, decline, block
  neutrality, challenge-fulfillment identity (spec §18.10)
- `M21-06` content subscriptions: five subject kinds, pause, notify on
  eligible publication, no subscriber list (spec §23.3)
- `M21-07` saved-search alerts: scheduled runs at reader permissions,
  frequency bounds, pausable (spec §14.2)
- `M21-08` `ai_training` assertion: works column surfaced in metadata and
  exports; generated robots.txt with operator-configurable defaults
  (spec §24.14, ADR 0018)

### 15.3 Work breakdown

**16.3.1 Monetization (M21-01..04).** Implement the db repository in
`crates/db/src/monetization.rs` (dual-dialect per the economy.rs pattern),
then fill the route bodies. Entitlement checks join the work-read path in
`can_access_content`'s neighbourhood — one function, not per-handler checks.
The earnings ledger is append-only like the credit ledger; a correction is an
opposing entry.

**16.3.2 Gifts (M21-05).** `work_gifts` inserts fail with a validation error
that does not reveal blocks (§18.10). A challenge fulfillment with a named
recipient writes the same row a gift writes.

**16.3.3 Subscriptions and alerts (M21-06..07).** A publication event
(enqueue through the existing outbox, M3's machinery) fans out to content
subscriptions; notifications respect digest and quiet-period settings.
Alerts run the saved query with the reader's own permissions at run time —
reuse the search executor, never a second implementation.

**16.3.4 AI-crawler posture (M21-08).** A robots generator route with the
disallow list from instance settings; the `ai_training` value joins work
metadata and the §13.1 export.

**16.3.5 Customization-first follow-ups.** These are spec text only in this
pass — the next feature milestone carries the code: per-surface recipes,
recipe diff, ephemeral "for now" filters (§16.7, §15.5), the
appearance-bundle import/export (§6.7), and the reader-influence dial
replacing the opt-out switch (§16.5).

### 15.4 Acceptance tests

- `milestone_21.rs` extended: a purchased work is readable by the buyer and
  paywalled (honest state) for others; an early-access chapter opens at
  `public_at`; a credit tip and a money tip land in different ledgers; a
  gift is declined account-wide; a subscriber count never returns a list; a
  daily alert does not re-run inside its window; `robots.txt` disallows
  AI crawlers by default.
- All new tests dual-run on SQLite (and PostgreSQL where the CI matrix has
  it), same as every milestone.

### 15.5 Pitfalls

- **Do not change a contract shape to make an implementation easier.** The
  501 tests pin the API; if a shape must change, change the spec, the route,
  and the test in one commit.
- **Never mix ledgers.** If a code path moves credits and money in one
  transaction, it is wrong by construction.
- **The entitlement check is server-side and central.** One function on the
  read path; a per-handler `if purchased` is the bug that ships.
- **Robots defaults are configuration, not folklore.** The default list
  lives in the operator docs and is asserted by a test.

### 16.4 Instance Topics (M30, §0.4) — landed 2026-09-19

Config shape (`[site] topics = [...]`), gamification effects (public-topic
completion bonus + per-public-topic leaderboard category), anti-gaming (never
applies to imported/own work, same time-on-page gate as §9.7.3), visibility
(`/api/v1/meta` reports public topic names only). Operator who declares
nothing behaves identically to a topic-agnostic instance — kink-focused
instances are the default.

---


---

## 15a. Forum-first milestones (repo M31–M35, spec §35)

**Read first:** spec §35 in full, plus §17 (the surface being revised),
§12.8 (forum positivity policy), §19 (trust ladder, modlog, appeals),
§23.6–23.10 (federation), §29.6 (ballot privacy), §0.3 (what votes and
karma may never buy). Every brief below follows the §1 workflow loop and
the §2 house rules; only the parts that differ are written out.

### 15a.0 Numbering map additions

| Repo | Spec | Topic | Depends on |
|---|---|---|---|
| M31 | §35.1 | Work-linked threads, reaction bar, discussion modes, comment migration tool | M12 |
|| M32 | §35.2 | Typed votes, budgets, meta-moderation, karma | M12, M14 |
|| M33 | §35.3 | Thread modes: AMA, reading group, critique circle, wiki pin, collab fic, prompt, character voice | M31, M32 |
|| M34 | §35.4 | Spoilers, content warnings, reading time, draft autosave, scheduling, long-post fold | M12 |
|| M35 | §35.5 | Discovery, health, UX, federation scope | M31–M34, M18 |

Order matters: M31 before M33 (thread modes link to works), M32 before M33
(votes are the signal inside prompt/AMA modes), M34 anywhere after M12,
M35 last (it builds on all of them plus M18's federation base).

### 15a.1 Milestone 31 (repo) — Work-linked threads and the reaction bar

**Ledger rows (add before code):**

```
M31-01,community,Work discussion mode per work and instance default (ThreadOnly/CommentsOnly/Both),M31,<status>,<evidence>,spec §35.0
M31-02,community,Typed-vote reaction bar on work page with one vote per pseud,M31,<status>,<evidence>,spec §35.1
M31-03,community,Work-linked forum topics with backlink cards and Discuss button,M31,<status>,<evidence>,spec §35.1
M31-04,community,Chapter publish auto-creates or prompts linked topic exactly once,M31,<status>,<evidence>,spec §35.1
M31-05,community,Batch comment-to-topic migration preserving authorship and timestamps,M31,<status>,<evidence>,spec §35.1
M31-06,community,ThreadOnly work page renders no comment form,M31,<status>,<evidence>,spec §35.0
```

**Migration `0038_work_discussion.sql` (both dialects):**

- `ALTER TABLE works ADD COLUMN discussion_mode TEXT NOT NULL DEFAULT 'comments_only'`
  — existing works keep today's behavior; the instance default is config
  (`[forum] work_discussion_default = "thread_only"`), applied to **new**
  works at creation, never retroactively.
- `topic_work_links (id, topic_id UNIQUE, work_id, chapter_id NULL, created_at)`,
  index on `work_id`; FK to `forum_topics`/`works`.
- `work_reactions (work_id, pseud, vote_type, created_at, updated_at,
  PRIMARY KEY (work_id, pseud))`.

**Domain (`crates/domain/src/forum.rs` grows, new `work_discussion.rs`):**
mode resolution (work override → instance default), reaction vote rules
(allowed types on this surface = positive set + `disagree`, one-per-pseud,
change/retract), linked-topic uniqueness per work/chapter, migration
round-trip rules.

**Repository (`crates/db/src/community.rs` grows):** reaction upsert with
count aggregation, `topic_work_links` CRUD, comment→post conversion in one
transaction (comments copied in order, `comments.deleted_at` set, tombstone
marker stored).

**Routes (`crates/app/src/routes/` — extend `community.rs`, new
`work_discussion.rs`):** `GET/PUT /works/{id}/discussion-mode`,
`GET /works/{id}/reactions`, `POST/DELETE /works/{id}/reactions/{type}` or
`POST /works/{id}/reactions` with body, `GET /works/{id}/thread`,
`POST /works/{id}/migrate-comments`. All `classified(...)`; author-only
gates on mode change and migration.

**Frontend:** `WorkPage.svelte` reaction bar (typed buttons + counts +
Discuss link, no comment form in ThreadOnly), discussion-mode picker in the
work editor, backlink card on the topic page. State quartet on every new
surface; labels `en` + `eo`.

**Acceptance tests (`crates/app/tests/milestone_31.rs`):** mode resolution
(preference order), reaction one-per-pseud under concurrency, linked topic
created exactly once per chapter publish, migration round-trip preserves
author + timestamp + order, ThreadOnly page has no comment form (assert the
route refuses comment creation in ThreadOnly mode), CommentsOnly work
behaves as before.

**Pitfalls:** do not apply the instance default retroactively; do not let
the migration tool run twice concurrently (idempotency key on work); the
reaction bar must fail closed (hidden) when the vote type set is
unconfigured.

### 15a.2 Milestone 32 (repo) — Typed votes, budgets, meta-moderation, karma
Spec §35.2. Depends on: M31 (work reactions), M14 (trust levels).
**Full detail: `docs/plans/m32-m35-detailed.md` §15a.2.**

**Migration `0039_forum_votes.sql` (SQLite dialect shown; PostgreSQL mirrors):**
```sql
CREATE TABLE forum_vote_types (
    id TEXT PRIMARY KEY,
    label TEXT NOT NULL UNIQUE,
    category_scope TEXT,            -- NULL = instance-wide default
    weight REAL NOT NULL DEFAULT 1.0,
    cost INTEGER NOT NULL DEFAULT 1,
    is_negative INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX forum_vote_types_category ON forum_vote_types (category_scope);

CREATE TABLE forum_votes (
    post_id TEXT NOT NULL,
    pseud TEXT NOT NULL,
    vote_type TEXT NOT NULL,
    weight_at_cast REAL NOT NULL DEFAULT 1.0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (post_id, pseud)
);
CREATE INDEX forum_votes_type ON forum_votes (post_id, vote_type);

CREATE TABLE forum_meta_votes (
    vote_post_id TEXT NOT NULL,
    vote_pseud TEXT NOT NULL,
    pseud TEXT NOT NULL,
    fair INTEGER NOT NULL,          -- 1 fair, 0 unfair
    created_at TEXT NOT NULL,
    PRIMARY KEY (vote_post_id, vote_pseud, pseud),
    FOREIGN KEY (vote_post_id, vote_pseud) REFERENCES forum_votes(post_id, pseud)
);

CREATE TABLE forum_karma (
    pseud TEXT PRIMARY KEY,
    karma REAL NOT NULL DEFAULT 0.0,
    updated_at TEXT NOT NULL
);
```
Seed default taxonomy: `insightful, funny, interesting, well-written, disagree`
(the last with `cost=3, is_negative=1`).

**Config `[forum]`:** `vote_budget_base INTEGER DEFAULT 10`,
`vote_budget_tl_multiplier INTEGER DEFAULT 5` (TL1=10, TL3=30, TL5=60),
`meta_mod_tl_required INTEGER DEFAULT 4`, `karma_decay_monthly REAL DEFAULT 0.05`,
`karma_inactivity_threshold_days INTEGER DEFAULT 30`.

**API response shapes:**
- `POST /forum/posts/{id}/vote` body `{"vote_type":"insightful"}` → `{"outcome":"cast"}` (201)
  Budget exhausted → 429 `BUDGET_EXHAUSTED`. Invalid type → 422.
- `DELETE /forum/posts/{id}/vote` → `{"outcome":"retracted"}` (024).
- `GET /forum/posts/{id}/votes` → `{"counts":{"insightful":5},"mine":"insightful"}` (200).
- `POST /forum/votes/{id}/meta` body `{"fair":true}` → `{"recorded":true}` (201).
- `GET /me/vote-budget` → `{"remaining":7,"reset_at":"...","trust_level":3}` (200).
- `GET /forum/karma/{pseud}` → `{"pseud":"...","karma":123.4}` (200).

**Acceptance tests (`milestone_32.rs`):**
- `budget_rejects_exhausted_voter`: POST 11 votes, 11th gets 429.
- `meta_mod_decay_reduces_weight`: cast 3 unfair meta-votes, check `weight_at_cast` drops.
- `transparency_hides_individual_votes`: anonymous user sees counts, not voter list.
- `karma_decays_on_inactivity`: UPDATE `forum_karma.updated_at` to 31 days ago, run decay, verify `karma *= 0.95`.
- `category_taxonomy_override`: create Critique category with custom types, verify different vote options.
- `negative_vote_costs_more`: disagree costs 3 budget (not 1).
- `karma_never_in_trust_code`: `grep -r forum_karma crates/ --include='*.rs'` only in its module.

**Frontend:** `ForumVoteBar.svelte` (per-post), `VoteBudget.svelte` (badge), `KarmaBadge.svelte` (profile).

### 15a.3 Milestone 33 (repo) — Thread modes
Spec §35.3. Depends on: M31 (linked topics), M32 (votes inside prompt/AMA).
**Full detail: `docs/plans/m32-m35-detailed.md` §15a.3.**

**Migration `0040_thread_modes.sql` (SQLite shown, PostgreSQL mirrors):**
```sql
ALTER TABLE forum_topics ADD COLUMN mode TEXT NOT NULL DEFAULT 'plain';
-- plain | ama | reading_group | critique | wiki_pin | collab_fic | prompt | character_voice

CREATE TABLE topic_schedules (
    id TEXT PRIMARY KEY,
    topic_id TEXT NOT NULL,
    position INTEGER NOT NULL,
    title TEXT NOT NULL,
    unlocks_at TEXT NOT NULL,
    chapter_start INTEGER,
    chapter_end INTEGER
);
CREATE INDEX topic_schedules_topic ON topic_schedules (topic_id);

CREATE TABLE topic_wiki_pins (
    id TEXT PRIMARY KEY,
    topic_id TEXT NOT NULL UNIQUE,
    post_id TEXT,
    revision INTEGER NOT NULL DEFAULT 0,
    approved_by TEXT,
    approved_at TEXT
);

CREATE TABLE critique_queue (
    id TEXT PRIMARY KEY,
    topic_id TEXT NOT NULL,
    pseud TEXT NOT NULL,
    position INTEGER NOT NULL,
    posted_at TEXT,
    UNIQUE (topic_id, pseud)
);
CREATE INDEX critique_queue_topic ON critique_queue (topic_id);

CREATE TABLE prompt_posts (
    id TEXT PRIMARY KEY,
    topic_id TEXT NOT NULL UNIQUE,
    prompt_date TEXT NOT NULL,
    winner_pseud TEXT
);

ALTER TABLE forum_posts ADD COLUMN character_id TEXT;
```

**API response shapes:**
- `PUT /topics/{id}/mode` body `{"mode":"ama"}` → `{"mode":"ama"}` (200).
- `POST /topics/{id}/sections` body `{"title":"Week 1","unlocks_at":"...","chapter_start":1,"chapter_end":5}` → `{"id":"..."}` (201).
- `GET /topics/{id}/sections` → `{"items":[{"title":"Week 1","unlocks_at":"...","unlocked":true}]}` (200; only unlocked sections visible).
- `POST /topics/{id}/wiki` body `{"body":"..."}` → `{"status":"pending"}` (201).
- `POST /topics/{id}/wiki/approve` → `{"status":"approved"}` (201).
- `POST /topics/{id}/critique/join` → `{"position":3}` (201).
- `POST /topics/{id}/critique/submit` → `{"accepted":true}` (201). Wrong turn → 422.
- `POST /topics/{id}/compile` → `{"text":"..."}` (200; stitched posts).
- `POST /topics/{id}/promote` → `{"work_id":"..."}` (201; creates work, thread read-only).

**Acceptance tests (`milestone_33.rs`):**
- `reading_group_hides_future_sections`: GET sections before unlock → 404.
- `critique_enforces_turn_order`: out-of-order submit → 422.
- `wiki_pin_invisible_until_approved`: GET pin shows pending state.
- `compile_stitches_posts_in_order`: verify output text order.
- `promote_to_work_creates_readable_work`: work exists with chapters.
- `ama_sorts_questions_to_top`: check reply ordering.
- `character_voice_post_resolves_to_user_for_mod`: block/mute still works.

**Frontend:** `ThreadModePicker.svelte`, `ReadingGroupProgress.svelte`, `WikiPin.svelte`, `CritiqueQueue.svelte`, `CollabCompileButton.svelte`, `CharacterVoiceBadge.svelte`.

### 15a.4 Milestone 34 (repo) — Spoilers, warnings, readability
Spec §35.4. Depends on: M12 (post drafts, scheduled posts — verify tables exist first).
**Full detail: `docs/plans/m32-m35-detailed.md` §15a.4.**

**Migration `0041_spoilers_warnings.sql` (SQLite shown, PostgreSQL mirrors):**
```sql
ALTER TABLE forum_topics ADD COLUMN spoiler_scope_chapter INTEGER;
ALTER TABLE forum_posts ADD COLUMN fold_at_word_count INTEGER;

CREATE TABLE content_warnings (
    id TEXT PRIMARY KEY,
    label TEXT NOT NULL UNIQUE,
    severity INTEGER NOT NULL DEFAULT 0  -- 0=info, 1=warn, 2=severe
);

CREATE TABLE post_content_warnings (
    post_id TEXT NOT NULL,
    warning_id TEXT NOT NULL,
    custom_text TEXT,
    PRIMARY KEY (post_id, warning_id)
);

-- Verify these exist from M12 before adding:
-- ALTER TABLE forum_posts ADD COLUMN scheduled_at TEXT;
-- ALTER TABLE forum_posts ADD COLUMN published_at TEXT;
-- CREATE INDEX forum_posts_scheduled ON forum_posts (scheduled_at) WHERE scheduled_at IS NOT NULL;
-- post_drafts table should exist from M12 §4.6.
```

**Config `[forum]`:** `fold_default_word_count INTEGER DEFAULT 800`.

**API response shapes:**
- `GET /topics/{id}` → includes `spoiler_scope_chapter: 12` (or null).
- `POST /posts/{id}/warnings` body `{"warning_id":"violence","custom_text":"..."}` → `{"added":true}` (201).
- `GET /posts/{id}` → includes `warnings: [{"id":"violation","severity":1}]`, `fold_at: 800`.
- `POST /posts/{id}/draft` body `{"body":"..."}` → `{"saved_at":"..."}` (201; UPSERT).
- `GET /posts/{id}/draft` → `{"body":"..."}` (200; only own draft).
- `POST /posts/{id}/schedule` body `{"scheduled_at":"..."}` → `{"scheduled":true}` (201).

**Acceptance tests (`milestone_34.rs`):**
- `spoiler_block_hidden_from_screen_reader`: verify `aria-expanded=false` + content in `aria-hidden`.
- `content_warning_blur_follows_viewer_prefs`: different users see different states.
- `draft_survives_hard_reload`: save → reload → resume.
- `scheduled_post_publishes_once_at_its_time`: job re-run does not double-post.
- `fold_never_hides_first_screenful`: measure rendered height > viewport.

**Frontend:** `SpoilerBlock.svelte`, `ContentWarningBar.svelte`, `PostComposer.svelte` (autosave), `ReadingTimeBadge.svelte`.

### 15a.5 Milestone 35 (repo) — Discovery, health, UX, federation
Spec §35.5. Depends on: M31–M34, M18 (federation base), M24 (moderation).
**Full detail: `docs/plans/m32-m35-detailed.md` §15a.5.**

**Migration `0042_discovery.sql` (SQLite shown, PostgreSQL mirrors):**
```sql
ALTER TABLE forum_topics ADD COLUMN summary_text TEXT;
ALTER TABLE forum_topics ADD COLUMN summary_revision INTEGER DEFAULT 0;
ALTER TABLE forum_topics ADD COLUMN federation_scope TEXT NOT NULL DEFAULT 'public';
ALTER TABLE forum_topics ADD COLUMN slow_mode_seconds INTEGER DEFAULT 0;
ALTER TABLE forum_topics ADD COLUMN activity_history TEXT NOT NULL DEFAULT '[]';

ALTER TABLE forum_posts ADD COLUMN original_topic_id TEXT;
ALTER TABLE forum_posts ADD COLUMN featured INTEGER NOT NULL DEFAULT 0;
ALTER TABLE forum_posts ADD COLUMN featured_by TEXT;
ALTER TABLE forum_posts ADD COLUMN featured_at TEXT;
ALTER TABLE forum_posts ADD COLUMN moderation_action TEXT;
ALTER TABLE forum_posts ADD COLUMN moderation_expires_at TEXT;
ALTER TABLE forum_posts ADD COLUMN moderation_reason TEXT;
```

**Migration `0043_federation.sql`:**
```sql
CREATE TABLE remote_profiles (
    actor_id TEXT PRIMARY KEY,
    instance_host TEXT NOT NULL,
    display_name TEXT,
    bio TEXT,
    avatar_url TEXT,
    cached_at TEXT NOT NULL,
    ttl_seconds INTEGER NOT NULL DEFAULT 3600
);

CREATE TABLE instance_reputation (
    instance_host TEXT PRIMARY KEY,
    spam_rate REAL NOT NULL DEFAULT 0.0,
    report_rate REAL NOT NULL DEFAULT 0.0,
    avg_takedown_seconds REAL NOT NULL DEFAULT 0.0,
    auto_defederated INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
);

CREATE TABLE forum_poll_votes (
    poll_id TEXT NOT NULL,
    actor_id TEXT NOT NULL,
    option_index INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (poll_id, actor_id)
);
```

**Config `[forum]`:** `summary_min_replies INTEGER DEFAULT 20`,
`summary_regenerate_every INTEGER DEFAULT 10`,
`slow_mode_default_seconds INTEGER DEFAULT 0`.

**API response shapes:**
- `GET /topics/{id}/summary` → `{"text":"...","revision":3}` (200; null if no summary).
- `PUT /topics/{id}/summary` body `{"text":"..."}` → `{"revision":4}` (201).
- `PUT /topics/{id}/federation-scope` body `{"scope":"local"}` → `{"scope":"local"}` (201).
- `POST /topics/{id}/fork` body `{"title":"...","post_ids":[...]}` → `{"topic_id":"..."}` (201).
- `POST /posts/{id}/feature` → `{"featured":true}` (201).
- `PUT /topics/{id}/slow-mode` body `{"seconds":300}` → `{"slow_mode_seconds":300}` (201).
- `GET /forum/digest` → `{"items":[{"post_id":"...","title":"..."}]}` (200).
- `GET /forum/health` → `{"daily_active_posters":42,"new_user_retention_7d":0.3,...}` (200; admin only).
- `GET /forum/search?q=...&semantic=true` → `{"items":[...],"method":"pgvector|fts"}` (200).
- `GET /forum/remote-profiles/{id}` → `{"actor_id":"...","display_name":"...","instance_host":"..."}` (200).

**Acceptance tests (`milestone_35.rs`):**
- `local_topic_absent_from_federation_outbound`: assert on delivery builder, not UI.
- `forking_preserves_audit_trail`: original_topic_id set, tombstone present.
- `health_dashboard_respects_privacy_ceilings`: no individual user data.
- `slow_mode_enforces_rate_limit`: 429 SLOW_MODE.
- `federated_poll_counts_one_actor_once`: duplicate actor_id rejected.
- `keyboard_shortcuts_documented_and_focus_safe`: ? modal lists all shortcuts.

**Frontend:** `ThreadSummary.svelte`, `ForkButton.svelte`, `FeatureBadge.svelte`, `DigestView.svelte`, `SlowModeIndicator.svelte`, `RemoteProfileCard.svelte`, `KeyboardShortcuts.svelte`, `SearchBar.svelte`.



Tag `v0.31-work-threads` … `v0.35-forum-federation` per milestone, each
with its ledger rows flipped and evidence named. The §16 checklist applies
to every tag. When M35 lands, update §17's revision note to say the forum-
first surface is implemented, and update `docs/verification.md`.

---

## 15b. Milestone 36 (repo) — Growth, Sharing & Ecosystem Expansion
Spec §36. Depends on: M31–M35, M16 (recommendation engines), M11 (adapter).

**Migration `0044_growth.sql` (both dialects):**
`quote_cards (id, work_id, author_pseud, start_pos, end_pos, theme, image_url, short_url, qr_url, created_at)`,
`import_jobs (id, profile_url, status, works_total, works_done, error, enqueued_at, completed_at)`,
`prompts (id, text, fandom_scope, author_pseud, created_at)`,
`prompt_votes (prompt_id, pseud, vote_type, created_at, PRIMARY KEY (prompt_id, pseud))`,
`prompt_adoptions (prompt_id, pseud, adopted_at, completed_at, PRIMARY KEY (prompt_id, pseud))`,
`gift_exchanges (id, title, signup_start, signup_end, matching_date, creation_end, reveal_date, status)`,
`exchange_participants (exchange_id, pseud, letter, fandoms, tropes, dnws, min_words, matched_pseud, created_at)`,
`beta_requests (id, work_id, focus_areas, turnaround_days, min_fandom_knowledge, word_count, status, created_at)`,
`beta_claims (request_id, pseud, status, claimed_at, completed_at, PRIMARY KEY (request_id, pseud))`,
`beta_annotations (id, claim_id, post_id, span_start, span_end, comment, status, created_at)`,
`recommendation_requests (topic_id PRIMARY KEY, description, filters_json, created_at)`,
`fan_art (id, work_id, artist_pseud, storage_key, status, cover_for_work_id, created_at)`,
`interactive_branches (id, work_id, from_choice, to_choice, label, position, created_at)`,
`reading_paths (reader_pseud, work_id, mood_tag, private_note, created_at, updated_at)`,
`mood_tags (work_id, reader_pseud, tag, note, created_at, PRIMARY KEY (work_id, reader_pseud))`,
`author_analytics (work_id, date, views, chapter_dropoffs_json, referral_sources_json, geo_json, device_json, completion_rate, PRIMARY KEY (work_id, date))`,
`webhooks (id, pseud, url, secret, events_json, active, created_at, updated_at)`,
`webhook_deliveries (id, webhook_id, event_type, payload_json, status_code, response_body, idempotency_key, created_at)`,
`instance_directory (instance_host PRIMARY KEY, name, description, fandoms_json, moderation_style, federation_scope, stats_json, signed_at)`,
`migration_jobs (id, source_instance, status, total_items, done_items, enqueued_at, completed_at)`,
`migration_items (job_id, item_type, item_id, status, error, created_at)`.

**Ordering:** quote cards → import → prompts → gift exchanges → beta marketplace → rec threads → fan art → interactive fiction → mood journal → analytics → SEO → webhooks/CLI → instance discovery/migration.

**Acceptance tests (`milestone_36.rs`):**
- `quote_card_contains_exact_text`
- `mirror_import_never_alters_source`
- `cross_post_fails_cleanly`
- `ffn_adapter_backsoff_on_429`
- `wattpad_adapter_completes_import`
- `wrapped_page_renders_at_375px`
- `daily_prompt_highest_voted_unused`
- `gift_exchange_matching_deterministic`
- `beta_annotations_private_to_author_and_reader`
- `auto_recommendation_labeled_machine_generated`
- `fan_art_invisible_until_approved`
- `interactive_fiction_branch_navigation`
- `mood_journal_searchable_by_mood`
- `analytics_suppresses_under_10_views`
- `json_ld_structured_data_emitted`
- `webhook_includes_hmac_and_idempotency_key`
- `migration_source_becomes_tombstone`

**Frontend:** `QuoteCard.svelte`, `ImportProgress.svelte`, `WrappedPage.svelte`, `PromptFeed.svelte`, `ExchangeSignup.svelte`, `BetaMarketplace.svelte`, `RecommendationThread.svelte`, `FanArtGallery.svelte`, `InteractiveFictionReader.svelte`, `MoodJournal.svelte`, `AuthorAnalytics.svelte`, `WebhookConfig.svelte`, `InstanceDirectory.svelte`, `MigrationWizard.svelte`, `SearchBar.svelte`.

---

### 15b.1 Sign-off

Tag `v0.36-growth-ecosystem` with its ledger rows flipped and evidence named.
The §16 checklist applies to the tag. When M36 lands, the forum-first and
growth surfaces are complete, and the specification at §35 + §36 is fully
represented in code.

---

## 15c. Milestone 37 (repo) — Multi-Platform Companion Bot
Spec §37. Depends on: §23.1 (public API), M31 (forum routes).

**Workspace layout** (separate `lorehaven-bot` workspace):
```
lorehaven-bot/
├── crates/
│   ├── bot-core/                    # platform-neutral core
│   ├── lorehaven-bot-discord/       # Poise + Serenity
│   ├── lorehaven-bot-telegram/      # Teloxide
│   ├── lorehaven-bot-matrix/        # matrix-sdk
│   ├── lorehaven-bot-irc/           # irc crate
│   ├── lorehaven-bot-fediverse/     # Mastodon + Bluesky
│   └── lorehaven-bot-cli/           # REPL + TUI + download
├── Cargo.toml                       # workspace root
└── .env.example                     # all LOHAVEN_BOT_* vars
```

**bot-core crate** (`crates/bot-core/`):
- `Cargo.toml`: `reqwest`, `serde`, `serde_json`, `redis`, `chrono`, `thiserror`, `tracing`, `url`
- `src/api.rs` — `LorehavenClient` with methods: `search`, `recommendations`, `work`, `work_download`, `work_bookmark`, `work_kudos`, `forum_categories`, `forum_topics`, `forum_topic`, `forum_post_create`, `fandoms`, `fandom`, `tags`, `random`, `trending`, `similar`, `also_bookmarked`, `blind_date`, `comments`, `reading_status`, `user`, `user_works`, `user_bookmarks`, `notifications`, `me`, `link_token`, `link`, `unlink`
- `src/model.rs` — response structs: `SearchResponse`, `SearchResult`, `Work`, `WorkDownload`, `ForumCategory`, `ForumTopic`, `ForumPost`, `Fandom`, `Tag`, `RecResult`, `User`, `Notification`, `LinkToken`
- `src/core.rs` — `PlatformMessage` IR: `title`, `description`, `url`, `fields: Vec<RichItem>`, `actions: Vec<ActionRow>`, `file: Option<FileAttachment>`
- `src/dispatch.rs` — `CoreCtx` + `do_*` functions: `do_search`, `do_ask`, `do_recs`, `do_fresh`, `do_gems`, `do_roll`, `do_download`, `do_metadata`, `do_bookmark`, `do_kudos`, `do_work`, `do_fandoms`, `do_fandom`, `do_forum_categories`, `do_forum_topics`, `do_forum_topic`, `do_forum_create`, `do_forum_reply`, `do_forum_follow`, `do_forum_mark_read`, `do_forum_search`, `do_blind_date`, `do_trending`, `do_similar`, `do_also_bookmarked`, `do_comments`, `do_random`, `do_help`, `do_link`, `do_unlink`, `do_me`
- `src/intent.rs` — `Intent` enum: `Search`, `Ask`, `Quote`, `Recs`, `Fresh`, `Gems`, `Roll`, `Download`, `Bookmark`, `Metadata`, `Help`, `Fandoms`, `Fandom`, `Work`, `Forum`, `BlindDate`, `Trending`, `Similar`, `AlsoBookmarked`, `Comments`, `Random`, `Link`, `Unlink`, `Me`. `classify(text) -> Intent` with heuristic fallback + optional Ollama
- `src/store.rs` — `TokenStore` (Redis-backed): `get(platform, user_id) -> Option<StoredToken>`, `set(platform, user_id, token)`, `del(platform, user_id)`, `get_prefs(platform, user_id) -> UserPrefs`, `set_prefs(platform, user_id, prefs)`, `create_link_code(platform, user_id) -> String`, `claim_link_code(code) -> Option<(String, String)>`
- `src/cache.rs` — `PageCache` (Redis-backed): `cache_response(key, value, ttl)`, `cached_response(key) -> Option<Value>`, `log_search(entry)`, `get_page(user_id) -> usize`, `set_page(user_id, page)`
- `src/ratelimit.rs` — `RateLimiter`: `check(platform, user_id) -> bool` (Redis bucket, configurable per-minute)
- `src/config.rs` — `BotConfig` from env vars (all `LOHAVEN_BOT_*`)
- `src/error.rs` — `BotError` enum: `Api`, `Redis`, `RateLimit`, `NotLinked`, `InsufficientLevel`, `Command(String)`
- `src/util.rs` — `extract_fanfic_url(text) -> Option<String>`, `format_words(n)`, `truncate(s, len)`, `normalize_url(u)`

**Discord adapter** (`crates/lorehaven-bot-discord/`):
- `Cargo.toml`: `poise`, `serenity`, `archivist-core`
- `src/main.rs` — build `Dispatcher` with Poise framework, register commands, start Serenity `Client`
- `src/commands/mod.rs` — re-export all command modules
- `src/commands/search.rs`, `recs.rs`, `download.rs`, `metadata.rs`, `bookmark.rs`, `kudos.rs`, `work.rs`, `fandoms.rs`, `fandom.rs`, `forum.rs`, `blind_date.rs`, `trending.rs`, `similar.rs`, `also_bookmarked.rs`, `comments.rs`, `random.rs`, `help.rs`, `link.rs`, `unlink.rs`, `me.rs`, `guild.rs` — one file per command group
- `src/render.rs` — `to_embed(PlatformMessage) -> CreateEmbed`, `to_components(PlatformMessage) -> Vec<ActionRow>`
- `src/intent.rs` — `on_mention(ctx, msg) -> Result<()>`: strip @mention, call `intent::classify`, dispatch

**Telegram adapter** (`crates/lorehaven-bot-telegram/`):
- `Cargo.toml`: `teloxide`, `archivist-core`
- `src/main.rs` — build `Dispatcher` with dptree, start Teloxide `Bot`
- `src/commands.rs` — `#[derive(BotCommands)]` enum mirroring Discord surface
- `src/handlers.rs` — one handler per command, calling `do_*` from core
- `src/render.rs` — `to_html(PlatformMessage) -> String`, `to_inline_keyboard(PlatformMessage) -> InlineKeyboardMarkup`
- `src/intent.rs` — inline query handler + free-text handler

**Matrix adapter** (`crates/lorehaven-bot-matrix/`):
- `Cargo.toml`: `matrix-sdk`, `archivist-core`
- `src/main.rs` — login, join rooms, start sync loop
- `src/parse.rs` — `detect_command(text) -> Option<Command>`, `strip_prefixes(text) -> String`
- `src/render.rs` — `render_message(PlatformMessage) -> RoomMessageEventContent`

**IRC adapter** (`crates/lorehaven-bot-irc/`):
- `Cargo.toml`: `irc`, `archivist-core`
- `src/main.rs` — connect, join channels, listen for messages
- `src/command.rs` — `parse_line(text) -> IrcCommand`
- `src/render.rs` — `render_plain(PlatformMessage) -> String` with numbered actions

**Fediverse adapter** (`crates/lorehaven-bot-fediverse/`):
- `Cargo.toml`: `reqwest`, `serde`, `archivist-core`
- `src/main.rs` — spawn Mastodon poller + Bluesky poller + optional Piefed monitor
- `src/mastodon.rs` — `poll_mentions() -> Vec<Mention>`, `reply_to(mention, PlatformMessage)`, `post_work(work, quote_card_url)`
- `src/bsky.rs` — `poll_mentions() -> Vec<BskyNotification>`, `reply_to(notif, PlatformMessage)` via raw XRPC
- `src/parse.rs` — `strip_mention(text) -> String`, `parse_command(text) -> Option<Command>`, `render_plain(PlatformMessage) -> String`
- `src/config.rs` — `FediverseConfig` from env

**CLI/TUI adapter** (`crates/lorehaven-bot-cli/`):
- `Cargo.toml`: `clap`, `tokio`, `crossterm`, `ratatui`, `archivist-core`
- `src/main.rs` — `clap` subcommands: `repl`, `tui`, `download`, `search`, `work`, `forum`
- `src/repl.rs` — `Repl` struct, `parse_line(raw) -> Line`, `run(config, client, token, user_id)`
- `src/tui.rs` — `run(config, client, token, user_id, force)` entry point
- `src/tui/state.rs` — `AppState`, `BrowseMode`, `Detail`, `ListState`, `Pane`, `Tab`
- `src/tui/worker.rs` — `WorkerCmd`, `TuiMsg`, background task owning `CoreCtx`
- `src/tui/commands.rs` — `TuiCommand` enum, `parse_input(raw) -> TuiCommand`
- `src/download.rs` — `download_work(client, url, format, path) -> Result<()>`
- `src/render.rs` — `render_search_json(results)`, `render_work_json(work)`, `render_forum_json(topic)`

**bot-core unit tests** (in `crates/bot-core/tests/`):
- `intent.rs` — `classify_search_query`, `classify_url`, `classify_question`, `classify_help`, `classify_fandom`, `classify_forum`, `classify_blind_date`, `classify_trending`, `classify_random`, `classify_link`, `classify_unknown`
- `store.rs` — `token_roundtrip`, `link_code_claim`, `link_code_expiry`, `prefs_roundtrip`
- `cache.rs` — `cache_response_roundtrip`, `page_pointer_roundtrip`
- `ratelimit.rs` — `rate_limit_allows_under_limit`, `rate_limit_blocks_at_limit`, `rate_limit_resets_after_window`
- `util.rs` — `extract_url_from_text`, `format_words_trousands`, `truncate_preserves_short`, `truncate_cuts_long`

**Adapter integration tests** (in each adapter crate's `tests/`):
- `discord::test_search_renders_embed`
- `telegram::test_search_renders_html`
- `matrix::test_parse_detects_command`
- `irc::test_parse_line_command`
- `fediverse::test_mastodon_parse_mention`
- `cli::test_repl_parse_line`
- `cli::test_tui_parse_input`

**Acceptance tests** (`milestone_37.rs` in bot-core):
- `search_returns_results` — `do_search` returns `PlatformMessage` with items
- `recs_requires_auth` — `do_recs` fails with `NotLinked` when no token
- `download_returns_links` — `do_download` returns EPUB/PDF/MOBI links
- `bookmark_requires_auth` — `do_bookmark` fails with `NotLinked` when no token
- `kudos_requires_auth` — `do_kudos` fails with `NotLinked` when no token
- `forum_categories_returns_list` — `do_forum_categories` returns categories
- `forum_topics_returns_list` — `do_forum_topics` returns topics
- `forum_topic_returns_detail` — `do_forum_topic` returns thread
- `fandoms_returns_list` — `do_fandoms` returns fandoms
- `fandom_returns_detail` — `do_fandom` returns fandom
- `random_returns_work` — `do_random` returns a work
- `trending_returns_list` — `do_trending` returns works
- `similar_returns_list` — `do_similar` returns works
- `also_bookmarked_returns_list` — `do_also_bookmarked` returns works
- `blind_date_returns_work` — `do_blind_date` returns a work
- `comments_returns_list` — `do_comments` returns comments
- `link_creates_code` — `do_link` returns a link code
- `unlink_deletes_token` — `do_unlink` deletes the stored token
- `me_returns_profile` — `do_me` returns the linked user's profile
- `help_returns_text` — `do_help` returns help text
- `intent_classifies_search` — `intent::classify("search drarry")` → `Intent::Search`
- `intent_classifies_url` — `intent::classify("https://...")` → `Intent::Metadata`
- `intent_classifies_question` — `intent::classify("how do I...")` → `Intent::Ask`
- `rate_limit_allows_under_limit` — 9 commands in 60s → allowed
- `rate_limit_blocks_at_limit` — 11 commands in 60s → blocked
- `cache_hit_returns_value` — cached search response returned without API call
- `pagination_next_advances` — `do_search` with page=2 returns next page
- `pagination_prev_returns` — `do_search` with page=1 returns previous page

**Frontend:** None (bot is API-only; no Svelte components).

---

### 15c.1 Sign-off

Tag `v0.37-companion-bot` with its ledger rows flipped and evidence named.
The §16 checklist applies to the tag. When M37 lands, the companion bot
covers Discord, Telegram, Matrix, IRC, the Fediverse (Mastodon + Bluesky),
and the terminal (REPL + TUI + download).

---

## 15d. Milestone 38 (repo) — Instance Configuration
Spec §38. Depends on: none (infrastructure).

**Goal**: Every hardcoded `const` in application code becomes a `Config` field
with a TOML key, documented default, and startup validation.

**Migration path** (per value):
1. Add field to `*Config` struct with `serde::Deserialize`.
2. Add default to `*Config::default()`.
3. Read in `Config::load()` from TOML.
4. Replace `const` reference with `config.xxx`.
5. Update spec §38.2/§38.3.

**Values to migrate** (from spec §38.3):

| Constant | Proposed key | Default |
|----------|--------------|---------|
| `RETENTION_DAYS = 7` | `[exports] retention_days` | `0` (forever) |
| `GRANT_TTL_SECONDS = 3600` | `[exports] grant_ttl_secs` | `3600` |
| `REVISION_TTL_SECONDS = 604800` | `[revisions] ttl_secs` | `604800` |
| `TERMINAL_JOB_RETENTION = 30d` | `[jobs] terminal_retention_days` | `30` |
| `DEFAULT_MAX_ITEMS = 50` | `[bulk_export] max_items` | `50` |
| `DEFAULT_MAX_BYTES = 1GiB` | `[bulk_export] max_bytes` | `1073741824` |
| `UPDATE_CHECK_RETENTION_DAYS = 90` | `[library] update_check_retention_days` | `90` |
| `CHECK_BATCH = 50` | `[library] check_batch` | `50` |
| `webhook_timeout_secs = 10` | `[administration] webhook_timeout_secs` | `10` |
| `webhook_max_attempts = 5` | `[administration] webhook_max_attempts` | `5` |
| `webhook_base_delay_ms = 500` | `[administration] webhook_base_delay_ms` | `500` |

**New config structs**:
- `ExportsConfig` — `retention_days`, `grant_ttl_secs`
- `RevisionsConfig` — `ttl_secs`
- `JobsConfig` — `terminal_retention_days`
- `BulkExportConfig` — already exists; add `max_items`, `max_bytes`
- `LibraryConfig` — `update_check_retention_days`, `check_batch`
- `AdministrationConfig` — already exists; add webhook fields

**Acceptance tests** (`milestone_38.rs`):
- `retention_zero_disables_cleanup` — `retention_days = 0` → sweep removes nothing
- `retention_seven_sweeps_old` — `retention_days = 7` → old exports purged
- `grant_ttl_respected` — grant expires after configured seconds
- `revision_ttl_respected` — revision discarded after configured seconds
- `terminal_job_retention_respected` — old terminal jobs purged
- `bulk_export_max_items_enforced` — bulk export capped at `max_items`
- `bulk_export_max_bytes_enforced` — bulk export capped at `max_bytes`
- `malformed_config_rejected` — bad value → startup error with clear message
- `defaults_produce_working_instance` — empty config → server starts
- `all_existing_tests_pass` — full suite green with default config

---

## 15e. Milestone 39 (repo) — Resource Directory

Spec §39. Depends on: M2 (accounts), M11 pattern (routes+db+domain split).

**Read first:** spec §39 in full, §38 (self-hosted contract — the directory
is operator-configurable), and §14.9's SSRF rules (URL validation reuses the
same posture).

### 39.1 Why this is next

The directory is self-contained: no dependency on any unbuilt milestone, no
interaction with the work pipeline, and every piece (migration, domain
validation, repo, routes, page) follows the M11–M12 pattern exactly. It is
the safest possible milestone for a junior to learn the loop on, and it
delivers a visible public surface on day one.

### 39.2 Ledger rows (add before any code)

- `M39-01` directory: ranked, category-browsed list of external resources
  (spec §39.1–39.2)
- `M39-02` directory: submission with operator review queue (spec §39.3)
- `M39-03` directory: one-vote-per-account toggle voting with denormalised
  score (spec §39.4)
- `M39-04` directory: URL validation refusing non-http(s) and private
  addresses (spec §39.3)
- `M39-05` directory: operator-configurable categories via TOML (spec §39.2,
  §38)

### 39.3 Migration `0046_resource_directory.sql` (both dialects)

```text
directory_entries(
  id TEXT PRIMARY KEY,            -- uuid
  category TEXT NOT NULL,
  title TEXT NOT NULL,
  url TEXT NOT NULL,
  description TEXT NOT NULL DEFAULT '',
  submitted_by TEXT NOT NULL,     -- account id, not pseud: votes/submit are account-scoped
  approved_by TEXT,               -- NULL = pending
  created_at TEXT NOT NULL,       -- RFC 3339
  updated_at TEXT NOT NULL,
  score INTEGER NOT NULL DEFAULT 0  -- denormalised sum of live votes
)
index (category, score DESC, created_at)
index (submitted_by, created_at DESC)

directory_entry_tags(
  entry_id TEXT NOT NULL,
  tag TEXT NOT NULL,              -- lowercase, trimmed, ≤ 32 chars
  PRIMARY KEY (entry_id, tag)
)
index (tag)

directory_votes(
  entry_id TEXT NOT NULL,
  account_id TEXT NOT NULL,
  vote_value INTEGER NOT NULL CHECK (vote_value IN (-1, 1)),
  voted_at TEXT NOT NULL,
  PRIMARY KEY (entry_id, account_id)
)
index (entry_id)
```

SQLite: identical (all TEXT/INTEGER). Postgres: same columns; `id` and
`entry_id`/`account_id` bind as `TEXT + ?::uuid` per ADR 0004 only where the
codebase already does so for account ids — follow `crates/db/src/community.rs`'s
existing convention for account-id columns (plain TEXT is used there).

### 39.4 Domain module `crates/domain/src/directory.rs`

Pure functions, unit tests in-file, no I/O:

- `validate_url(&str) -> Result<(), AppError>`: absolute http(s) only;
  host must not be `localhost`, `*.localhost`, a loopback IP (v4/v6), a
  private range (10/8, 172.16/12, 192.168/16, 169.254/16, fd00::/8) or
  `[::1]`. Return `AppError::Validation` with a named reason. Reuse
  `crates/domain/src/webhooks.rs`'s URL checks if a shared helper exists
  there — check before writing a second copy (DRY).
- `validate_title(&str) -> Result<(), AppError>`: 1–120 chars after trim.
- `validate_description(&str) -> Result<(), AppError>`: ≤ 500 chars.
- `normalize_tag(&str) -> Option<String>`: lowercase, trim, collapse
  whitespace, None if empty or > 32 chars.
- `normalize_tags(&[String]) -> Vec<String>`: map + dedupe + sort.

Unit tests (in-file, `#[cfg(test)]`): each validator's happy path and each
refusal; `normalize_tags` dedupes and sorts; a URL with a userinfo trick
(`http://user@127.0.0.1/`) is refused.

### 39.5 Repository `crates/db/src/directory.rs`

Follow `crates/db/src/discovery.rs` for the Backend match pattern. Functions:

- `submit_entry(db, id, category, title, url, description, submitted_by, now)`
  — INSERT with `approved_by = NULL`.
- `approve_entry(db, id, operator_id, now) -> Result<bool>`
- `remove_entry(db, id) -> Result<bool>` (operator; sets no tombstone — rows
  stay, but `list_entries` excludes removed via a `removed_at TEXT` column
  you add to the migration; simpler: hard DELETE. Choose hard DELETE; votes
  and tags cascade or are left orphaned intentionally — document the choice
  in the module doc-comment. Prefer: DELETE the entry row, leave
  votes/tags rows (audit), and make `list_entries` JOIN on entries so
  orphans are invisible.)
- `list_entries(db, filter) -> Result<Vec<DirectoryEntryWithScore>>` where
  `DirectoryEntryFilter { category, tag, q, sort: Top|New, limit, offset,
  viewer: Option<String>, is_operator: bool }`. Visibility rule in SQL:
  `(approved_by IS NOT NULL OR submitted_by = ?viewer OR ?is_operator)`.
  JOIN tags when `tag` is set; LIKE on title/description when `q` is set
  (lower(title) LIKE '%'||lower(?)||'%' — same pattern as
  `forum_search`'s SQLite path).
- `get_entry(db, id, viewer, is_operator) -> Result<Option<DirectoryEntry>>`
- `set_vote(db, entry_id, account_id, value, now) -> Result<bool>`:
  same-value DELETE (toggle off), other-value UPDATE, no-row INSERT —
  then `UPDATE directory_entries SET score = (SELECT COALESCE(SUM(vote_value),0)
  FROM directory_votes WHERE entry_id = ?) WHERE id = ?` **in the same
  transaction** (`db.begin()`/commit — follow an existing transactional
  example in `crates/db/src/` before writing one). Returns true if a live
  vote remains.
- `my_vote(db, entry_id, account_id) -> Result<Option<i64>>`
- `category_counts(db) -> Result<Vec<(String, i64)>>` — approved only.
- `pending_entries(db) -> Result<Vec<DirectoryEntry>>` — operator queue.

Register `pub mod directory;` in `crates/db/src/lib.rs`.

### 39.6 Config `crates/app/src/config.rs`

```rust
#[derive(Debug, Clone, serde::Deserialize)]
#[serde(default)]
pub struct DirectoryConfig {
    pub enabled: bool,               // default true
    pub require_approval: bool,      // default true
    pub page_size: i64,              // default 50
    pub extra_categories: Vec<String>,
}
```

Add `pub directory: DirectoryConfig` to `Config`, the TOML key
`[directory]`, and document it in `lorehaven.toml.example`. Seed categories
are a `const` in the domain module; effective categories = seed +
`extra_categories` (validated: lowercase snake_case, unique).

### 39.7 Routes `crates/app/src/routes/directory.rs`

```
GET  /directory?category=&tag=&q=&sort=&limit=&offset=
GET  /directory/categories
GET  /directory/{id}
POST /directory                      (RequireSession) submit
POST /directory/{id}/vote            (RequireSession) { value: 1 | -1 }
POST /directory/{id}/approve         (operator gate, same pattern as
                                      discovery.rs::require_operator)
DELETE /directory/{id}               (operator gate)
GET  /directory/pending              (operator gate)
```

Wire `pub mod directory;` into `routes/mod.rs` and
`.nest("/directory", directory::router())` in `server.rs`'s API router —
check how `community.rs` is nested and copy exactly. When
`config.directory.enabled == false`, every route returns 404 (gate inside
`router()` construction, same approach `admin_router` uses).

Vote handler: parse `value` (must be 1 or -1), call `set_vote`, return
`{ "score": <new>, "my_vote": <1|-1|null> }`.

### 39.8 Page `frontend/src/routes/Directory.svelte`

Follow `Discover.svelte`'s structure (loading/error/empty states, session
import from `../lib/session.svelte.ts`, api from `../lib/api`):

- Category tabs from `GET /directory/categories` (active tab in URL query).
- Search input (debounced 300ms) + tag chips from the entry list.
- Entry cards: `<a href={entry.url}>` title, description, score, vote
  buttons (▲ ▼ with active state from `my_vote`), submitter handle, date.
- Signed-in: "Submit an entry" opens a form (category select, title, URL,
  description, tags comma-separated). Pending entries show a
  "pending review" badge when the viewer submitted them.
- Load-more button at the list tail (offset += page_size).

Router: add `'directory'` to the route union in
`frontend/src/lib/router.ts`, match `^/directory$`, add the
`{:else if route.id === 'directory'}` branch in `App.svelte`, add
`Directory` to the nav (after Discover) in
`frontend/src/lib/components/Nav.svelte`.

API functions in `frontend/src/lib/api.ts`:
`listDirectoryEntries(opts)`, `listDirectoryCategories()`,
`getDirectoryEntry(id)`, `submitDirectoryEntry(data)`,
`voteDirectoryEntry(id, value)`.

### 39.9 Hand journey (drive before writing tests)

Run the server locally against SQLite, register two accounts (one is the
operator via config), and drive: submit → pending invisible to account two →
operator approves → visible → account two upvotes → score 1 → upvotes again
→ score 0 → downvotes → score -1. Submit `http://127.0.0.1/x` and watch the
refusal name the reason. Then write the tests below to encode exactly what
you drove.

### 39.10 Acceptance tests `crates/app/tests/milestone_39.rs`

Follow `milestone_12.rs`'s test-app harness. Tests:

- `an_entry_can_be_submitted_approved_and_voted_on`
- `pending_entries_are_invisible_to_strangers`
- `voting_twice_toggles_and_flipping_changes_the_score`
- `internal_urls_are_refused_with_a_named_reason`
- `operators_see_the_pending_queue_and_can_remove`
- `extra_categories_appear_without_code_changes` (construct Config with an
  extra category, assert it's in the effective list)

### 39.11 Pitfalls

- **Score drift**: recompute the score in the vote transaction, never in a
  follow-up query — a crash between them leaves a stale score.
- **SQL injection in q**: use bound parameters in LIKE patterns, never
  string interpolation.
- **Approval leak**: the visibility rule must be in SQL, not filtered in
  Rust after fetch — pagination counts would disagree with the page.
- **Category case**: compare categories case-insensitively on input,
  store lowercase.

---

## 15f. Milestone 40 (repo) — Fork with provenance and permission statements

Spec §40 (and §33.1, which it implements). Depends on: M3 (drafts), §32
derivative tables (migration 0030), M33.1 permission statement model.

**Read first:** spec §40 and §33.1 in full, §32.3 (orphaning), and
`migrations/postgres/0030_*.sql` for the existing derivative tables.

### 40.1 Why this is next

The derivative data model has existed since migration 0030 and §33.1 has
been spec-only. Everything the fork needs is built: drafts (M3), lineage
edges (0030), permission statements (M33.1). This milestone is pure
composition — no new subsystems, only an affordance over existing ones.

### 40.2 Ledger rows

- `M40-01` fork action creating an empty draft with lineage edge, gated by
  permission statement, exclusion registry and depth limit (spec §40.1)
- `M40-02` permission statements editable per work with effective-sentence
  UI (spec §40.2)
- `M40-03` Remixes list on parent, parent link on child, orphan-safe
  (spec §40.1)
- `M40-04` statement enforcement at fork, translation request and
  narration request doors (spec §40.2)

### 40.3 Migration `0047_fork_permissions.sql` (both dialects)

First read migration 0030 and M33.1's migration (0036) — do not duplicate
tables that exist. What 0030/0036 already provide: derivative lineage edges
and permission statements. Verify with `grep -rn 'permission' migrations/`
and `grep -rn 'derivative' migrations/` before writing anything. This
migration adds only what is missing:

```text
-- only if 0036 does not already store per-work statements:
work_permission_statements(
  work_id TEXT PRIMARY KEY,
  podfic TEXT NOT NULL DEFAULT 'unstated',        -- yes|ask|no|unstated
  translation TEXT NOT NULL DEFAULT 'unstated',
  remix TEXT NOT NULL DEFAULT 'unstated',
  continuation TEXT NOT NULL DEFAULT 'unstated',
  redistribution TEXT NOT NULL DEFAULT 'unstated',
  ai_training TEXT NOT NULL DEFAULT 'unstated',
  updated_at TEXT NOT NULL
)

-- only if 0030 does not already carry fork depth:
-- (check first; if lineage edges exist, depth is computed by walking them,
-- and no column is needed)
```

Statement changes audit (append-only):

```text
work_permission_statement_changes(
  id TEXT PRIMARY KEY,
  work_id TEXT NOT NULL,
  changed_by TEXT NOT NULL,
  field TEXT NOT NULL,             -- 'remix' etc.
  was TEXT NOT NULL,
  now_value TEXT NOT NULL,
  changed_at TEXT NOT NULL
)
```

### 40.4 Domain `crates/domain/src/fork.rs`

Pure functions:

- `check_fork(parent_statements, exclusion_hit, chain_depth, max_depth)
  -> ForkCheck` where `ForkCheck = Proceed | Refuse { reason: String } |
  NeedsAsk`. Reasons name the statement ("remix: no") or the limit
  ("fork depth 3 reached: A → B → C → this work").
- `effective_sentence(field, value) -> &'static str` — the plain sentences
  from spec §40.2, unit-tested for all 4 values × 6 fields.
- `chain_display(chain: &[&str]) -> String` — "A → B → C".

### 40.5 Repository `crates/db/src/fork.rs`

- `statements_for(db, work_id) -> Result<Statements>` (defaults when no row)
- `set_statement(db, work_id, field, value, changed_by, now)` — upsert +
  audit row, in one transaction.
- `lineage_chain(db, work_id) -> Result<Vec<String>>` — walk parent edges to
  the root (bounded by config max_depth × 2 iterations; a cycle returns the
  prefix seen — cycles must not hang the walker; unit-test with a fixture).
- `create_fork(db, parent_work_id, forker_pseud, now) -> Result<ForkOutcome>`
  — creates a draft work (reuse `content::create_work`), copies tag/fandom/
  character links (INSERT ... SELECT from the parent's rows), writes the
  lineage edge (`kind = 'remix'`), all in one transaction.
- `children_of(db, work_id) -> Result<Vec<ChildRef>>` — for the Remixes list.

### 40.6 Routes

In `crates/app/src/routes/works.rs` (extend, don't create a module — the
work page owns this surface):

- `POST /works/{id}/fork` (RequirePseud) — runs `check_fork`, then
  `NeedsAsk` → 422 with `{ "needs_ask": true }`; `Refuse` → 403 with the
  named reason; `Proceed` → `create_fork`, return the draft's id.
- `GET /works/{id}/permissions` — statements + effective sentences.
- `PUT /works/{id}/permissions` (owner only — reuse the existing
  ownership check in works.rs) — one or more fields, each validated to the
  4-value enum.
- `GET /works/{id}/remixes` — children list.

Enforcement hooks (one line each, at the top of the existing handler):
- translation request creation (M17 routes) checks `translation`.
- narration request (if M26 routes exist) checks `podfic`.

### 40.7 Pages

`WorkPage.svelte`: a **Sharing & permissions** section in the owner's edit
view (six selects with the effective sentence under each); a **Fork this
work** button (signed-in, non-owner) whose click handles the three outcomes
(proceed → navigate to the new draft; needs_ask → inline "the author will be
asked" confirmation; refuse → error banner with the named reason); a
**Remixes** section listing children when non-empty.

### 40.8 Acceptance tests `crates/app/tests/milestone_40.rs`

- `fork_proceeds_on_unstated_and_creates_empty_draft_with_lineage`
- `fork_refused_when_remix_is_no_and_names_the_statement`
- `fork_ask_creates_a_request_not_a_draft`
- `depth_limit_refuses_with_the_chain_displayed`
- `fork_inherits_tags_but_copies_no_body`
- `statements_are_editable_by_owner_only_and_audited`
- `deleting_a_parent_never_deletes_a_child`

### 40.9 Pitfalls

- **Do not re-create lineage tables.** Migration 0030 owns them; read it
  first. If 0036 already stores statements, this milestone only adds the
  affordance and the audit table.
- **Cycle safety.** A lineage chain with a cycle must not hang; bound the
  walk and unit-test it.
- **Copy links, not bodies.** A fork with the parent's text is plagiarism
  tooling; the test `copies_no_body` enforces the empty draft.

---

## 15g. Milestone 41 (repo) — Half-life and interaction tiers

Spec §41. Depends on: M11 (discovery blend), M4 (reading events), M12
(community routes).

**Read first:** spec §41 in full, §16.3 (the silent-reordering contract),
§0.2/§0.3 (attention rules — this milestone lives or dies by them).

### 41.1 Why this is next

Both signals consume tables that already exist (`reading_events`,
`reading_progress`, comments, reactions) and write onto columns this
milestone adds. The half-life job rides the existing jobs infrastructure
(M5); warmth hooks ride existing handlers. No new subsystems.

### 41.2 Ledger rows

- `M41-01` half-life job writing `half_life_bp` on eligible works, used as
  a silent ranking multiplier (spec §41.1)
- `M41-02` warmth accumulation hooks on reading/reaction/comment with
  tier thresholds in config (spec §41.2)
- `M41-03` author-facing aggregate tier panel; no per-reader exposure
  anywhere (spec §41.2, §41.3)

### 41.3 Migration `0048_longevity_signals.sql` (both dialects)

```text
-- half-life: INTEGER basis points, never REAL (ADR 0004)
ALTER TABLE works ADD COLUMN half_life_bp INTEGER;   -- NULL = not yet scored

interaction_warmth(
  account_id TEXT NOT NULL,        -- the reader
  author_account TEXT NOT NULL,    -- the author
  warmth_bp INTEGER NOT NULL DEFAULT 0,
  tier TEXT NOT NULL DEFAULT 'lurk',   -- lurk|react|comment|create
  updated_at TEXT NOT NULL,
  PRIMARY KEY (account_id, author_account)
)
index (author_account, tier)
```

### 41.4 Domain `crates/domain/src/longevity.rs`

- `half_life_bp(recent_starts: i64, first_window_starts: i64) -> i64` —
  basis points, `first_window_starts == 0` → 0; clamp 0..=10000. Unit tests:
  zero-window, equal windows (10000), half (5000), clamp.
- `apply_half_life(candidates: Vec<Candidate>, half_life_of: &dyn Fn(&WorkId)
  -> Option<i64>) -> Vec<Candidate>` — multiplier `1.0 + bp/20000.0` (so
  10000bp = 1.5×, 0bp = 1.0×), score scaled and re-sorted; **no field added,
  no reason changed** — the §16.3 silent contract. Unit test asserts field
  equality apart from score.
- `warmth_delta(action) -> i64` — reading a chapter 100, finishing 500,
  reacting 200, commenting 400 (basis points).
- `tier_for(warmth_bp, thresholds) -> &'static str` — thresholds from
  config `[community].warmth_thresholds` (lurk 0, react 200, comment 1000,
  create 3000 defaults); unit tests at each boundary.
- `aggregate_tiers(counts) -> Aggregates` — the author panel shape.

### 41.5 Repository `crates/db/src/longevity.rs`

- `recompute_half_life(db, now) -> Result<u64>` — for each work published
  ≥ `min_age_days` ago: count starts in the trailing `window_days` and in
  the first `window_days` after publication, compute via domain, write
  `half_life_bp`. Idempotent by construction (pure overwrite).
- `record_warmth(db, reader, author, delta, now)` — upsert by pair,
  accumulate, recompute tier, one transaction.
- `author_tier_aggregates(db, author, window_days) -> Result<Aggregates>` —
  counts by tier for authors the viewer owns.

### 41.6 Job + hooks

- Job `crates/app/src/jobs/half_life.rs` (follow an existing job module's
  shape): scheduled nightly when `config.discovery.enable_half_life`;
  calls `recompute_half_life`.
- Hooks (one call each, at the end of the existing handler, inside its
  transaction): `record_reading_progress` (resolve the work's owner —
  skip silently when unresolvable), quick-reaction POST, comment POST.
  A hook failure logs and continues — warmth must never fail a read.

### 41.7 Discovery integration

In `discovery.rs::get_discovery`, after `blend()` and before affinity
ranking, when `enable_half_life`: `apply_half_life(blended, &|id|
half_life_lookup(id))`. Fetch `half_life_bp` for the page's candidates in
one query (`SELECT id, half_life_bp FROM works WHERE id IN (...)`).

### 41.8 Routes + page

- `GET /works/mine/{id}/audience` (owner only) — tier aggregates for one
  work; plus `GET /me/audience` for all the author's works. Both refuse
  non-owners with 404.
- `WorkPage.svelte` owner view: an **Audience** panel (four tier counts,
  this month). No reader-facing surface at all — no route, no UI, no API
  that takes a reader parameter.

### 41.9 Acceptance tests `crates/app/tests/milestone_41.rs`

- `half_life_job_scores_evergreen_above_forgotten` — fixture: work A with
  recent starts, work B without; job; A's bp > B's bp.
- `half_life_changes_ranking_with_no_field_changes` — two candidates, one
  with bp 10000; assert order flips and every field except score is equal.
- `half_life_job_is_idempotent` — run twice, second run changes nothing.
- `warmth_accumulates_and_promotes_tier` — read+react+comment; tier moves.
- `warmth_is_never_exposed_per_reader` — every public route on the work and
  author surfaces is asserted to contain no per-reader warmth value (grep
  the response bodies in the test).
- `failed_interaction_writes_no_warmth` — a rejected comment leaves warmth
  unchanged.

### 41.10 Pitfalls

- **INTEGER, never REAL** for both `half_life_bp` and `warmth_bp` — ADR 0004.
- **The silent contract.** `apply_half_life` must not touch `reason` or add
  fields; the test enforces it, because a future refactor will want to
  "helpfully" annotate.
- **Hook failure isolation.** Warmth hooks log-and-continue; a warmth bug
  must never make reading fail.
- **No reader-facing anything.** If a route name starts looking like
  "my warmth" — stop; the spec forbids it (§41.4).

---

## 16. Cross-cutting sign-off checklist (run at every milestone tag)

- [ ] Ledger: rows added **before** code; flipped after evidence; `M<repo>-NN`
      ids unique; spec §-references present
- [ ] Migrations: both dialects, identical sets; drift test green; the
      migration counter is the next expected number
- [ ] Domain: policy in `crates/domain`, unit-tested, no I/O; error taxonomy
      extended, not bypassed
- [ ] Repository: both dialects written out; binds are `String`/`i64`;
      cursor envelope on lists; transactions where invariants demand
- [ ] Routes: `classified(...)`; CSRF on cookie-authenticated writes;
      extractors gate auth; visibility honoured; limits from configuration
- [ ] Frontend: runes only; field bindings tested; api.ts mirrored; router
      `Planned` never dead-links; labels in `en` and `eo`; state quartet
      (loading/empty/error/success) on every page
- [ ] Tests: acceptance suite in `crates/app/tests/milestone_<n>.rs`;
      component tests beside pages; properties not implementation details;
      bugs found on the way got their tests
- [ ] Docs: `verification.md` rows with real evidence; `README.md` plan table
      current; tutorial chapter if listed; ADRs written for the decisions a
      reviewer would otherwise have to ask about
- [ ] Tag: `v0.<nn>-<name>`, tutorial README's scheme, annotated tag message
      naming the milestone and its ledger rows

### 16.1 The four questions before every tag

1. **Which journey does a human drive, and did they drive it?**
2. **Which invariant does a test prove, and would it fail if broken?**
3. **What does the ledger say, and does the evidence column name a command?**
4. **What are we honestly not doing, and where is that written?**

If any answer is a shrug, the milestone is not done.

### 16.2 Escalation and the honesty vocabulary

When a step cannot be completed: stop, write the gap in
`docs/verification.md` with the status vocabulary, register it in the debt
register (§0.3) if it outlives the milestone, and tell the operator. The
repository's culture is that an honest "not done, here is why" is a
completed task, and a silent downgrade is a defect. Never let a milestone
tag paper over a gap — the spec's own release gate (§25) is built on the
same rule.

### 16.3 A note on spec drift

This plan was written against spec + verification as of 2026-09-11. The spec was revised on 2026-09-14 (work monetization §20.9, subscriptions, saved-search alerts, gifts §18.10, editor blocks §8.3, AI-crawler posture §24.14, and the customization-first reweighting of §6.7/§9.2/§15.4–15.5/§16.2–16.9/§19.1) — milestone 21 below is the skeleton for that revision, and its ledger rows are M21-01..M21-08 in `docs/requirements.csv`. When the
spec changes, update this plan in the same commit as the spec change (the
spec's own header says it is the single source of truth; this plan exists so
a junior never has to re-derive the current state from git archaeology).
If you find this plan contradicting `docs/requirements.csv`, the CSV wins —
fix this plan and note the correction in the commit message.


---


## 15h. Milestone 42 (repo) — Export CTAs (spec §42)

Instance-configurable CTA in exported EPUBs with a curator-quorum
author-CTA exemption.

**Domain** (`crates/domain/src/exports/epub.rs`):
- `CtaPlacement` enum: `PerChapter`, `PerWork`, `Off`; `FromStr`/`as_str`.
- `EpubInput` gains `cta: Option<&EpubCta>` where `EpubCta { html: &str }`.
- `chapter_xhtml` appends the CTA when placement says so for that ordinal
  (every chapter for `PerChapter`, last only for `PerWork`).
- `EpubFacts` gains `cta_chapters: Vec<u32>`; `validate()` extracts them.
- Unit tests: default per-chapter, per-work last-only, off, exemption
  suppresses everywhere, sanitize refuses `<script>`.

**Config** (`crates/app/src/config.rs`):
- `[exports] cta_placement` (default `per_chapter`), `cta_html`
  (default: link to base_url), `cta_quorum` (default 2).
- `cta_html` sanitized through `document::Document::to_sanitized_html` at
  load; unsanitizable input fails validation with a named error.

**DB** (`crates/db/src/exports.rs` + migration 0048):
- `cta_marks` table: `work_id, curator, has_own_cta, marked_at`, PK
  `(work_id, curator)`.
- `mark_cta`, `retract_cta_mark`, `cta_exemption(db, work_id, quorum) ->
  bool` (agreeing true-marks >= quorum), both dialects.

**Routes** (`crates/app/src/routes/exports.rs`):
- `POST /works/{id}/cta-mark {has_own_cta: bool}` — TL3+ only.
- `GET /works/{id}/cta-mark` — the current marks and exemption state.
- Export job passes `cta: None` when exempt, else the configured CTA.

**Tests** (`crates/app/tests/milestone_42.rs`):
|- Full-stack: placement modes, exemption via quorum, retraction restores,
|  TL2 refused, malformed CTA refused at config load.

---

## 15i. Milestone 43 (repo) — Recommendation-first browsing: one ordering contract for every surface

Spec §43, ADRs 0021 (browse ordering contract) and 0022 (demand weight). Depends on: M11 (discovery), M39 (vote_weight primitive), M41 (half-life).

**Read first:** spec §43 in full, §16.15 (taste sources), §16.16 (demand weight), ADRs 0021/0022, and `docs/plans/browse-and-demand-weight.md` (the slice-by-slice implementation plan that this milestone follows).

### 43.1 Why this is next

Every browse surface currently invents its own ordering. §43 ends that: one `Sort` vocabulary, one set of defaults, one per-pseud stickiness rule, on every surface from `/discover` to the directory. The demand-weight primitive (§16.16) reuses the vote-weight machinery M39 built and is the last ranking signal before the gamification guards (M43-15, M43-16).

### 43.2 Ledger rows (spec §43 acceptance + §16.15/§16.16)

Full list in `docs/requirements.csv` (M43-01..M43-16). The rows already exist as `planned`; the work here is to flip them to `implemented-locally-tested` as each slice lands.

### 43.3 Migration 0050 — taste sources (both dialects)

```
taste_sources(
  id TEXT PRIMARY KEY,
  kind TEXT NOT NULL,          -- admin | cohort | roles | combined
  name TEXT NOT NULL,
  min_members INTEGER NOT NULL DEFAULT 5,
  created_at TEXT NOT NULL
);

taste_source_members(
  source_id TEXT NOT NULL,
  account_id TEXT NOT NULL,
  joined_at TEXT NOT NULL,
  PRIMARY KEY (source_id, account_id)
);
```

Config: `[discovery] taste_sources = [{ kind = "admin" }]`, `taste_source_min_members = 5`. Startup refuses a source below the floor with a named error. Member withdrawal triggers a profile recompute for every remaining source.

### 43.4 Domain (`crates/domain/src/` grows)

- `browse.rs`: `Sort` enum (`ForYou`, `New`, `Updated`, `Top`, `Trending`, `BestMatch`, `Az`), `FromStr` that names the accepted set on failure.
- `taste_sources.rs`: source kinds, min-membership validation, member transitions.
- `demand.rs`: promote `vote_weight` to the shared primitive; contribution term (§16.16.1: quality composite, half-life, delivered positive feedback, new-to-instance supply, curatorial labour; never volume/tenure/spend); floor×ceiling `≤ 1.0`; published on `/api/v1/meta`.

### 43.5 Config additions (`crates/app/src/config.rs`)

```toml
[browse]
default_sort = "for-you"
anonymous_sort = "top"
surface_defaults = {}          # per-surface overrides

[discovery]
taste_sources = [{ kind = "admin" }]
taste_source_min_members = 5

[weighting]
mode = "trust_taste_contribution"   # flat | trust | trust_taste | trust_taste_contribution
taste_floor = 0.75
taste_ceiling = 1.25
contribution_floor = 1.0
contribution_ceiling = 2.0
contribution_window_days = 180
demand_diversity_percent = 20
```

Validation: `taste_floor * taste_ceiling ≤ 1.0`, `demand_diversity_percent > 0`, unknown `mode` refuses to start.

### 43.6 Routes / surfaces (§43.1)

Every one of: `/discover`, `/fandoms/:id`, `/tags/:tag`, `/moods/:mood`, `/people`, `/authors/:id`, `/collections/:id`, `/series/:id`, `/reading-paths/:id`, library discovery rails, directory lists, challenge/request/wishlist boards, and the email digest takes `?sort=`. The vocabulary is parsed once in the domain; each surface passes the candidate pool through the same resolver.

`for-you` is a **permutation** — a test asserts set equality: for one fixture, the union of pages under `for-you` equals the union under `new`, in both directions, including a work `for-you` ranks last.

Default stickiness: per-pseud `browse_sort_preferences` table. Anonymous on an undeclared/private-topic instance gets the neutral order (§43.4). A no-profile signed-in request equals the baseline byte-for-byte.

### 43.7 Reason degradation (§43.6, §16.16.2)

A `reason` field may name an instance-level term only for a topic declared `public = true`; otherwise it degrades to one undifferentiated line. Applies to `/discover`, saved responses, digests, and the "why am I seeing this" explanations. Test asserts *absence* of theme terms on every surface below public topics.

### 43.8 Per-surface diversity + exposure measurement (§43.5, §43.6)

§16.4's reservations apply per surface, not only `/discover`. Operator-only aggregate counters: what each sort surfaced, the overlap, the first-seen share. No per-reader history exposed to anyone, including the operator.

### 43.9 Demand weight call sites (§18.5, §20.5)

Wishlist votes, bounty visibility/queue position, prompt votes, and "write-next" ordering consume the weight. The contribution term ignores forbidden metrics by construction; a test feeds a forbidden metric (words published, hours online) and asserts it moves nothing.

### 43.10 Guards (§16.16.3)

- `demand_diversity_percent > 0` enforced; surfaced-without-boost accounting.
- Majority-integrity: compute both weighted and unweighted outcomes; disagreement routes to §19.4 quorum.
- No weight exposure on any surface, export, or error message (§16.16.2, §39.6).
- Leaderboard categories reject volume metrics and all-time boards (§9.7.5).
- No lifetime ladder exists; recognition never gates or governs (asserted across every route and surface).

### 43.11 Pages

A shared sort control component (`SortControl.svelte`) on every §43.1 surface. No new vocabulary — `for-you` is labelled exactly as specced. No surface invents a second label. The weight-blind UI stands free of influence disclosure.

### 43.12 Acceptance tests (`crates/app/tests/milestone_43.rs` and `milestone_44.rs`)

Follow the slice plan in `docs/plans/browse-and-demand-weight.md`. Tests per slice:
1. Unknown `sort` errors naming the accepted set; exact queries bypass every profile.
2. Permutation equality test (for-you vs new, union in both directions).
3. Anonymous gets neutral order on private-topic instance; no-profile signed-in equals baseline.
4. Taste-source min-membership refusal (3 members), acceptance (5), withdrawal recompute.
5. Reason degradation: theme terms absent below public topics.
6. Demand-weight off-switch is identity; forbidden metrics move nothing.
7. Floor×ceiling validation; both published on `/api/v1/meta`.
8. Majority-integrity disagreement routes to quorum.
9. Leaderboard metric-category refusal.
10. Absence test across every endpoint, export, and log shape.

### 43.13 What deliberately ships last

The default `weighting.mode` flips from `flat` only after the guards, the absence tests, and the exposure report are green. Until then `flat` is the honest state, and every paragraph of §16.16 that says "never" is a test before it is a claim.

## 15j. Milestone 44 (repo) — Media Resilience & Availability Guarantee (spec §32.7)

Phase 1 of the §32.7 roadmap. Ships the media reference graph, availability links,
curator rewards, and the public read API. Later phases (reverse search UI, local
mirroring, IPFS, import rescue) build on this foundation.

### 44.1 Why this is next

Link rot silently destroys the reading experience over years: fanfic references
external character art, playlists, moodboards that live on platforms with brutal
churn rates. §32.7 specifies multi-link redundancy with curated mirrors so a
reader is always served a working link.

### 44.2 Ledger rows (spec §32.7 acceptance)

| Requirement | Test |
| --- | --- |
| §32.7.1 media reference graph exists | media_resilience_config_defaults, media_resilience_insert_and_fetch |
| §32.7.4 link health monitoring | media_resilience_availability_link, media_resilience_links_needing_check |
| §32.7.5 curator rewards | media_resilience_curator_rewards |
| §32.7.7 public read API | media_resilience_get_route, media_resilience_missing_returns_404 |

### 44.3 Migration 0060 — media resilience (both dialects)

`media_references`, `availability_links`, `work_media_references`,
`curator_standing_bounties`, `curator_rewards`, `link_health_checks` tables with
matching column/index parity across SQLite and Postgres (guarded by the
`the_two_dialects_declare_the_same_columns_and_indexes` test).

### 44.4 Domain (`crates/domain/src/media_resilience.rs`)

`MediaKind`, `LinkProvider`, `LinkStatus`, `MediaContext`, `CuratorAction`
vocabularies with round-trip string conversions and `ALL` lists.

### 44.5 Config additions (`crates/app/src/config.rs`)

`MediaResilienceConfig`: min_healthy_links, mirror_add_credits, archive_add_credits,
verify_credits, daily_credits_cap, dead_threshold_failures, check_interval_secs.

### 44.6 Routes (`crates/app/src/routes/media_resilience.rs`)

- `GET /media/references/{reference_id}` — public reference + best link
- `POST /media/references/{reference_id}/report-broken` — reader report
- `GET /works/{work_id}/media` — list work's media references
- `POST /works/{work_id}/media` — author adds a reference
- `POST /media/references/{reference_id}/mirrors` — curator adds a mirror

### 44.7 Acceptance tests (`crates/app/tests/media_resilience.rs`)

7 tests: config defaults, reference CRUD, availability link lifecycle
(insert → pending_verification → healthy → count), curator rewards, public
GET route, 404 on missing reference, links-needing-check queue.

### 44.8 What deliberately ships later (per §32.7 phasing)

- Phase 2 (Curation): curator role management, standing bounty matching
- Phase 3 (Author tools): media health dashboard, insertion UI
- Phase 4 (Advanced mirroring): local mirror, IPFS, federation
- Phase 5 (Discovery): reverse image search, MediaReferenceCollaborative strategy
- Phase 6 (Import rescue): bulk media rescue, aggressive-mirror sources

## 15k. Milestone 45 (repo) — Roadmap consensus: Elo-ranked feature board (spec §44)

Spec §44, ADR 0023. Depends on: M14 (trust ladder, `trust_levels` table), M0 (workspace layout).
Ported from FicHub's `fichub-consensus` crate (`~/code/rust/ficnexus/crates/consensus`) — the Elo
math and MaxDiff→match translation are copied from there; storage is rewritten for Lorehaven's
dual-backend `Database` (SQLite + Postgres), and the voter gate is `governance::trust_for`.

**Read first:** spec §44 in full; `crates/db/src/governance.rs` (`trust_for`); ADR 0023.

### 45.1 Migration 0066 (both dialects)

`migrations/sqlite/0066_roadmap_consensus.sql` and `migrations/postgres/0066_roadmap_consensus.sql`:

```sql
CREATE TABLE roadmap_cards (
    id              TEXT PRIMARY KEY,            -- UUID
    title           TEXT NOT NULL,
    category        TEXT NOT NULL DEFAULT 'general',
    stage           TEXT NOT NULL DEFAULT 'idea'
                    CHECK (stage IN ('idea','up_next','in_progress','finished',
                                     'shipped','medium_term','long_term','rejected')),
    elo_rating      REAL NOT NULL DEFAULT 1500.0,
    matches_played  INTEGER NOT NULL DEFAULT 0,
    times_best      INTEGER NOT NULL DEFAULT 0,
    times_worst     INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL,               -- TIMESTAMPTZ on Postgres
    updated_at      TEXT NOT NULL
);
CREATE INDEX idx_roadmap_cards_stage ON roadmap_cards (stage);

CREATE TABLE roadmap_suggestions (
    id          TEXT PRIMARY KEY,
    account_id  TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    raw_text    TEXT NOT NULL,
    card_id     TEXT REFERENCES roadmap_cards(id) ON DELETE SET NULL,
    created_at  TEXT NOT NULL
);

CREATE TABLE roadmap_ballots (
    id           TEXT PRIMARY KEY,
    card_ids     TEXT NOT NULL,   -- JSON array of 4 card ids (JSONB on Postgres)
    served_elo   TEXT NOT NULL,   -- JSON map card_id → pre-match Elo (JSONB on Postgres)
    account_id   TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    created_at   TEXT NOT NULL,
    voted_at     TEXT
);

CREATE TABLE roadmap_moves (
    id          TEXT PRIMARY KEY,
    card_id     TEXT NOT NULL REFERENCES roadmap_cards(id) ON DELETE CASCADE,
    from_stage  TEXT NOT NULL,
    to_stage    TEXT NOT NULL,
    reason      TEXT NOT NULL,
    moved_by    TEXT NOT NULL REFERENCES accounts(id) ON DELETE CASCADE,
    created_at  TEXT NOT NULL
);
```

Postgres dialect: `TEXT` timestamps → `TIMESTAMPTZ`, JSON `TEXT` → `JSONB`, `id TEXT` stays (UUIDs
stored as text — match §44's `card_id` UUID-as-text; consistent with `roadmap_cards.id TEXT`).

Verify: `cargo run -p lorehaven-app -- migrate` applies 0066 on SQLite; same on Postgres via
`DATABASE_URL`.

### 45.2 Domain crate: Elo math (TDD, pure functions)

`crates/domain/src/consensus.rs` — port of the pure math from ficnexus
(`crates/consensus/src/lib.rs` lines 478–500):

```rust
pub fn expected_score(rating_a: f64, rating_b: f64) -> f64 {
    1.0 / (1.0 + 10f64.powf((rating_b - rating_a) / 400.0))
}

pub fn elo_update(rating: f64, opponent_rating: f64, score: f64, k: f64) -> f64 {
    rating + k * (score - expected_score(rating, opponent_rating))
}

/// MaxDiff → two virtual 1v1 matches per unchosen card (spec §44.3):
/// best beats both unchosen, worst loses to both unchosen.
pub fn maxdiff_elo_updates(
    best: (i64, f64),
    worst: (i64, f64),
    unchosen: &[(i64, f64)],
    k: f64,
) -> Vec<(i64, f64)> { /* best +2 wins, worst +2 losses, each unchosen ±1 */ }
```

Register `pub mod consensus;` in `crates/domain/src/lib.rs`. Test first
(`crates/domain/src/consensus.rs` `#[cfg(test)]`): expected_score(1500,1500)=0.5;
elo_update symmetric; best-of-four at equal ratings gains, worst loses; sum of
updates across the four cards is zero (zero-sum). RED → implement → GREEN → commit.

### 45.3 DB layer: cards, ballots, moves

`crates/db/src/roadmap.rs` — same dual-backend pattern as `governance.rs`
(`match db.backend() { Backend::Sqlite => …, Backend::Postgres => … }`):

- `upsert_card(db, card) -> Result<()>` — insert or update by id; never
  downgrades a `shipped` stage (§44.6).
- `find_card_by_title_normalized(db, title) -> Result<Option<Card>>` —
  lowercase, collapse whitespace, strip punctuation.
- `list_cards(db, stage: Option<&str>) -> Result<Vec<Card>>` — Elo DESC,
  tie-break `matches_played` DESC then `card_id` ASC.
- `arena_candidates(db, limit) -> Result<Vec<Card>>` — `stage = 'idea'` only,
  random order (SQLite `RANDOM()`, Postgres `RANDOM()`), limit 4.
- `create_ballot / fetch_ballot / mark_voted(db, ballot_id, account_id)` —
  one-vote-per-ballot enforced by `voted_at IS NULL` check + UPDATE … WHERE.
- `apply_elo_and_counters(db, updates: &[(card_id, new_elo, is_best, is_worst)])`.
- `record_move(db, card_id, from, to, reason, moved_by)`.
- `list_moves(db, limit, offset)`.

Register in `crates/db/src/lib.rs`, add migration 0066 to `migrate.rs`'s file list.

### 45.4 Routes

`crates/app/src/routes/roadmap.rs` — follow `vanguard.rs` for auth patterns:

- `pub fn read_router()` — `GET /roadmap`, `GET /roadmap/changelog`:
  `RouteClass::Default` (anonymous-readable, §44.5).
- `pub fn router()` — `GET /roadmap/arena`, `POST /roadmap/arena`,
  `POST /roadmap/suggest`: session + `governance::trust_for(db, account) >= 1`
  gate, `RouteClass::Write`.
- `pub fn admin_router()` — `POST /admin/roadmap/move`: operator check per
  `admin.rs` pattern.

Wire into `server.rs`'s `api` router with `classified(...)` like the others.
Route inventory test (`route_inventory.rs`) extended with the new paths.

### 45.5 Milestone tests

`crates/app/tests/milestone_45.rs`, patterned on `milestone_43.rs`: seed two
accounts (TL0, TL2 via `governance::set_trust`), seed cards via the same
upsert path the script uses, then per §44.7:

- TL0 vote → 403 with named error; TL2 vote → 200, Elo deltas applied, counters
  incremented, ballot marked voted.
- Second vote on same ballot → 400 named error.
- `shipped` card never in `arena_candidates`.
- Upsert with `shipped` stage on a card the CSV says `idea` → stays `shipped`.
- Anonymous `GET /roadmap` → 200; anonymous `POST /roadmap/arena` → 401.
- Move endpoint writes a changelog row; changelog lists it.
- Elo tie-break ordering stable.

### 45.6 Seed script

`scripts/seed_roadmap.py` (Python, stdlib only): reads `docs/requirements.csv`,
maps each row → card (§44.6 mapping), connects to the target DB (SQLite file or
Postgres URL from `--db-url`, default `sqlite://./data/lorehaven.sqlite` or
`DATABASE_URL`), upserts by normalized title. `--dry-run` prints the plan.
Never downgrades `shipped`. Idempotent: second run reports all `unchanged`.

### 45.7 Sign-off

Cross-cutting checklist (§16 of this plan) + requirements rows M45-01…M45-07
(§44.7 acceptance items) + `scripts/seed_roadmap.py --dry-run` output reviewed.
