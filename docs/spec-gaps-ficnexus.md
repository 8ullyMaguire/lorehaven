# FicNexus features absent from the Lorehaven plan

Status: **resolved by ADR 0024 (2026-09-24).** The gap analysis below was
produced before the decision; it is kept as the audit trail. Resolution
summary: the A-items are adopted (most were already absorbed into `spec.md`
during the M6–M26 build — credential vault §11.6, source health §11.8, body
retention §11.15, ratings/reviews §9.5, view counts §9.8, query language
§15.5, the rec strategy registry via the new §16.1a, the bot port via the
amended §23.2); the adapter porting backlog is the new §11.16 with plan
milestones M53+ in `docs/plans/remaining-work.md`; the C-items keep the
plan's stance (C3: reputation stays out of the trust axes). Nothing from
this document remains silently unaddressed.

## How this was produced

The comparison is between the plan in `docs/spec.md` and the older FicNexus
repository (`~/code/rust/ficnexus`, a separate codebase sharing no code with
this one). Read on the FicNexus side:

- `docs/src/features.md`, `what-is-fichub.md`, `downloading.md`, `searching.md`,
  `tag-search.md`, `bookmarks.md`, `ratings.md`, `comments.md`, `forum.md`,
  `recommendations.md`, `rec-engines.md`, `translations.md`, `profile.md`,
  `people.md`, `integrations.md`, `transparency.md`, `anti-bot.md`,
  `self-healing.md`, `faq.md`, `quickstart.md`, `epub-codebase.md`
- `specs/001-forum-depth-phase2/spec.md`, `specs/002-otwarchive-parity/spec.md`,
  `specs/003-otw-roadmap-2013/spec.md`, `specs/004-site-stats/spec.md`,
  `specs/forum-nodebb/spec.md`

Each item below cites where the FicNexus behaviour is described. Absence from
`docs/spec.md` was checked by keyword search over the whole file; a few items
("ratings", "history", "i18n") are named in the spec only in a different sense,
and those are called out where they occur.

---

## A. Missing from the plan

Ordered by fit with the plan's own three stated priorities (§ front matter):
importing and private libraries, writing and publishing, reading and offline.

### A1. Importing and the private library

- **Per-source credential vault.** FicNexus stores credentials for
  login-requiring sources: opt-in consent, encrypted at rest (AES-256-GCM),
  30-day auto-expiry, never returned by any endpoint, decrypted in memory only
  at download time (`docs/src/features.md:39-44`). The plan says only "redact
  source credentials" (§11) — it assumes credentials exist without specifying
  how they are held, so every login-walled source is unimplementable as written.
  *Touch:* new §11 subsection, a vault table in §4.4, a privacy classification
  in the §24 endpoint contract.
- **Source health and support transparency.** FicNexus tracks scrape failures
  per domain with health and retry (`docs/src/features.md:190`,
  `docs/src/self-healing.md`). The plan has `adapter_versions.verification_status`
  (§4.4) — a static label set by a human — but no runtime health, no degradation
  path, and no way to tell a reader that a source is failing today.
  *Touch:* §11, surfacing in §22.
- **Shared body cache.** FicNexus persists every scraped body on attached
  storage so repeat exports are instant and the instance accretes an archive
  (`docs/src/features.md:36-38`). M5's content-addressed storage covers
  user-owned content; nothing describes a shared corpus keyed by source
  revision. This is the enabler for cheap update checks, cross-user dedup, and
  body search (A3), so it must be decided before M6, not after.
  *Touch:* §10 storage, §4.4, §11 update policy.
- **Cross-source identity and community merge proposals.** FicNexus shows the
  same story from different sites as one page and lets the community propose
  merges (`docs/src/features.md:170-172`). The plan has `work_relations` (§4.3)
  and canonicalization for *tags* (§14) but never states that a work imported
  from two sources is one entity, and has no merge queue. Without it every
  import is a duplicate.
  *Touch:* §4.3/§4.4, §14 canonicalization, §18 proposal quorum.
- **Author/bibliography batch import.** FicNexus accepts an AO3 user URL or a
  XenForo member URL and pulls the whole bibliography with live progress
  (`docs/src/downloading.md:51-52`). The plan's M6 handles one URL or one file
  per import. This is the highest-leverage import feature and it is absent.
  *Touch:* §11, §10 (job model already fits), §17 (requests) not applicable.
