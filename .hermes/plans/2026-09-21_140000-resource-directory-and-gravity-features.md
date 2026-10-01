# Resource Directory + Gravity Feature Adoption Plan

**Audience:** implementer with zero codebase context. Read the spec sections named in each brief, then follow tasks verbatim.

---

## Goal

Add a community-curated, ranked directory of fandom resources (fanfiction archives, Discord servers, author platforms, writing tools) to Lorehaven, then adopt the highest-value Gravity features that fit Lorehaven's fandom mission.

---

## Current Context / Assumptions

- Lorehaven is a single-instance self-hosted fanfiction platform (Rust/Axum + SvelteKit + PostgreSQL/SQLite).
- Route pattern: `crates/app/src/routes/<module>.rs` → `pub fn router()` → merged in `crates/app/src/server.rs`.
- Domain types: `crates/db/src/<module>.rs` (SQL), `crates/domain/src/<module>.rs` (pure rules).
- Migrations: `migrations/postgres/XXXX_name.sql` + `migrations/sqlite/XXXX_name.sql`, embedded by `build.rs`.
- Frontend routes: `frontend/src/routes/<Name>.velte`, registered in `frontend/src/lib/router.ts`, wired in `frontend/src/App.svelte`.
- API functions: `frontend/src/lib/api.ts`.
- Existing surfaces this plan extends:
  - Discovery/taste engine (`crates/app/src/routes/discovery.rs`, `crates/db/src/discovery.rs`) — M11, built.
  - Collections (`collection_kind` includes `series`, `anthology`, `reading_list`, etc.) — M13.
  - Typed votes (`forum_votes` with vote types) — M32, built.
  - Instance directory (federation `GET /federation/directory`) — M36, built but Lorehaven-instances only.

---

## Part 1 — Resource Directory

### 1.1 Architecture

A new `directory` module: `crates/app/src/routes/directory.rs`, `crates/db/src/directory.rs`. The directory stores **external resources** (not works). Entries are submitted by signed-in users, tagged with a category, and ranked by community votes.

Categories (seeded, admin-extensible via `lorehaven.toml [directory].extra_categories`):
- `fanfiction_archive` — multi-fandom archives (AO3, FFN, Wattpad…)
- `discord_server` — community Discord servers
- `author_platform` — author homepages, Carrd, Linktree
- `writing_tool` — Scrivener, Notion, Campfire, etc.
- `community` — subreddits, Tumblr tags, forums
- `lorehaven_instance` — other Lorehaven instances (syncs with federation directory)
- `other` — catch-all

Ranking: score = Σ vote values (up=+1, down=−1, `vote_value` TINYINT). Sort by score desc, then submission date desc. A separate `directory_entry_tags` table allows free-form tags per entry (language, fandom, topic).

Tables (migration 0046):

```sql
CREATE TABLE directory_entries (
    id               TEXT PRIMARY KEY,
    category         TEXT NOT NULL,
    title            TEXT NOT NULL,
    url              TEXT NOT NULL,
    description      TEXT NOT NULL DEFAULT '',
    submitted_by     TEXT NOT NULL,          -- account id
    approved_by      TEXT,                   -- null = pending review
    created_at       TEXT NOT NULL,
    updated_at       TEXT NOT NULL,
    score_generated  INTEGER NOT NULL DEFAULT 0  -- denormalized, refreshed on vote
);
CREATE INDEX idx_directory_entries_category ON directory_entries (category, score_generated DESC);
CREATE INDEX idx_directory_entries_submitted ON directory_entries (submitted_by, created_at DESC);

CREATE TABLE directory_entry_tags (
    entry_id    TEXT NOT NULL,
    tag         TEXT NOT NULL,
    PRIMARY KEY (entry_id, tag)
);
CREATE INDEX idx_directory_entry_tags_tag ON directory_entry_tags (tag);

CREATE TABLE directory_votes (
    entry_id    TEXT NOT NULL,
    account_id  TEXT NOT NULL,
    vote_value  INTEGER NOT NULL CHECK (vote_value IN (-1, 1)),
    voted_at    TEXT NOT NULL,
    PRIMARY KEY (entry_id, account_id)
);
CREATE INDEX idx_directory_votes_entry ON directory_votes (entry_id);
```

