# Lorehaven Spec Summary

An ~8k-word distillation of the Lorehaven implementation specification (64.5k words) and taste-gravitational amendment (6.6k words), designed for brainstorming. Quoted section numbers (`§0.3`, `§16.17`, etc.) cross-reference the canonical docs: `docs/spec.md` and `docs/spec-amendments/taste-gravitational-system.md`. Structure follows the spec's own order: premise, architecture, milestones (the build order), the taste-gravitational amendment, then later roadmap sections.

---

## 1. Premise and Priority Stack (spec §0)

Lorehaven is a **self-hosted, self-governing fanfiction platform** that lets users shape their own experience through extensions, grows the body of available fiction through frictionless importing and writing, keeps authors motivated through a positivity-first feedback culture, and uses trust-level and quorum-based community governance so the administrator can focus on curating taste rather than policing behavior.

The **priority stack** (lower number wins when goals conflict):

1. Fully customizable website (themes, engines, feeds, widgets, tools, challenges, reader enhancements)
2. Maximize available high-quality fiction (aggressive importing, low-friction writing, discovery, quality signals)
3. Maximize positive feedback and suppress destructive negativity (constructive critique requires opt-in)
4. Grow admin-aligned fiction without becoming monothematic (private taste influence + diversity mechanisms)
5. Trust-level self-governance with minimal administrator involvement
6. Quorum-based moderation and curation
7. Translate everything (interface fully translated; content on demand)
8. Revenue through credits, subscriptions, marketplace fees

**Foundational protections (§0.3)** — these are *not* priorities that yield to others; they constrain every priority above:

- Private libraries, drafts, reading history, source credentials, pseud linkage stay private
- Authors control their work; readers control their data
- Core reading, writing, publishing, basic community participation remain free
- Bounded resource use at every trust and payment level
- Accessibility, child safety, harassment protection are foundational
- **No purchased trust, purchased ranking, or purchased moderation authority**
- Credits and gamification must never purchase trust, moderation authority, or search ranking
- No advertising, no third-party trackers, no sponsored placement
- Third-party AI crawlers refused by default
- Administrator's taste profile must never be visible, inferable, or hinted at
- Honest verification claims — feature presence in this document is not evidence of implementation

**Instance Topics (§0.4)** let an operator declare public thematic focuses (e.g., "we love slow-burn romance"). Topics have gamified bonuses (extra credits for topic-aligned reading) and leaderboards, but are always optional and disclosed.

---

## 2. Stack and Architecture (spec §2)

| Area | Choice |
|---|---|
| Backend | Rust |
| HTTP | Axum |
| Async | Tokio |
| Serialization | Serde |
| Database access | SQLx |
| Frontend | Svelte + TypeScript + Vite |
| Editor | Tiptap with restricted document schema |
| Styling | CSS variables, scoped component CSS, design tokens |
| SQLite search | FTS5 with trigram extensions |
| PostgreSQL search | Native full-text search + `pg_trgm` |
| Browser testing | Playwright |
| Frontend unit tests | Vitest |
| WASM execution | `wasmi` initially |
| Reverse proxy | Caddy |
| Service management | systemd |
| Default storage | Local filesystem |
| Optional billing | Operator-chosen payment processor adapter |
| Optional AI | Pluggable adapter interface including Ollama |
| Optional caching | Redis, with in-process fallback |

**Why Rust:** native compilation, memory efficiency, expressive type system, ARM64 support. **Why Svelte/TS:** concise interactive frontend without Node.js in production. **Why modular monolith:** simplified local development, transactions, deployment, debugging. **Single instance per deployment** — multi-tenancy is explicitly excluded.

**Runtime components (§2.2):** Browser/PWA/API client → HTTPS → Caddy → Lorehaven executable (HTTP/API, embedded frontend, extension host, job scheduler, import/export/translation workers, search indexing, positivity filter, notifications, federation workers) → SQLite or PostgreSQL → local file storage. Optional: SMTP, browser push, payment processor, document converters, AI providers, ActivityPub, chat platforms, Redis.

**Payment processor compatibility (§2.2):** The operator chooses the processor and owns its content-policy compatibility. Mainstream processors restrict adult content; the operator must select one matching the instance. This is an operator compliance concern, not a platform feature.

---

## 3. Shared Engineering Conventions (spec §3)

**IDs and timestamps (§3.1):** UUIDs for primary identifiers, stored natively (PostgreSQL) or consistently encoded (SQLite). UTC internally, RFC 3339 in APIs, locale-formatted for display. Do not expose sequential IDs or ownership through public URLs.

**Money, credits, counts (§3.2):** Money and credits use integer minor units. Word counts are nonnegative integers. Never use floating-point for balances. Statistical estimates and recommendation scores may use floating-point with documented interpretation.

**API conventions (§3.3):** Endpoints prefixed `/api/v1`. Collections use `{items[], next_cursor}`. Errors use `{error: {code, message, field_errors, request_id}}`. Canonical error codes: AUTH_REQUIRED, ACCESS_DENIED, NOT_FOUND, VALIDATION_FAILED, REVISION_CONFLICT, RATE_LIMITED, QUOTA_EXCEEDED, CONTENT_RESTRICTED, SOURCE_UNSUPPORTED/UNAVAILABLE/AUTH_REQUIRED/CREDENTIAL_EXPIRED, IMPORT_REVIEW_REQUIRED, QUERY_INVALID/TOO_COMPLEX, JOB_FAILED, CONVERTER_UNAVAILABLE, INSUFFICIENT_CREDITS, EXTENSION_PERMISSION_DENIED/QUOTA_EXCEEDED, COMMENT_HELD_FOR_REVIEW, TRANSLATION_UNAVAILABLE/PERMISSION_REQUIRED, QUORUM_INSUFFICIENT, DMCA_TAKEDOWN_PENDING. Use 404 rather than revealing inaccessible private objects.

**Concurrency (§3.4):** Every editable resource has a monotonically increasing `version`. Updates include `expected_version`. Mismatch returns 409 REVISION_CONFLICT.

**Authentication (§3.5):** Opaque server-managed sessions in secure cookies (HttpOnly, Secure in production, SameSite, narrow path/domain). CSRF protection for state-changing requests. Scoped API tokens with hashed storage.

**Authorization (§3.6):** authenticate → resolve pseud → load resource → evaluate policy → perform → audit. Policy lives in policy functions, not scattered frontend checks.

**Privacy classifications (§3.7):** Public, Unlisted, Restricted community, Pseud-private, Account-security, Source-secret, Staff-confidential, Aggregate-public, Aggregate-internal. Preserved in cache keys, search indexes, analytics, notifications, background jobs.

**Rate limiting (§3.8):** Layered limits: account/token, anonymous client identifier, IP/network ceiling, endpoint class, source domain, concurrent jobs, expensive-operation budget.

---

## 4. Database Plan (spec §4)

Mutable tables carry `id`, `created_at`, `updated_at`, `version`. Index foreign keys, owner+time pairs, status+time for jobs, normalized handles, canonical source identifiers, composite filter keys, search scope+revision. Use JSON for flexible documents only, not as substitute for searchable relationships.

**Identity tables (§4.2):** accounts, password_credentials, sessions, recovery_tokens, second_factors, pseuds, public_pseud_links, privacy_settings, age_assessments, guardian_authorizations, blocks, mutes, api_tokens, integration_authorizations, invitations, registration_applications. Mute scopes: hide from feed, hide from search results, hide comments authored by, combinations.