- **Open Doors-style archive import batches.** FicNexus scopes batched,
  idempotent at-risk-archive import with original attribution and an import
  collection link (`specs/003-otw-roadmap-2013/spec.md:182`). Nothing in the
  plan. Strong fit for preservation-minded self-hosting.
- **Bookmarklet and paste-a-URL entry points.** FicNexus ships a drag-to-bar
  bookmarklet and a paste box on the home page (`docs/src/features.md:27`,
  `:95`). The plan describes the import workflow abstractly (§11) with no entry
  points.

### A2. Exports and offline

- **AZW3 and Markdown export.** The plan orders text, HTML, EPUB, PDF, MOBI
  (§12); FicNexus also produces AZW3 and Markdown (`docs/src/features.md:25`).
- **Send-to-Kindle.** FicNexus emails an EPUB to a Kindle address
  (`docs/src/features.md:32`). In the plan, email appears only as optional
  notification infrastructure (§2.2); delivering an export by email is not a
  feature anywhere.
- OPDS is already covered by §21 — no gap.

### A3. Reading

- **Ratings and written reviews.** The plan's end-of-work page lists
  appreciation, bookmark and comment, but no rating (§9), while §15 cites
  "private ratings" as a discovery signal. The signal is specified and the
  feature that produces it is not. FicNexus has 1–5 stars plus review text
  feeding recommendations (`docs/src/features.md:102-103`). *Touch:* §9, §15.
- **Reading history and reading analytics.** M8 has `reading_progress`
  (position sync) but no history page and no totals. FicNexus ships words-read,
  works-read, login streak and a recently-read list
  (`specs/004-site-stats/spec.md:49-72`).
- **Reading-time estimate**, dialogue-density aware (`docs/src/features.md:161`).
- **Hit/view counts.** Neither the concept nor the vocabulary appears in the
  plan at all. AO3 parity and FicNexus both count views
  (`specs/004-site-stats/spec.md:120`). Deserves a deliberate yes or no.

### A4. Search and discovery

- **A user-facing query syntax.** §14 defines a typed JSON AST and a filter
  list but no text grammar for a search box. FicNexus documents `AND`/`OR`/`NOT`,
  quoted phrases, `-exclusion`, parentheses, and
  `title:`/`author:`/`fandom:`/`character:` fielded search
  (`docs/src/features.md:58-63`). Without it the AST is API-only: readers get
  filters and no query language.
- **Search inside work bodies.** FicNexus quotes prose and dialogue across
  every cached body with highlighted snippets (`docs/src/features.md:76-79`).
  The plan's `Text` node (§14) is metadata-scoped; there is no body index.
  Depends on A1's shared body cache.
- **Zero-result search demand mining.** Clustering the queries that returned
  nothing so curators can see what readers want and the archive lacks
  (`docs/src/features.md:282-284`). Unusually well matched to an archive that
  grows by importing.
- **Author/people directory.** FicNexus browses users A–Z and filters by fandom
  (`docs/src/people.md:8-13`). The plan has only `/u/:handle`.
- **Saved searches as pinnable named views.** The plan has a `saved_searches`
  table (§4.6) but never a feature for it; FicNexus pins saved views
  (`docs/src/features.md:221-222`).

### A5. Community and forum (M11)

- **Read state and unread badges per topic.** An entire FicNexus spec covers
  this: per-user last-read position, unread counts, advancing on open, bounded
  idempotent writes (`specs/001-forum-depth-phase2/spec.md:79-91`). The plan
  mentions "read/unread" once, as a *search filter* (§14) — not forum behaviour.
- **Forum full-text search with snippets** (`specs/forum-nodebb/spec.md:207`).
- **@mentions, email digests, notification preferences**
  (`specs/forum-nodebb/spec.md`, FR-030/031).
- **Topic tags**, freeform and admin-mergeable, with tag pages
  (`specs/forum-nodebb/spec.md:206`). The plan's forum list (§16) has
  categories, boards, topics, posts, polls, reactions, pins, locks,
  subscriptions and pagination — no topic tags.