SQLite variant uses the same columns with `TEXT` timestamps.

### 1.2 Backend routes

File: `crates/app/src/routes/directory.rs`

```
GET  /directory                       list entries (category filter, sort, pagination)
GET  /directory/:id                   single entry detail
POST /directory                       submit new entry (signed-in)
POST  /directory/:id/vote             up/down vote (signed-in, idempotent toggle)
POST  /directory/:id/upvote           convenience wrapper
POST  /directory/:id/downvote         convenience wrapper
GET  /directory/categories            list categories + entry counts
POST  /directory/:id/approve          operator only
DELETE /directory/:id                 operator only
```

Handler `get_entries` query params: `category`, `tag`, `sort` (`top` | `new`), `limit` (default 50), `offset` (default 0), `q` (title/description search).

Vote logic: upsert on `(entry_id, account_id)`; if same `vote_value` again, delete (toggle off). After any change, recompute `score_generated` = `SUM(vote_value)` for that entry.

Entry listing excludes rows where `approved_by IS NULL` unless the viewer is the submitter or an operator.

### 1.3 DB module

File: `crates/db/src/directory.rs`

Functions:
- `submit_entry(db, id, category, title, url, description, submitted_by, now)`
- `list_entries(db, category, tag, sort, limit, offset, q, viewer, is_operator) → Vec<DirectoryEntry>`
- `get_entry(db, id) → Option<DirectoryEntry>`
- `set_vote(db, entry_id, account_id, vote_value, now) → Result<bool>` (returns true if vote active after op)
- `approve_entry(db, id, operator_id, now)`
- `delete_entry(db, id)`
- `list_categories(db) → Vec<CategoryCount>`
- `tags_for_entry(db, id) → Vec<String>`

Register in `crates/db/src/lib.rs`: `pub mod directory;`

### 1.4 Domain validation

File: `crates/domain/src/directory.rs`

- `validate_url(url: &str) -> Result<(), AppError>` — must be http(s), no localhost/internal IPs.
- `validate_category(cat: &str) -> bool` — matches known categories.
- `slugify_title(title: &str) -> String` — for URL-friendly entry IDs.

### 1.5 Config

Add to `crates/app/src/config.rs`:

```rust
pub struct DirectoryConfig {
    pub enabled: bool,
    pub require_approval: bool,
    pub extra_categories: Vec(String),
}
```

Default: `enabled: true`, `require_approval: true`, `extra_categories: []`.

### 1.6 Frontend

File: `frontend/src/routes/Directory.svelte`

- Category tabs (fanfiction_archive, discord_server, …).
- Entry cards: title (linked), description, tags, score, submitter, vote buttons.
- "Submit an entry" modal (signed-in).
- Search input.
- Pagination (infinite scroll or load-more).
- Link to `/directory/categories` for the full directory.

Register in `frontend/src/lib/router.ts`:
- Route id `'directory'`, match `/directory`.
- Route id `'directory-categories'`, match `/directory/categories`.
- Route id `'directory-detail'`, match `/directory/:id`.

Wire into `frontend/src/App.svelte` with `{:else if route.id === 'directory'}` etc.

API functions in `frontend/src/lib/api.ts`:
- `listDirectoryEntries(opts)`
- `getDirectoryEntry(id)`
- `submitDirectoryEntry(data)`
- `voteDirectoryEntry(id, value)`
- `listDirectoryCategories()`

---

## Part 2 — Gravity Feature Adoption

Priority order. Each item lists the spec section to extend, the repo files to touch, and the migration number.