**Content tables (§4.3):** works, work_contributors, chapters, chapter_revisions, publication_events, series/series_entries, work_relations, media_assets, collaboration_invites, story_identities, identity_merge_proposals/history, work_feedback_preferences, dmca_notices/counter_notices.

**Import/storage tables (§4.4):** sources, adapter_versions, source_health_windows/incidents, external_records, library_items, import_snapshots/chapters/jobs/attempts, provenance_records, source_credentials/consents, import_batches/entries, preservation_batches, content_blobs/references, source_revision_cache_entries, author_watches, cross_post_targets.

**Taxonomy tables (§4.5):** canonical_entities, entity_aliases/relations, work_characters/attributes/roles, relationships/participants, work_relationships/dynamics, work_tags, tag_proposals/votes/merge_history, metadata_completeness/suggestions, mood_tags, work_moods, rating_check_flags.

**Other modules (§4.6):** Library (bookmarks, notes, shelves, reading_progress/events/aggregates, saved_searches, reading_goals), Reader feedback (ratings, reviews, work_metric_aggregates, quick_reactions, appreciation_notes, cheer_events), Positivity (comment_classifications, feedback_holds, moderation_queue, classifier_training_signals), Jobs, Search, Community (comments, reactions, follows, groups, boards, topics, posts, polls), Forum depth, Messaging (conversations, messages, chat_rooms, presence), Collections, Writing events (challenges, prompts, signups, claims, fulfillments, mentorships, sprints, wishlists), Governance (trust_policies/history, expertise, role_assignments, reports, cases, proposals, votes, sanctions, appeals, audit_events, quorum_records, shadowban_actions), Economy (wallets, ledger_transactions/entries, credit_holds, subscriptions, payment_events, bounties, entitlements, work_pricing, author_earnings, payouts, monetization_assertions, pool_b_distributions, work_ai_declarations), Extensions (packages, versions, manifests, installations, grants, reviews, approvals, execution_usage, webhook_subscriptions), Discovery (user_preferences, taste_profiles/versions, permitted_signals, exposure_events, aggregate_affinities, similarity_suggestions/votes, recommendation_recipes/versions, diversity_budgets, editorial_picks, browse_sort_preferences, taste_sources, demand_weights/history, demand_signal_events, demand_diversity_state), Interface (dashboard_layouts, widget_instances, locale_preferences, navigation_customizations), Integrations (notifications, push_subscriptions, feed_tokens, federation_actors/deliveries, bot_links, sitemap_state), Translation (requests, jobs, reviews, translated_works, permissions, translation_memory_entries), Operations (site_metrics, security_events, retention_runs, feature_flags, ab_test_assignments).

---

## 5. Milestones (spec §5–§25) — Detailed Build Order

The build order. Each milestone is a vertical slice — real user behavior end-to-end, not a horizontal layer. Every milestone has acceptance criteria that must pass before the next begins.

### Milestone 0: Repository, Tooling, Running Application (§5)

Workspace setup, dev config, structured logging with request IDs, embedded frontend assets, DB selection, migrations, health endpoints, dev seed, CI, dependency/license checks, OpenAPI gen, Docker, feature flags.

**Commands:** `lorehaven serve`, `worker`, `migrate`, `seed --development`, `doctor`, `flags list/set`. Config precedence: CLI → env → file → default.

**Feature flags (§683):** Server-side flags for gradual rollout and kill switches. Never affect security/privacy/safety. Visible to admins; users not told which flags affect them.

### Milestone 1: Design System, Navigation, Localisation, Extension Slots (§6)

Buttons, inputs, dialogs, tabs, pagination, toasts, work cards, identity switcher, skeletons. Design tokens: `color.background/surface/text/muted/primary/danger/positive`, `space.*`, `radius.*`, `font.interface/reader`. Direction: warm neutral surfaces, deep plum accent, teal secondary, generous spacing, light/dark/system modes.

Navigation: Desktop `Discover | Search | Library | Write | Community | Notifications | Pseud`; Mobile `Discover | Search | Library | Write | More`. Reader pages use reduced shell.

**Extension slots (§6.3):** Dashboard widgets, reader sidebar, work-page metadata, search decorators, writing-tool panels, theme override tokens. Contracts established first so first-party features use the same mechanism as third-party extensions.

Command palette (Ctrl+K), interface localisation, contextual help, appearance modes.

### Milestone 2: Accounts, Pseuds, Privacy, Age Policy, Registration Modes (§7)

Registration, login, password reset, email verification, session listing/revocation, sign-in alerts (opt-out), TOTP + recovery codes, pseud creation/switching, privacy settings, scoped blocks/mutes, age state machine, scoped API tokens, per-pseud learning/reading-history/feedback controls.

**Pseud behavior (§7.2):** Each pseud has separate public profile, works, follows, messages, recommendation settings, ratings, feedback preferences, bookmarks, reading history, dashboard, navigation, extensions, source credentials, notifications, presence, language. Account shares credentials, security state, private wallet, trust eligibility, invite quota. Never publicly reveal shared ownership.

**Age state machine (§7.3):** unknown → declared_minor → declared_adult; or authorization_required → authorized_under_policy → restricted. Self-declared adult is not verified.

**Registration modes (§7.4):** open, invite-only, application-based. Invite code generation and tracking. Registration application queue.

### Milestone 3: Drafts, Chapters, Publishing, Feedback Preferences, Post Drafts (§8)

**First vertical slice (§8.1):** Create draft → add chapter → save → preview → publish → read publicly → edit → republish.

**Work states (§8.2):** Lifecycle: draft|scheduled|published|withdrawn|deleted. Visibility: public|unlisted|restricted. Completion: in_progress|complete|hiatus|abandoned.

**Editor (§8.3):** Restricted Tiptap schema (paragraphs, headings, emphasis, strong, lists, blockquotes, links, scene breaks, author notes, footnotes, endnotes). Author notes are first-class blocks (before/after body, collapsed, excluded from word count/exports/body search). Footnotes/endnotes are structured (anchor resolves to chapter foot or work end matter). Autosave: debounce → local recovery → versioned server update. Imported DOCX/HTML/EPUB/Markdown convert to the same schema.

**Feedback preferences (§8.4):** Per work/pseud/account: accept public comments (yes/no/moderated), anonymous thanks, quick reactions, private thank-yous, constructive critique (opt-in, default off), custom author note.

**Publishing transaction (§8.5):** Single transaction validates metadata, verifies contributors, updates state, records publication event, inserts outbox events. No email inside transaction. Scheduled publication uses same idempotent service.

**Post drafts and scheduled posts (§8.6):** Local draft recovery, optional server-side drafts, draft expiry, scheduled publication (positivity filter applies at publication time, editable/cancel before publish).

### Milestone 4: Reader, Ratings, Reactions, History, Goals (§9)

**Reader features (§9.2):** Chapter navigation, whole-work mode, TOC, typography settings (font family incl. dyslexia-friendly, size, line height, justification), light/dark/sepia/high-contrast themes, distraction-free mode, spoiler reveal, progress, private notes, in-work search, reading-time estimates, end-of-work actions.

**Text-to-speech (§9.2):** Default = browser speech synthesis (free, offline). Higher-quality AI voice as metered task under budget guardrails. Published podfic/audiobook takes precedence. TTS output never stored as work, never indexed, never federated.

**Reader layout persistence (§9.2):** Layout and typography saved per reader per work (not per browser). Custom reader CSS scoped to reading surface, sanitized like marketplace themes, opt-in/off by default.