- **Real-time transport with catch-up.** The plan allows WebSocket or SSE and
  calls live transport "an optimization" (§16). FicNexus specifies Redis pubsub
  per topic and category, a single-process fallback when Redis is down, an SSE
  read-only fallback, and `?after={lastPostId}` catch-up
  (`specs/forum-nodebb/spec.md`, FR-027…029). Finer-grained; worth copying.
- **Category-scoped timeouts and bans with expiry**, and per-category watch
  levels (`specs/forum-nodebb/spec.md`, FR-011/FR-018). §18 has sanctions; the
  scoping is absent.
- **Earned reputation, distinct from trust.** See C3 — this one needs a
  decision, not a copy.

### A6. Discovery and personalisation

- **Community "similar work" suggestions** with up/down voting, separate from
  engine-generated similarity (`docs/src/features.md:147-151`).
- **Community judgement of moderation fairness** — anonymised verdicts on
  moderator actions (`docs/src/features.md:139-140`). §18 has appeals to
  independent reviewers, which is a different mechanism.
- **Widget-composed dashboard** with a widget registry and a default layout
  factory (`docs/src/features.md:92-93`).
- **Recipe builder** — user-composed recommendation blends (weights, filters,
  boosts) published as installable objects (`docs/src/features.md:208-212`).
  §20 lists "recommendation engines" as an extension category and §15 has
  tunable weights, but user-authored blends are not specified.
- **Gallery mechanics**: install dedup, per-extension star rating, remix/fork
  (`docs/src/features.md:250-254`). §20 specifies a paid marketplace and a
  review workflow instead.

### A7. Interface

- **Command palette** (`Ctrl+K`) — absent (`docs/src/features.md:7`).
- **Contextual help**: `?` links that open the exact docs section, an in-app
  docs modal, "try it" deep links that prefill an action
  (`docs/src/features.md:10-20`). §25 plans a tutorial *document*, which is a
  different thing from in-app help.
- **Interface localisation.** The plan mentions language only as a work
  attribute and as AI translation; localised *chrome* is never a requirement.
  FicNexus runs six locales with translatable header, nav, footer and flash
  messages (`specs/003-otw-roadmap-2013/spec.md:105`, FR-009). For AGPL
  self-hosted software this is a conspicuous omission.
- **Archive-look versus modern UI mode** (`docs/src/epub-codebase.md:173-175`).
  The plan has themes but no per-surface look mode.

### A8. Operations and administration

- **Public site statistics page.** An entire FicNexus spec covers `/stats`:
  per-period views, kudos, bookmarks, works-read, plus total works and active
  users, motivated by anonymous visitors leaving when they cannot judge whether
  the archive is worth joining (`specs/004-site-stats/spec.md:18-27`). §22 is
  admin-only operations health; nothing public.
- **Non-PII usage analytics**: unique daily/weekly/monthly visitors, active
  versus view-only, action timeline (`docs/src/features.md:52-54`). §22 rightly
  forbids exposing reading histories; aggregate visitor counts are not that.
- **Bot/abuse dashboard**: top addresses by requests per hour, export:request
  ratio, failed-auth bursts (`docs/src/anti-bot.md:40-66`).
- **(ip, client_id) rate-limit keys.** FicNexus keys the limiter on address
  *and* a client id specifically so shared NAT and campus users are not
  punished (`docs/src/anti-bot.md:117`). The plan's M2-09 is address-and-account
  keyed, which for anonymous traffic collapses to address alone: one misbehaving
  client rate-limits everyone behind the same NAT. Worth an explicit rule even
  if the layered anti-bot work is deferred.
- **Layered anti-bot beyond rate limiting** — proof-of-work, JS challenge,
  honeypots, form timers, headless-browser detection
  (`docs/src/anti-bot.md:72-99`), with the accompanying "do not do this"
  list (`:116-121`).
- **Metadata correction tool** and an **auto-tag suggestion queue**. §14 has
  `tag_proposals` and `tag_votes` with quorum, but no suggestion pipeline and no
  way for a curator to correct a work's title, author, status or description
  (`docs/src/features.md:267-270`).
- **Translation review queue.** §21 offers AI translation with no review
  workflow; FicNexus has curator approve/reject/edit
  (`docs/src/features.md:269`).

### A9. Integrations