### 2.1 Fork-with-provenance (G §11.9) — **Priority 1**

Remix is native to fandom. A one-button fork creates a derivative work linked to its parent with visible provenance chain.

- Migration 0046 (same file, add column): `works.parent_work_id TEXT, works.fork_depth INTEGER NOT NULL DEFAULT 0`.
- Domain rule: `crates/domain/src/works.rs::fork_work(source, forker) → Result<Work>` — validates forker ≠ original owner, enforces `config.works.max_fork_depth` (default 3), inherits tags/characters but copies no body.
- Route: `POST /works/:id/fork` → `crates/app/src/routes/works.rs::fork_work_handler`.
- Frontend: "Fork this work" button on `WorkPage.svelte`, visible when signed-in and work permits derivatives (check consent directive if present).
- Existing `derivatives` table (migration 0030) is reused for the provenance chain.

### 2.2 Consent architecture + creator content directives (G §5.24) — **Priority 1**

Per-work machine-readable interaction directives: may-podfic, may-translate, may-remix, interaction boundaries.

- Migration 0046 (add table):
```sql
CREATE TABLE work_consent_directives (
    work_id     TEXT PRIMARY KEY,
    may_podfic  INTEGER NOT NULL DEFAULT 1,     -- 0=no, 1=yes, 2=ask
    may_translate INTEGER NOT NULL DEFAULT 1,
    may_remix   INTEGER NOT NULL DEFAULT 1,
    may_recommend INTEGER NOT NULL DEFAULT 1,
    updated_at  TEXT NOT NULL
);
```
- Domain: `crates/domain/src/consent.rs::Directives` struct with `permits(action: ConsentAction) -> TriState`.
- UI: expand existing `WorkPage.svelte` "Edit" → "Sharing" tab with toggles. Add to work creation flow (2 additional radio groups).
- Enforcement: `fork_work` checks `may_remix`; translation pipeline checks `may_translate`; recommendation engine checks `may_recommend`.

### 2.3 Content half-life as curation signal (G §11.1) — **Priority 2**

A work still being read/discounted months after publication. Inputs the quality ranking.

- New column on `works`: `half_life_score REAL` (default null).
- Background job: `crates/app/src/jobs/half_life.rs` — queries `reading_events` for works published >30 days ago, computes decayed engagement rate, writes `half_life_score`.
- Domain function: `crates/domain/src/discovery.rs::half_life_boost(score, half_life) -> f64`.
- Discovery engine: incorporate `half_life_score` into `blend()` weighting.
- Schedule: run nightly via existing `jobs` infrastructure.

### 2.4 Interaction tiers & ambient social suite (G §10.5) — **Priority 2**

Implicit warmth tiers (lurk < react < comment < create) for shy readers.

- New table `interaction_tiers` (migration 0046):
```sql
CREATE TABLE interaction_tiers (
    account_id  TEXT NOT NULL,
    target_account_id TEXT NOT NULL,
    warmth      REAL NOT NULL DEFAULT 0,   -- accumulated signal
    tier        TEXT NOT NULL DEFAULT 'lurk', -- lurk|react|comment|create
    updated_at  TEXT NOT NULL,
    PRIMARY KEY (account_id, target_account_id)
);
```
- Domain: `crates/domain/src/interaction.rs::record_event(actor, target, action)`.
- Routes: none private — warmth is ambient. Only author-facing aggregate (percentile tier) on `PeoplePage.svelte`.
- Event hooks: in `post_comment`, `post_reaction`, `record_reading_progress`, call `record_event`.

### 2.5 Typed votes upgrade (G §3) — **Priority 3**

Extend `forum_votes` (M32) to carry semantic signal, not just magnitude.