**Progress (§9.3):** Store work/chapter/revision/anchor/fraction/time/device. Stable paragraph anchors with approximate fallback. Device disagreement presents a choice.

**Quick reactions (§9.4):** One-click labels ("made me cry", "the banter!", "I need more of this", "this scene!", "comforting", "worldbuilding!"). Subject to feedback preferences. Aggregate publicly if author permits. Count once per reader per target. Bypass positivity filter (inherently positive).

**Ratings (§9.4):** Per-pseud, per-work, spoiler-tagged. Public rating aggregate with honest display. Author may hide public rating aggregate without touching private ratings. Author cannot see individual private ratings (only pseud-identity aggregates above threshold). No public rating histogram breakdown.

**Reading history (§9.5):** Private per pseud, reverse-chronological, item type marker (work/chapter/external/library), device, timestamps, completion status, time spent. Pause/resume per pseud. De-duplicated. No cross-device learning from private history beyond aggregate signals.

**Reading goals (§9.6):** Per-pseud: annual reading goal (number of works), optional monthly cadence, progress display, no social pressure, no failure penalty, optional sharing to profile, private by default.

**Gamification (§9.7):** Episodic credits (never cumulative XP), streak tracking, quality multiplier (works with strong quality signals earn more), demand multiplier (1.0x–1.5x, silent, folds in admin taste). Topic bonuses for instance-declared themes. Leaderboards (daily/weekly/monthly windows, never all-time, never volume metrics, opt-out). Milestone badges with credit bonuses. Streak freezes (5 credits).

**Gamification caps (§9.7.8):** Per-action daily caps, rolling 30-day caps, author weekly/daily earning caps, never exceed hard platform ceilings. Credits never negative. Display honest remaining caps.

### Milestone 5: Jobs, Storage, Cache Boundaries, Secret Management (§10)

Async job scheduler with priority queues, retry, backoff, concurrency limits, quote/reserve/capture flow, local file storage with integrity, pluggable object storage, cache boundaries (Redis with in-process fallback), secrets rotation.

### Milestone 6: Imports, Credentials, Batches, Watches, Preservation, Cross-posting (§11)

**Adapter abstraction (§11.1):** Pluggable source adapters with versioned contracts, safe fetching (robots.txt, rate limits, timeouts), per-source credential vault, runtime source health monitoring.

**Destinations (§11.2):** Library item, draft, series. **Entry points (§11.3):** URL paste, bookmarklet, API, bulk import. **Formats (§11.7):** AO3, FFN, Wattpad, generic RSS/Atom/OPDS, HTML scraping, EPUB, DOCX.

**Author bibliography imports and watches (§11.9):** Import an author's full bibliography from a source. Watch for new works and notify subscribers.

**Cross-source identity (§11.10):** Normalize works across sources to prevent duplicates.

**Preservation batches (§11.11):** Curated migration batches from dying archives with permission tracking.

**Cross-posting (§11.12):** Post works to external sites via their APIs.

**Work body retention (§11.15):** Retain original imported body alongside editable draft for provenance and recovery.

### Milestone 7: Positivity Filter and Feedback Delivery (§12)

**Positivity model (§12.1):** Only positive or constructive criticism reaches authors. Destructive negativity is held. Constructive critique requires opt-in from the author.

**Classification implementation (§12.2):** AI-assisted classifier (pluggable provider) scores comments on positivity/constructiveness. Threshold-based routing: positive → delivered; constructive → delivered if author opted in; destructive → held for moderation queue.

**Delivery rules (§12.3):** Held comments enter moderator queue. Authors see only delivered feedback. Commenters see their own held comments with status explanation.

**Commenter experience (§12.4):** Pre-submit positivity nudge, post-submit status, edit to improve classification.

**Quorum review (§12.5):** Held comments reviewed by multiple trusted reviewers (quorum). Disagreements escalate.

**Author overrides (§12.6):** Authors can always see feedback on their own works (privacy override for owned content).

**Appreciation and quick reactions (§12.7):** Bypass positivity filter (inherently positive).

**Author boundary tools (§12.10):** Per-work and per-pseud controls, disable comments, freeze threads, hide individual comments.

### Milestone 8: Exports, Device Delivery, Offline Reading (§13)

EPUB export (validated), PDF, HTML, Markdown, MOBI. Send-to-Kindle and device email. PWA with offline reading, offline privacy (no cached content leaks). Export preserves footnotes/endnotes, excludes author notes from main body.

### Milestone 9: Library, Saved Views, Bookmarks, Updates (§14)

**Core library (§14.1):** Bookmarks (public/private per pseud), notes, shelves (named, public/private), reading progress, reading events/aggregates, library item types (work, external record).

**Saved searches and named views (§14.2):** Saved search queries with named views, search alerts (notify when new works match), batch actions on result sets.

### Milestone 10: Structured Taxonomy, Mood Search, Query Language, Fuzzy Matching (§15)

**Character assertions (§15.1):** Per-work character tags with attributes (role, prominence) and canonicalization.

**Relationship assertions (§15.2):** Typed relationships (romantic, familial, antagonistic) with participant sets, dynamics, spoiler status.

**Typed query AST (§15.3):** Structured query representation.

**User-facing query language (§15.4):** Human-readable search syntax: `fandom:HP tag:time-travel rating:teen word-count:<10k mood:angsty`.

**Fuzzy matching (§15.5):** Typo tolerance via FTS5 trigram / pg_trgm, correction suggestions.

**Metadata completeness (§15.6):** Score works by metadata richness, suggest improvements.

**Filters (§15.7):** Rating, completion status, language, word count, relationship type, character, content notes.

**Mood and tone taxonomy (§15.8):** Mood tags (angsty, fluff, humorous, dark) for discovery.

**Full-text body search (§15.9):** Search within work bodies (respects visibility, excludes author notes).

**Ranking (§15.10):** Default sort = freshness × quality × taste alignment.

**Canonicalization and metadata correction (§15.11):** Merge duplicate tags, canonicalize characters/relationships.

**People directory (§15.12):** Public directory of authors/curators.

**Fandom landing pages (§15.13):** Curated fandom pages with editorial picks, trending works.

**Zero-result demand insights (§15.14):** Aggregate and publish what readers search for but don't find.

### Milestone 11: Discovery, Taste Influence, Recipes, and Dashboards (§16)

**Baseline engines (§16.1):** Recent, trending, content similarity, also-bookmarked, Blind Date, user-preference matching, community-suggested similarity, completion-rate weighted, mood-matched, cross-fandom dynamic, complete-and-under-read. Each engine implements `generate_candidates(context, limit) → IDs + scores`. All candidates pass shared eligibility rules.

**Feed reason transparency (§16.1):** Every feed item may include `reason` — which of the reader's own recipe terms matched ("because: fandom:HP, tag:time travel"). Surfacing it to its owner is the default. Distinct from admin-influence secrecy: truth about one's own feed costs nothing. Instance-level terms only named for `public = true` topics.

**Administrator taste profile (§16.2):** Explicitly selected taste-source profile. Excludes moderation sessions, import tests, accidental opens. Signals: likes/dislikes, bookmarks, private ratings, completions, seed prompts, wishlists. One profile, recomputed on demand, inspectable by admin. Long-term + recent blend is implementation detail, not published number. Profile versions kept.

**Influence layer (§16.3):** Admin taste profile shapes recommendations through silent gravitational pull (never labeled, never disclosed).

**Diversity budget (§16.4):** Percentage of recommendation slots reserved for non-admin-aligned content to prevent monothematic drift.