- **Chat bot as a thin REST client** (Discord/Telegram/Matrix), with a linking
  flow that exchanges credentials once and stores only a token
  (`docs/src/integrations.md:33`, `docs/src/features.md:292-305`). §21 covers
  ActivityPub and feeds; §24's route-ownership table has no bot row.
- **A documented public developer API**: `/api/v1`, keys, scopes, pagination,
  rate limits, OpenAPI, versioning
  (`specs/003-otw-roadmap-2013/spec.md:183`). §24 requires an internal endpoint
  inventory with per-endpoint auth, limits and privacy classification — thorough
  — but "public API for third-party tools" is never a deliverable, and the chat
  bot above needs exactly that.

---

## B. Already covered — do not add

Checked and genuinely present in the plan, listed so nobody re-adds them:

| FicNexus feature | Where the plan already has it |
|---|---|
| Blind Date | §15 baseline engines; route `/blind-date` §24 |
| RSS/Atom feeds | §21, with scoped revocable private feeds |
| OPDS | §21 |
| PWA offline reading | §12, including IndexedDB and offline privacy |
| Threshold/lock on secret works | §8 visibility states plus §2 age policy |
| Blocks and mutes | §4.2 tables; enforcement requirements §16 |
| Kudos-style appreciation | §16 "appreciation" (unnamed but specified) |
| Queues with priority, no starvation | §19 weighted queues with ageing |
| Pseuds, co-creators, series, collections, challenges | §4.2/§4.3, §17 |
| Tag canonicalisation, synonyms, merges with redirects | §14 workflow, §4.5 constraints |
| Site and work skins (sanitised CSS) | §20 themes |
| Audit log of moderator actions | §18 public modlog, `audit_events` §4.6 |
| Account/pseud/work deletion and export | §22 |
| Search filters by rating, warning, language, completion, word count | §14 filter list |
| Content-safety "spice" sorting | §2 age policy plus §14 taxonomy |

---

## C. Deliberate divergences — port with care

### C1. XP, levels, ranks, Ability Tree, rank-gated features

FicNexus grants XP for reading and downloading, runs 100 levels and 10 ranks,
and gates widgets, the recipe builder, the theme editor and custom views behind
rank (`docs/src/features.md:195-204`). The plan replaces this with TL0–TL6 where
"higher levels require reviewed conduct, not merely point totals" (§18) and
trust "cannot be purchased" (§19). **Do not port.** A leaderboard is only
compatible if it is informational and gates nothing.

### C2. "107+ supported sites" as a shipped claim

`docs/src/features.md:28`. §11 explicitly forbids promising a count in advance
and requires per-adapter verification status with unsupported adapters excluded
from support counts. **Keep the plan's stance**; the adapter count is an outcome,
not a specification item.

### C3. Reputation as an earned, visible score

FicNexus awards reputation for creating topics and posts, caps it daily, and
surfaces it on a leaderboard (`specs/forum-nodebb/spec.md`, FR-038…041). The
plan separates account reliability (TL0–TL6), scoped expertise, and appointed
staff roles (§18) — reputation is *not* one of the three axes. Awarding a
visible score for volume of posting is in tension with "reviewed conduct, not
merely point totals". **Open decision:** either keep reputation strictly
internal and non-display, or specify it explicitly as a forum-local signal that
never affects trust, moderation weight, or access.

### C4. Shared fetcher versus per-source adapters

FicNexus relies on source credential storage to reach login-walled sites (A1).
§11's security rules require the shared fetcher to reject private-network
targets "unless explicitly permitted for a trusted administrative integration".
If the credential vault is adopted, the two must be reconciled in one place: a
credentialed fetch is exactly the case where SSRF rules need a written exception
and an audit trail.

---

## D. Suggested next step

`docs/spec.md` §4.4 (private import tables), §10 (jobs and storage), §11
(imports), §12 (exports), §15 (discovery) and §22 (administration) are the
sections any of A1–A9 would change. Of those, A1's five items (credential vault,
source health, shared body cache, cross-source identity, author batch import)
change the import tables, so they are the ones to settle **before** Milestone 5
is built rather than after; the rest can be folded in as their milestones
arrive.

Deciding A1 changes the §4.4 schema; deciding C3 changes the §18 trust model.
Everything else is additive.