- Migration 0046 (alter): `ALTER TABLE forum_votes ADD COLUMN signal_type TEXT DEFAULT 'upvote'`; valid values: `upvote`, `downvote`, `well_written`, `emotionally_impactful`, `research_rich`, `hard_hitting`.
- Domain: `crates/domain/src/votes.rs::SignalType` enum with `to_weight() -> f64`.
- UI: `ForumPost.svelte` — replace single upvote with a small picker (default upvote, expand for typed signals).
- Forum post score becomes Σ `signal_type.weight` instead of ±1.

### 2.6 Community tag governance (G §4.6) — **Priority 3**

Scale AO3-style tag wrangling with voting and gardening.

- Migration 0046 (add table):
```sql
CREATE TABLE tag_proposals (
    id          TEXT PRIMARY KEY,
    tag         TEXT NOT NULL,
    proposal    TEXT NOT NULL,          -- merge|split|alias|deprecate
    target_tag  TEXT,
    rationale   TEXT NOT NULL,
    proposed_by TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'open',  -- open|accepted|rejected|implemented
    created_at  TEXT NOT NULL
);
CREATE TABLE tag_proposal_votes (
    proposal_id TEXT NOT NULL,
    account_id  TEXT NOT NULL,
    vote        TEXT NOT NULL,          -- aye|nay
    voted_at    TEXT NOT NULL,
    PRIMARY KEY (proposal_id, account_id)
);
```
- Routes: `crates/app/src/routes/tags.rs::propose_tag_change`, `vote_on_proposal`, `list_proposals`, `implement_proposal` (operator).
- Domain: `crates/domain/src/tags.rs::governance` module with quorum rules.
- UI: `Tags.svelte` → "Propose change" button; `TagDetail.svelte` → proposal list + voting.

---

## Part 3 — Spec + Plan Updates

### 3.1 Spec updates

File: `docs/spec.md`

- Insert new §15.16 **Resource Directory** after §15.15 (Length histogram):
  - `/people`-adjacent, external resources, categories, voting, operator moderation.
- Extend §11 (Works) with §11.9a **Fork with provenance** (fork button, depth limit, provenance chain).
- Extend §12 with §12.x **Consent directives** (per-work interaction policy).
- Extend §16 with §16.xx **Content half-life** (decayed engagement signal, feeds).
- Extend §17 with §17.xx **Interaction tiers** (ambient warmth, aggregate-only author signal).
- Extend §15 with §15.xx **Tag governance** (proposals, quorum, gardening).
- Extend §3 with §3.x **Typed vote signals** (semantic vote types, weights).

### 3.2 Requirements CSV

File: `docs/requirements.csv`

Mark new rows (one per feature):
```
M38-01,directory,Resource directory: ranked community-curated external resources,M38,planned,...
M38-02,works,Fork with provenance,M38,planned,...
M38-03,rights,Consent directives per work,M38,planned,...
M39-01,discovery,Content half-life curation signal,M39,planned,...
M39-02,social,Interaction tiers (ambient warmth),M39,planned,...
M40-01,governance,Tag governance (proposals + quorum),M40,planned,...
M40-02,votes,Typed vote signals,M40,planned,...
```

Update `M11-01` (Discovery) to note the new half-life extension.

### 3.3 Junior implementation plan

File: `docs/plans/junior-implementation-plan.md`

Add after the M30 row:

```
| M38 | §15.16 + §11.9a + consent | Resource directory, fork-with-provenance, consent directives | M36, M11 |
| M39 | §16.xx + §17.xx | Content half-life, interaction tiers | M11, M12 |
| M40 | §3.x + §15.xx | Typed vote signals, tag governance | M32, M10 |
```

Add milestone briefs:
- **M38 brief** — resource directory (§15.16), fork (§11.9a), consent (§12.x).
- **M39 brief** — half-life scoring (§16.xx), interaction tiers (§17.xx).
- **M40 brief** — typed votes (§3.x), tag governance (§15.xx).

---

## Part 4 — Step-by-Step Tasks (TDD per task)

Each task: write failing test → run, see fail → implement → run, see pass → commit.

### Task 1: Domain URL validation