**Meaningful opt-out (§16.5):** Per reader: "show me only my taste", "show me only admin-aligned", "show me everything", "surprise me". Opt-out dial governs the influence part that is not the reader's own.

**Recommendation recipe builder (§16.7):** Composable recommendation recipes (weighted engine combinations), admin-seeded default recipes, reader-customizable.

**Widget-composed dashboard (§16.8):** Customizable dashboard with widget instances (trending, recent, recommendations, leaderboards, streak, etc.).

**Blind-spot and surprise-me (§16.10):** Deliberately surface content outside reader's usual patterns.

### Milestone 12: Comments, Forums, Groups, Messaging, Presence (§17)

**Comments and reviews (§17.1):** Work/chapter comments, replies, appreciation, spoiler formatting, edit history, author locking, reporting, block/mute enforcement. All pass positivity filter. Nesting depth limited.

**Forums (§17.2):** Categories, boards, topics, posts, polls, reactions, pins, locks, subscriptions, topic tags, forum full-text search, mentions, read state. Lighter positivity policy (constructive disagreement allowed; hostility filtered).

**Read state (§17.3):** Per-pseud high-water mark using stable post ordering. Paginated entry doesn't mark unseen pages read. "Mark read" is explicit. Deleted/hidden posts don't break cursors.

**Forum search and tags (§17.4):** Topic titles, post bodies, category scope, author, tags, dates, highlighted snippets.

**Groups (§17.5):** Membership, roles, group pages, group libraries, group reading events.

**Messaging and presence (§17.6):** Direct messages, conversation threading, presence preferences, typing indicators, read receipts.

### Milestone 13: Collections, Challenges, Requests, Wishlists, Writing Events (§18)

**Collections (§18.1):** Curator roles, invitations, submission approval, sections, work-owner withdrawal, preservation-batch provenance links. No republishing permission granted.

**Challenges (§18.2):** Signups, prompt pools, assignments, claims, deadlines, reveal dates, anonymous-until-reveal, fulfillment. Variants via shared configurable workflow: exchanges, pinch hits, treats, fests, Big Bang variants. Finished-work reading challenges ("Read 5 completed fics under 10k words this month"). Private by default, no public ranking, no trust reward.

**Mentorship and beta-reading (§18.3):** Applications, availability, interest matching, session completion. Reciprocal beta-reading matching (pairs matched for mutual beta).

**Sprints (§18.4):** Start/end time, shared room, optional counters, no publication requirement.

**Requests, bounties, wishlists (§18.5):** Search help, recommendation request, writing prompt, wishlist item (no obligation), commission/bounty (with escrow), preservation request. Wishlists are public boards; fulfillments link back.

### Milestone 14: Trust, Reports, Quorum, Appeals, Sanctions, Process Feedback (§19)