Test file: `crates/domain/src/directory.rs::tests::rejects_internal_urls`

```rust
#[test]
fn rejects_internal_urls() {
    assert!(validate_url("https://example.com").is_ok());
    assert!(validate_url("http://localhost").is_err());
    assert!(validate_url("http://127.0.0.1").is_err());
    assert!(validate_url("http://[::1]").is_err());
    assert!(validate_url("ftp://example.com").is_err());
}
```

### Task 2: Migration 0046 + DB layer

Create `migrations/postgres/0046_resource_directory.sql` + SQLite variant. Apply to production. Write `crates/db/src/database.rs` functions. Test with `cargo test -p lorehaven-db --lib` (existing test harness spins up SQLite in-memory).

### Task 3: Route handlers + tests

Test file: `crates/app/tests/milestone_38.rs`:

```rust
#[tokio::test]
async fn an_entry_can_be_submitted_listed_and_voted_on() { ... }
#[tokio::test]
async fn unapproved_entries_are_hidden_from_non_operators() { ... }
#[tokio::test]
async fn voting_twice_toggles_off() { ... }
```

Implement `crates/app/src/routes/directory.rs`. Wire into `crates/app/src/server.rs`.

### Task 4: Frontend directory page

`frontend/src/routes/Directory.svelte`, API functions, router entries, `App.svelte` branches.

### Task 5: Fork + consent

Migration columns + `crates/domain/src/works.rs::fork_work`, route `POST /works/:id/fork`, UI button. Tests in `milestone_38.rs`.

### Task 6: Half-life

Job `crates/app/src/jobs/half_life.rs`, domain `half_life_boost`, integrate into `blend()`. Tests in `milestone_39.rs`.

### Task 7: Interaction tiers

Domain `record_event`, hooks in `post_comment`/`post_reaction`, UI warmth indicator. Tests in `milestone_39.rs`.

### Task 8: Typed votes + tag governance

Migration `signal_type` column, domain `SignalType`, UI picker, tag proposal routes + UI. Tests in `milestone_40.rs`.

### Task 9: Spec + plan

Update `docs/spec.md`, `docs/requirements.csv`, `docs/plans/junior-implementation-plan.md` per Part 3.

---

## Part 5 — Tests / Validation

Per task above. Global gates:

- `cargo test --workspace` — all unit + doc tests pass.
- `cargo check` — no warnings introduced by this work.
- `just check` — CI gate passes.
- `cd frontend && npm run build` — frontend compiles.

---

## Part 6 — Risks, Tradeoffs, Open Questions

### Risks
- Directory invites spam if `require_approval` is misconfigured → default to true, audit log.
- Fork chains could proliferate low-effort copies → depth limit + consent directive gate.
- Interaction tiers risk gamification → aggregate-only author display, never per-reader exposure.

### Tradeoffs
- Half-life job adds nightly compute → gate behind `config.discovery.enable_half_life` (default false until M39).
- Tag governance is high-effort for limited return → defer to M40, after votes stabilize.

### Open Questions
- Should directory entries support **multiple categories** (e.g., AO3 is both `fanfiction_archive` and `community`)? Current schema: single category + free tags. Revisit if multi-category proves needed.
- Should consent directives be **per-pseud or per-work**? Current: per-work. Per-pseud is simpler but less precise.
- Should interaction tiers be **visible to the holder** (my warmth toward X)? Current: author-facing aggregate only. Revisit after user research.

---

## Part 7 — Milestone Map

| Milestone | Features | Depends |
|---|---|---|
| M38 | Resource directory, fork-with-provenance, consent directives | M36, M11 |
| M39 | Content half-life, interaction tiers | M11, M12 |
| M40 | Typed vote signals, tag governance | M32, M10 |

Commit convention: one commit per task (1–9). Tag `v0.38-directory` after M38 lands.

---

*Plan saved. Ready to execute via subagent or directly.*