**Trust levels (§19.1):** TL0 (new) → TL1 (established) → TL2 (regular) → TL3 (reviewed trusted) → TL4 (trained steward) → TL5 (senior independent) → TL6 (appointed trustee). Requires reviewed conduct, not point totals. Core publishing/reading available at TL0. Trust is the safety ramp of demand weighting (a week-old account's demand counts for less).

**Effects (§19.2):** Trust increases rate limits, batch sizes, proposal/curator eligibility, extension ceilings, gift/bounty limits, moderation queue eligibility, wishlist priority, invite quota. Does not grant private-message access or administrator powers.

**Reports (§19.3):** Work, comment, user, source reports. Report reasons, evidence attachment, duplicate detection, report history.

**Cases (§19.4):** Case lifecycle (open → under review → resolved), case assignment, priority, notes, linked reports.

**Quorum (§19.5):** Multiple reviewers for significant decisions. Quorum records kept. Single-admin mode is default (quorum fields exist but default to operator).

**Sanctions (§19.6):** Warnings, temporary restrictions, suspensions, bans. Graduated. Shadowban with documented scope and appeal path.

**Appeals (§19.7):** Appeal process for sanctions and moderation decisions. Independent reviewer pool.

### Milestone 15: Credits, Fair Queues, Bounties, and Billing (§20)

**Ledger (§20.1):** Balanced entries (not balance mutation). transaction_id, type, idempotency_key, reference, entries[], created_at. Separate earned/subscription/purchased/held credits.

**Job charging (§20.2):** quote → reserve → submit → complete → capture. Failure releases hold or applies documented partial charge.

**Initial credit economy (§20.3):** Daily login (5), read chapter (1), quick reaction (1), positive comment (2), constructive review (3), forum post/topic (1/2), bookmark (1), import (2), import new source (10), mark finished (5), finish short/long work (3/10), finish in new fandom (3), fulfill wishlist (15), translation chapter (5), quorum vote (2), author unique reader (1), author completion (5), author reactions/comments/bookmarks (1/2/1), author quality multiplier (up to 1.8x), author demand multiplier (up to 1.5x silent), leaderboard rewards (1st=50, 2nd-3rd=25, 4th-10th=10, participation=2), badges with credit bonuses.

**Author earnings (§20.9):** Tips, subscription revenue (attributed by reading time), bounty payouts, marketplace sales.

**The cap (§20.10.3):** Graduated damper on Pool A income only. Reference point = active-earner median Pool A income (trailing 3-month window, configurable multiplier default 10). Soft graduated bands: up to 5× median keep 100%; 5×–10× keep 50%; above 10× keep 0%. Overflow spills to Pool B (solidarity pool).

**Pool B distribution (§20.10.4):** Proportional to per-author quality score (never word/chapter/work count). One excellent oneshot can out-earn a high-volume author.

### Milestone 16: Marketplace, Extension Isolation, Webhooks, Gallery Mechanics (§21)

**Manifest (§21.1):** Extension manifest with permissions, resource limits, entry points.

**Categories (§21.2):** Themes, recommendation engines, writing tools, reader enhancements, feed widgets, challenge variants.

**Capabilities (§21.3):** WASM-based execution with sandboxed host APIs, permission grants, resource metering (CPU, memory, storage).

**WASM execution (§21.4):** `wasmi` runtime, deterministic execution, fuel metering, memory limits.

**Review workflow (§21.5):** Extension review queue under positivity framework.

**Themes and layout safety (§21.7):** CSS sanitization, no data exfiltration through selectors, reset after broken theme.

**Paid marketplace (§21.9):** Extension purchases with credit splits.

### Milestone 17: Translation Pipeline (§22)

**Scope (§22.1):** Interface translation (UI chrome, help, errors, docs — human-reviewed catalogs) + content translation (works, comments, posts, summaries, tags — on-demand AI with human review queue).

**Content translation flow (§22.2):** Reader requests → AI translates → human review queue → approved → stored as translated version.

**Cost model (§22.3):** Metered AI task, quoted before running.

**Permission handling (§22.4):** Translation requires author permission (opt-in per work).

**AI overrides human (§22.5):** Approved human translation always supersedes AI.

**Machine-translation labeling (§22.6):** Clear "machine-translated" badge on AI translations.

**Translation memory (§22.7):** Reuse approved translations for consistency.

### Milestone 18: Public API, Bots, Feeds, Push, Federation, AI Providers (§23)

**Public developer API (§23.1):** OpenAPI spec, interactive docs, scoped tokens, acting pseud, cursor pagination, rate-limit headers, idempotency, versioning/deprecation, example clients.

**Chat bots (§23.2):** Bots as thin REST clients with scoped tokens.

**Notifications (§23.3):** In-app, email, push notifications with preferences.

**RSS/Atom/OPDS/sitemaps (§23.5):** Standard feed formats for works, searches, series.

**ActivityPub (§23.6):** Federation for announcements (new works, challenges, collections).

**AI provider interface (§23.7):** Pluggable AI adapters (Ollama, OpenAI, etc.) for translation, summarization, classification.

**Natural-language search assist (§23.8):** AI-assisted query refinement.

**Fic trailers, mood boards, rich sharing (§23.9):** Visual sharing assets for works.

### Milestone 19: Administration, Statistics, Abuse Defense, Privacy, Operations (§24)

**Administration routes (§24.1):** /admin/users, /policies, /discovery, /importers, /preservation, /jobs, /storage, /retention, /metadata, /abuse.

**Public statistics (§24.2):** Honest aggregate stats (works, authors, reads, translation counts).

**Privacy-preserving analytics (§24.3):** Aggregate analytics without individual tracking.

**Abuse dashboard (§24.4):** Reports, cases, sanctions overview.

**Layered abuse defense (§24.5):** Positivity filter → reports → quorum review → sanctions. Trust-based rate limiting. Shadowban for subtle abuse.

**A/B testing (§24.6):** Feature experimentation with consent.

**Data export (§24.7):** GDPR-style data export for all user data.

**Deletion (§24.8):** Account and data deletion with grace period.

**Backups (§24.9):** Automated backup with integrity verification.

**Upgrades (§24.10):** Online schema migration with rollback.

**Caretaker mode (§24.11):** Succession planning, break-glass access.

**Account dormancy (§24.13):** Dormancy detection, data lifecycle management.

**AI crawlers (§24.14):** Refused by default. AI training availability is author's statement, presented as such.

### Milestone 20: Hardening and Release (§25)

**Functional browser journeys (§25.1):** Playwright tests covering all critical user journeys (register → write → publish → read → comment → export).

**Security tests (§25.2):** OWASP top 10, session management, CSRF, XSS, SQL injection, authorization bypass.

**Accessibility (§25.3):** WCAG 2.1 AA compliance, screen reader support, keyboard navigation, dyslexia-friendly fonts.

**Performance (§25.4):** Page load budgets, long-work rendering (not all paragraphs at once), database query budgets.

**Platform evidence (§25.5):** Cross-browser, mobile, PWA installation.

**Cognitive accessibility (§25.6):** Clear language, consistent navigation, error prevention.

---

## 6. Route and API Ownership (§26)

Every route and API endpoint is owned by exactly one milestone. Route registration is centralized. `/api/v1` prefix for application endpoints; administrative endpoints under `/admin`. Route ownership prevents cross-milestone coupling.

---

## 7. Tutorial Delivery Plan (§27)

In-context tutorials delivered via extension slot system. Progressive disclosure: first-run experience, contextual help, command palette hints. Localized through the same message catalogs as the interface.

---

## 8. Final Completion Checklist (§28)

Every milestone must pass its acceptance criteria before the next begins. Honest verification: tests pass, features work, performance targets met. Feature presence in spec ≠ implementation evidence.

---

## 9. Community Roadmap, Feature Consensus, Changelog (§29)

Public roadmap with feature consensus voting (Elo-ranked). Community proposes and votes on features. Changelog tracks shipped features. Roadmap is aspirational, not commitment.

---

## 10. Non-written Works, Media, and Aggregation (§30)

Support for non-written works: podfic, audiobook, art, multimedia. Media assets with storage, checksums, type detection. Media query API for searching across media types. Aggregation of works into story identities (canonical metadata grouping multiple editions/formats of the same story).

---

## 11. Sharing: Fic Cards, Unfurling, Link Previews (§31)

Visual fic cards for social media sharing. Open Graph / Twitter Card metadata for work pages. Link preview generation with title, author, summary, cover image.

---

## 12. Generalized Media Platform: Creators, Distributors, Collections, Media Query API (§32)

Lorehaven as a generalized media platform beyond fiction. Creators (authors, artists, podficcers), distributors (sources, importers), collections (curated groupings), media query API (unified search across media types). Extends the work/chapter model to support audio, video, images alongside text.

---

## 13. Consent, Integrity, and Transparency (§33)

**Consent framework:** explicit permission for translation, AI training, cross-posting, data use. Granular per-work and per-pseud controls.

**Integrity:** content checksums, revision history, provenance records, edit attribution.

**Transparency:** honest labels (machine-translated, AI-assisted, curated), disclosure of algorithmic influence (instance-level topics public, admin taste invisible).

---

## 14. Decision Services (§34)

Calibrated classifiers for positivity, metadata quality, duplicate detection, abuse. Calibration = documented confidence thresholds, human review for borderline cases, appeal paths. Services are pluggable and auditable.

---

## 15. Forum as a First-Class Surface (§35)

**Core principle:** One conversation, one place. Quick reactions and real discussion are different behaviors. Reactions = signal (typed votes), not conversation. Text discussion belongs in exactly one place: a linked forum thread with full infrastructure (threading, search, moderation, typed votes, follow/notify, federation).

**Typed votes (§35.2):** Slashdot-style meta-moderation replaces generic likes. `well-written`, `insightful`, `creative` are inherently positive signals. Abuse handled by meta-moderation, not text classifier. Votes and karma never purchase trust, moderation authority, or search ranking.

**Work discussion modes (§35.0):** Each work carries a mode (instance default, per-author override):
- `ThreadOnly` (default): typed-vote reaction bar on work page, all text discussion in linked forum thread
- `CommentsOnly`: inline work/chapter comments, no forum link (legacy)
- `Both`: both surfaces, admin-warned (splits conversation)

---

## 16. Growth, Sharing & Ecosystem Expansion (§36)

Growth strategies: SEO (public works indexable, honest metadata), social sharing (fic cards, link previews), federation (ActivityPub announcements), cross-posting to external platforms, import friction reduction. Ecosystem expansion: marketplace, extensions, translations, challenges.

---

## 17. Multi-Platform Companion Bot (§37)

Official companion bot for chat platforms (Telegram, Discord, Matrix). Bot features: notifications (new works, comments, challenge updates), reading list management, quick search, reading progress. Bot is a thin REST client with scoped tokens. Bot identity managed per instance.

---

## 18. Instance Configuration (§38)

`lorehaven.toml` configuration file. Sections: [site] (name, URL, taste_profile), [signals] (mode, weights), [discovery] (diversity_budget, recipe defaults), [gamification] (credit caps, leaderboard config), [economy] (cap_multiplier, pool_b split), [extensions] (marketplace, resource limits), [translation] (providers, cost model), [federation] (ActivityPub), [ai] (providers, models). Documented defaults for every parameter. Single-admin mode as default.

---

## 19. Resource Directory (§39)

Community-curated map of the fandom ecosystem: external sites, archives, communities, events, challenges. Curated by trusted contributors (vanguard). Searchable and browsable. Demand-weighted (what readers look for but can't find on-instance is highlighted). Vote weighting modes: flat | trust | trust_taste | trust_taste_contribution.

**Category governance (§45):** categories are community-moderated — proposed by trusted users, executed by quorum, with operator override, anti-churn guards, a public changelog, and entry moderation by quorum.

---

## 20. Remix (§40)

Fork works with provenance. Attribution preserved across forks. Permission statements made enforceable (author declares remix policy: allowed/with-credit/not-allowed). Fork chain tracked. Remix culture encouraged while respecting author boundaries.

---

## 21. Longevity and Ambient Social Signals (§41)

Long-term engagement through ambient signals: streak indicators, reading goals progress, "you haven't read from this fandom in a while", gentle re-engagement. Not manipulative (no dark patterns). Episodic rewards over cumulative assets.

---

## 22. Export CTAs (§42)

Instance-configurable calls-to-action on exported works: "find more at [instance]", "this work was imported from [source]", curator-marked recommendations. Respects author's export preferences.

---

## 23. Recommendation-First Browsing (§43)

One ordering contract for every surface (discover, search, fandom, tag, author, notifications): `RecommendationRecipe` → eligibility filter → scoring → diversity injection → render. Same contract everywhere; surfaces differ in context, not mechanism.

---

## 24. Roadmap Consensus — Elo-Ranked Feature Board (§44)

Community proposes features. Voters are pairwise-presented two proposals and pick which they'd rather see implemented. Elo ranking emerges. Admin can override but transparency shows divergence. Board resets per planning cycle. Prevents loudest-voice-wins, surfaces true preference intensity.

---

## 25. Taste-Gravitational System Amendment

The amendment consolidates and extends the existing taste infrastructure (demand multiplier, theme gravity, admin taste profile) into a full **taste-gravity engine**. Where this amendment conflicts with the base spec, the amendment governs. Amends §0.4, §0.4.6, §9.7, §9.7.1, §9.7.3, §9.7.5, §14.5, §16.2, §16.17–16.19, §17.6, §19, §20.3.

### Instance Taste Profile (amends §0.4)

Companion to Instance Topics. Both optional; neither = taste-neutral (gravity engine disabled).

Profile structure:
- `dimensions: [{key, label, admin_target, weight}]` — named axes (angst, pacing, prose_density, canon_compliance, trope_diversity). admin_target is admin's ideal (0.0–1.0). weight determines influence. Instance-defined, no hardcoded taxonomy.
- `anti_examples: [work_id]` — works admin explicitly dislikes (negative anchors)
- `exemplars: [work_id]` — works admin loves (positive anchors)

Multi-dimensional over scalar: a single resonance score collapses "loves darkfic in one fandom and fluff in another" into noise. Dimensions let the system distinguish axes. Scalar resonance remains computable as a projection.

Config: `[site] taste_profile = { enabled, dimensions, exemplars, anti_examples }`. Reported on `/api/v1/meta` only as `taste_enabled: true/false` — never dimensions, never targets.

### Taste-Weighted Signals (amends §9.7.3)

The demand multiplier (1.0×–1.5×) expands into a general system operating on the entire recommendation algorithm, not just credit earnings.

Every user action (kudos, bookmark, comment, completion, reading time, re-read) carries a **signal weight** equal to that user's alignment with the instance taste profile. Signal weights feed into:
- Recommendation scoring on every surface
- Credit demand multiplier (existing behavior, now derived from alignment)
- Community signal aggregation (bookmark from 0.9-aligned user counts more than from 0.1-aligned user)

**Alignment score** = Taste Resonance Score. Always hidden. Admin's own actions carry maximum weight (1.0×).

Instance config:
```yaml
[signals]
mode = "taste_weighted"    # egalitarian | taste_weighted | admin_only
```
- `egalitarian`: all signals equal (demand multiplier only)
- `taste_weighted`: signals weighted by resonance (default when taste profile exists)
- `admin_only`: only admin actions influence recommendations

An instance without a taste profile defaults to `egalitarian`. Signal weights, resonance scores, and their influence on any ranking are invisible in any label, tooltip, breakdown, or API response. Only the behavior changes.

### Streak-Linked Flat Bonuses (amends §9.7.1)

Credits for actions remain episodic and non-cumulative. A flat bonus may be granted for streak milestones:
- 7-day streak: +10 flat credits (one-time)
- 30-day streak: +30 flat credits
- Streak freeze: 5 credits (unchanged)

Rationale: streak multipliers inflate all signal equally, diluting taste-weighted credits. Flat bonuses reward the habit without distorting per-action economics.

### Taste Resonance Score (§16.17, NEW)

The hidden per-user alignment metric powering Taste-Weighted Signals, Vanguard selection, Taste notifications, and Trust × Taste coupling.

**Computation (per account):**
```
resonance = weighted_sum(
    bookmark_overlap,        # Jaccard similarity of user vs admin exemplars (0.30)
    rating_correlation,       # correlation of shared ratings (0.25)
    completion_alignment,     # fraction of admin-liked works this user finished (0.25)
    reading_time_ratio,       # time on admin-aligned works / total (0.20)
)
```
Weights are instance-configurable. Clamped to 0.0–1.0.

**Update cadence:**
- Incremental: on each bookmark/rating/completion, recompute affected component only (near-real-time)
- Full recompute: weekly batch job (correction pass)
- Cold start: new accounts with <5 bookmarks AND <5 ratings: resonance = 0.0, signals fall back to `egalitarian`

**Visibility:** Never shown to any user except the account owner via `GET /api/v1/me/taste-resonance`. Shown as qualitative label ("Aligned: Strong" for >0.7) rather than raw number, to discourage optimization. Admin sees all users' resonance in admin panel. Instance without taste profile: computation disabled, signals default to `egalitarian`.

### Taste Vanguard Role (§16.18, NEW)

Top-aligned users who act as the admin's taste scouts (curatorial layer).

**Selection method (config: `vanguard.method`):**
| Method | Default |
|---|---|
| `resonance_threshold` (top N%, default 10%) | when taste profile exists |
| `admin_appointment` | |
| `community_election` (quarterly) | |
| `contribution_volume` | |
| `hybrid` | |

Weekly batch job applies selection. Admin can override at any time.

**Vanguard permissions:**
- Pin works to fandom pages (30 days, logged, revocable)
- Nominate works for admin review (enters queue, not auto-approved)
- Vanguard Picks shelf (front page)
- Create reading clubs
- Provisional bounty triggering
- Post bounties at 50% cost reduction

**Profile badge:** visible when `vanguard.public_badge = true`. Badge says "Vanguard" — never reveals why (never mentions taste/resonance/admin). Instance without taste profile: selection defaults to `contribution_volume` or `admin_appointment`.

### Instance Presets (§0.6, NEW)

Bundles of configuration defaults so operators don't tune 40 parameters individually. Selectable at setup, individually overridable after.

| Preset | Taste gravity | Diversity injection | Leaderboards | Vanguard | Best for |
|---|---|---|---|---|---|
| `gallery` | Strong (1.5×) | 10% | Taste-aligned categories | Resonance-based | Curated showcases |
| `archive` | Mild (1.2×) | 25% | Per-topic boards | Hybrid | General-purpose libraries |
| `commons` | Off (egalitarian) | 30% | Public-topic only | Community election | Community-driven spaces |
| `showcase` | Strong (1.5×) | 15% | Single "Admin's Picks" | Admin appointment | Single-editor showcases |
| `sandbox` | Off | Off | Off | None | Testing, development |

Each preset defines: taste profile dimensions (if any), signal mode, diversity budget percent, recipe defaults, leaderboard categories, vanguard method, economy cap multiplier, extension marketplace availability.

### Reading Clubs (§17.6, NEW)

Vanguard-created or admin-created reading clubs: groups of readers working through a shared reading list. Club features: shared reading list, discussion thread per work, progress tracking, club shelf on member profiles. Clubs can be public or invite-only. Club activity feeds into taste-weighted signals (club-aligned reading counts as taste-aligned).

### Demand Weighting Modes (amends §16.16)

Demand weight modes extended for taste-gravitational system:
```toml
[weighting]
mode = "trust_taste_contribution"  # flat | trust | trust_taste | trust_taste_contribution
taste_floor = 0.75
taste_ceiling = 1.25
contribution_floor = 1.0
contribution_ceiling = 2.0
contribution_window_days = 180
demand_diversity_percent = 20
```
`mode = "flat"` is the off switch (every demand signal weighs 1). No XP/level/points/reputation value here — weight is computed when used, never accumulated. Recognition is episodic (leaderboards, badges).

**What contribution may count:** finishing works with strong quality signals (in the demand item's domain); new-to-instance imports, first translations, first narration editions; fulfilling demand (wishlist/bounty, requester-confirmed).

**What contribution never counts:** chapters read, works published, words written, posts made, hours online, days logged in; re-importing a work the instance already had; fulfilling own request; voting on something you nominated.

---

## 26. Design Notes for Brainstorming

**Tensions worth exploring further:**

1. **Positivity filter vs. free expression:** The filter is opt-in for constructive critique and filters destructive negativity. Where is the line between "destructive" and "uncomfortable but valuable"? How does quorum review avoid groupthink?

2. **Taste gravity vs. monothematic drift:** The diversity budget (§16.4) and instance presets address this, but the admin's taste is always pulling. How strong should the ceiling be? Is the graduated cap model (§20.10.3) a good metaphor?

3. **Vanguard selection fairness:** Resonance-based selection rewards readers who already align with admin taste. Does this create a taste elite? Is community_election a sufficient counterweight?

4. **Trust vs. activity-volume:** The spec explicitly forbids trust from post count/kudos/credits. But "reviewed conduct" requires reviewers. On a small instance, who reviews? Single-admin mode handles this but concentrates power.

5. **Episodic vs. cumulative rewards:** No XP, no all-time leaderboards, no permanent multipliers. Does this actually reduce grind mentality, or just hide it behind leaderboard resets?

6. **Translation permission:** Author must opt in to translation. How is this presented? Can authors set a default? What about orphaned works where the author is gone?

7. **The cap (§20.10.3):** Soft graduated bands (100% / 50% / 0%) with trailing 3-month median. Does this actually prevent winner-take-all? Could authors game the trailing window?

8. **Machine-translation labeling:** "Machine-translated" badge is honest but may reduce trust in translations. Is there a "human-reviewed" badge too?

9. **Forum as first-class (§35):** ThreadOnly default moves discussion off the work page. Does this hurt engagement for casual readers who won't click through to a forum? Does it improve discussion quality enough to compensate?

10. **Reader layout persistence (§9.2):** Saved per reader per work, not per browser. What happens when the work is edited and paragraph anchors shift?

**Open questions:**

- How does the positivity filter handle sarcasm, irony, cultural context?
- What happens to a work's visibility when the admin's taste profile changes?
- Can readers opt out of taste-weighted recommendations entirely? (§16.5 says yes, but how prominent is the option?)
- How does cross-posting handle DMCA takedowns on the destination site?
- What's the minimum viable trust quorum for a 3-person instance?

---

## 27. Brainstorming Prompts

These are starting points for ideation, not spec requirements. Each is a design space where the spec leaves room for creative interpretation.

#### Customization and Extension

- **Theme marketplace curation:** How are themes reviewed and ranked? Should there be a "staff pick" program? Can themes be forked like works?
- **Recommendation engine plugins:** If someone writes a better trending algorithm, how is it integrated? Can engines be A/B tested per-user?
- **Widget composition:** Can widgets read from multiple sources? Can a widget display a live leaderboard, a reading goal, and a fandom trend simultaneously?
- **Writing tool panels:** What would a collaborative outline plugin look like? A character relationship visualizer? A timeline tool?
- **Challenge variants:** The spec mentions exchanges, pinch hits, treats, fests, Big Bang. What other variants exist in fandom culture? Gift exchanges? Secret Santa? Kink memes?

#### Discovery and Taste

- **Feed reason transparency (§16.1):** The `reason` field shows which recipe terms matched. Could this become a reader-facing "why am I seeing this" tooltip? Would that increase trust or just gamification awareness?
- **Diversity budget dials:** §16.5 offers opt-out levels. Could there be a slider (0%–100% admin-aligned)? Would that be too revealing of the admin's influence?
- **Blind Date mode:** What if Blind Date became a daily push notification with one random work? Could that be a retention hook?
- **Cross-fandom matching (§16.1):** How does the system know what "cross-fandom" means? Is it based on reader behavior (readers who like A also like B across fandoms)?
- **Mood-based discovery:** The mood taxonomy (§15.8) exists. Could there be a "mood map" — a visual navigation surface for emotional territory?

#### Positivity and Feedback

- **Positivity filter calibration:** How is the classifier trained? On what data? Who labels it? What's the false positive rate target?
- **Constructive critique opt-in:** §8.4 says default off. Could there be per-fandom norms? Some fandoms have strong critique cultures; others don't.
- **Author-visible feedback view (§12.9):** What does this look like? Can authors see held comments? Can they see the classifier's confidence score?
- **Quorum review workload:** If there are 100 held comments per day and 5 quorum reviewers, each reviews 20. How is the queue prioritized? First-in-first-out? Severity?
- **Quick reaction labels:** The spec lists initial labels. Can the marketplace add new ones? Can reactions be customized per instance?

#### Governance and Trust

- **Trust level visibility:** The spec says trust is not calculated from activity metrics. But are trust levels visible to other users? If so, how? If not, how do reviewers know who to assign?
- **Quorum reviewer selection:** How are reviewers chosen for a case? Random from eligible pool? Volunteer? Admin-assigned? Does the reported user have any say?
- **Shadowban transparency (§19.6):** The spec says shadowban has a "documented scope and appeal path." What does the user see? How do they discover they're shadowbanned?
- **Caretaker mode (§24.11):** What happens if the admin is incapacitated for 6 months? Is there a time-based auto-escalation to a trustee?
- **Single-admin default:** The spec says TL4–TL6 are opt-in. On a small instance, is there any governance at all? Or is the admin truly sovereign?

#### Economy and Credits

- **Credit sinks:** The spec lists earning actions. What spends credits? Job charges (imports, translation, device delivery). What else? Could there be a "boost my work" feature? Should there be?
- **Streak freeze (5 credits):** A streak freeze costs 5 credits. Is that expensive or cheap? Does it encourage or discourage streak maintenance?
- **Author quality multiplier (up to 1.8x):** What defines "quality"? Reading completion rate? Feedback signals? How is it calculated without gaming?
- **Pool B distribution (§20.10.4):** Quality score for Pool B. If one author has one excellent oneshot and another has 50 mediocre works, who gets more Pool B? How is "excellent" defined?
- **Cap gaming (§20.10.3):** Trailing 3-month median. Could an author strategically under-earn for 3 months to lower the median, then earn heavily? How is this prevented?

#### Translation

- **Translation memory (§22.7):** Shared approved translations for consistency. Is this per-work or per-instance? Can it be shared across instances?
- **Human review queue (§22.2):** Who reviews translations? Trusted translators? Any TL3+ user? Is there a translation-specific trust role?
- **AI overrides human (§22.5):** If an approved human translation exists, AI cannot override. But what if the AI translation is clearly better? Is there an appeal path?
- **Translation of comments (§22.8):** Comments are translated too. Is this on-demand or batch? What about nested replies?

#### Search and Taxonomy

- **Fandom landing pages (§15.13):** "Curated fandom pages with editorial picks." Who curates? Admin only? Vanguard? Community vote?
- **Zero-result demand insights (§15.14):** "Aggregate and publish what readers search for but don't find." Could this become a public "most wanted" list that authors can browse for inspiration?
- **People directory (§15.12):** "Public directory of authors and curators." What's in a directory entry? Pseuds, works, trust level? Can users opt out?
- **Canonicalization (§15.11):** "Merge duplicate tags." Who proposes merges? How are conflicts resolved? What about tags that are similar but distinct (e.g., "angst" vs "hurt/comfort")?

#### Importing and Preservation

- **Adapter versioning (§11.1):** Adapters have versions with verification_status. What happens when a source changes its HTML? Is there an automated test suite?
- **Source health windows (§4.4):** Track outcome counts and latency. Is there an alert when a source degrades? Does the system auto-disable failing adapters?
- **Preservation batches (§11.11):** "Curated migration batches from dying archives with permission tracking." Who initiates? What permission is needed?
- **Work body retention (§11.15):** "Retain original imported body alongside editable draft." How long is it retained? Forever? What about storage costs?

#### Federation and API

- **ActivityPub scope (§23.6):** "Federation for announcements (new works, challenges, collections)." What about comments? Could a comment on a federated work appear on the original instance?
- **Chat bots (§23.2):** "Bots as thin REST clients with scoped tokens." What can bots do? Read works? Post comments? What are the rate limits?
- **AI provider interface (§23.7):** Pluggable adapters. What's the fallback if the AI provider is down? Does the positivity filter degrade gracefully?
- **Natural-language search assist (§23.8):** AI-assisted query refinement. Does this require an AI provider? What's the user experience when none is configured?

#### Media and Non-written Works

- **Podfic/audiobook support (§9.2, §30):** "A published podfic or audiobook unit always takes precedence over generated speech." How is podfic stored? As a media asset linked to a work? As a chapter?
- **Art and multimedia (§30):** "Support for non-written works: podfic, audiobook, art, multimedia." Can an art piece be a "work" in the system? Can it have chapters (gallery pages)?
- **Media query API (§32):** "Unified search across media types." Does this include text, audio, video, images in one result set? How is relevance ranked across types?
- **Fic trailers (§23.9):** "Visual sharing assets for works." Are these uploaded by authors? Generated automatically? Stored as media assets?

#### Reading Experience

- **TTS quality tiers (§9.2):** Browser speech synthesis vs AI voice. What's the quality difference? Is there a middle tier (e.g., a lighter AI model)?
- **Custom reader CSS (§9.2):** "Sanitized by the same pipeline as marketplace themes." What's allowed? Custom fonts? Layout changes? Color schemes? Can users share CSS snippets?
- **Reading-time estimates (§9.2):** Based on what? Average reading speed? Personal reading speed? Word count only or does it account for formatting?
- **Distraction-free mode (§9.2):** What does this hide? Navigation? Comments? Metadata? Is it per-work or global?

#### Writing Experience

- **Autosave conflict resolution (§8.3):** "Preserve local text when server rejects." How does the user resolve conflicts? Side-by-side diff? Overwrite?
- **Footnotes/endnotes (§8.3):** "Structured: a footnote anchor in the body resolves to a note rendered at the chapter's foot." In whole-work mode, do endnotes from all chapters aggregate at the end? Can readers toggle between inline and end-of-chapter display?
- **Post drafts (§8.6):** "Automatic draft recovery on refresh." Is this localStorage only? Or server-side per pseud? How much history is kept?
- **Scheduled posts and publishing (§8.5):** "Scheduled publication uses the same idempotent service." What if the server is down at the scheduled time? Is there a catch-up mechanism?

#### Community and Social

- **Groups (§17.5):** "Membership, roles, group pages, group libraries, group reading events." Can groups be private/invite-only? Can they have their own forums?
- **Mentorship matching (§18.3):** "Reciprocal beta-reading matching: authors opt in with fandoms, strengths, and needs." How is the matching algorithm? Is it manual? Automated?
- **Sprints (§18.4):** "Optional shared room, private/public counters, no publication requirement." What's the "shared room"? A chat room? A forum thread?
- **Wishlists (§18.5):** "Wishlists are public boards where anyone can post 'I'd love a fic where X.'" Can wishlists be upvoted? Can authors mark a wishlist item as "in progress"?

#### Administration

- **Abuse dashboard (§24.4):** "Reports, cases, sanctions overview." What metrics? Response time? Case backlog? Recidivism rate?
- **Retention runs (§24.9):** "Automated backup with integrity verification." What's the retention policy? Daily for a week, monthly for a year? Off-site backup?
- **Dormancy (§24.13):** "Dormancy detection, data lifecycle management." What happens to a dormant account's works? Are they deleted? Does the author get a warning?
- **AI crawlers (§24.14):** "Refused by default." Is this robots.txt? A crawler user-agent block? A legal notice? Can authors opt in to allow crawling?

---

## 28. Cross-cutting Themes for Brainstorming

These themes recur across multiple sections and are fertile ground for ideation.

**Honesty and transparency.** The spec repeatedly demands honest labels, honest verification claims, and honest display of aggregates. But it also demands secrecy (admin taste invisible, signal weights invisible). Where is the line? What does "honest" mean when the system is deliberately opaque about some influences?

**Episodic vs. cumulative.** No XP, no all-time leaderboards, no permanent multipliers, streak bonuses are flat not percentage. This is a deliberate anti-grind philosophy. Does it work? Or do users just find new things to grind?

**Single-instance simplicity.** Multi-tenancy is excluded. Operators who need multiple communities run multiple instances. This simplifies everything (data isolation, pseud privacy, resource accounting) but limits network effects. Could instances federate for discovery while remaining separate for data?

**Trust as safety ramp.** Trust levels gate rate limits, batch sizes, moderation queue eligibility. Trust is earned through reviewed conduct, not activity. This is a deliberate inversion of "karma = posts" models. Does it scale? What happens when an instance has 100 active users and 3 reviewers?

**Positivity as infrastructure.** The positivity filter is not a moderation tool; it's a design philosophy. Destructive negativity is held, not deleted. Constructive critique requires opt-in. Quick reactions bypass the filter entirely. This is a bet that most negativity is noise and most positivity is signal. Is that bet correct?

**Taste gravity as curation.** The admin's taste shapes every surface through invisible gravitational pull. Readers can opt out (§16.5). Diversity budget prevents monothematism. Vanguard users amplify the effect. This is a bet that good curation is more valuable than democratic ranking. Is it?

**Author protection.** Authors control their work, their feedback preferences, their rating aggregates, their translation permissions, their remix policies. Readers control their data, their reading history, their pseud linkage, their opt-out dials. The spec is asymmetric in favor of authors. Is that the right balance?

---

## 29. User Configuration (§46)

Priority 1 extended userward: every user-visible behavior has a setting with a documented default, resolved through one hierarchy (context → pseud → account → instance) with visible provenance and reset at every level. Per-domain settings tables (never one JSONB blob), one API pattern, a domain-grouped /settings surface with client-side settings search in the command palette, server-enforced content filters, per-event notification routing, and portable export/import. The operator shapes the instance; each user shapes their experience of it.

**Growth vs. integrity.** Sharing loops (quote cards, fic trailers, cross-posting) turn readers into promoters. But growth can dilute culture. How does the spec prevent growth-harming-asymmetry between promotional tools and community-maintenance tools?

---

*This summary covers the complete spec (§0–§46, including the §45 category governance and §46 user configuration additions) and the taste-gravitational amendment. Cross-references like §16.17 point to the canonical docs. Last updated: 2026-09-23.*
