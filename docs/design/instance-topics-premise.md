# Instance Topics & Taste Gravity — single-page design premise

*A one-page compression of the Lorehaven spec for brainstorming with a chatbot.
The full, normative spec lives in `docs/spec.md` (§0–§34); this is a map, not the
territory. Every section number below points to the authoritative section.*

---

## 0. What Lorehaven is (spec §0)

A **self-hosted, self-governing fanfiction platform** that keeps authors writing
through a positivity-first feedback culture, grows the body of available fiction
through frictionless importing, and governs itself through trust levels and
quorum — so the operator curates *taste* instead of policing behavior.

**Priority stack** (lower number wins when goals conflict):
1. Fully customizable site · 2. Maximize high-quality fiction ·
3. Positivity-first feedback (critique is opt-in) ·
4. Admin-aligned but never monothematic · 5. Trust-level self-governance ·
6. Quorum moderation · 7. Translate everything · 8. Revenue.

**Foundational protections** (yield to nothing): private libraries, drafts,
history, source credentials, pseud linkage. No purchased trust, ranking, or
moderation. No ads or trackers. AI crawlers refused by default. Admin taste
invisible. Honest verification only.

---

## 0.4 Instance Topics & Taste Profile (configurable, NEW)

An instance **may** declare the topics it is about — but is never required to.
An instance whose focus is a kink or sensitive subject does not have to say so.

Each topic has three properties:
- **`name`** — the operator's own label ("hurt/comfort", "omegaverse", …).
- **`public`** — `true` means the topic is shown publicly (landing page, meta
  API, leaderboard descriptions); `false` means it is used only internally
  for recommendations and credit bonuses, never named.
- **`bonus_credits`** — extra credits a reader earns for finishing a work in
  that topic (default 0).

An instance may declare **multiple topics**; a work may match several; the
bonus stacks per matching public topic. Private topics earn the same bonus
but are never surfaced. **No topics = topic-agnostic**; §9.7 categories
apply universally, no bonuses.

Config (`[site] topics = [...]`). Reported on `/api/v1/meta`.

Gamification effects of public topics: an extra completion-credit row
("Finish a work in a public topic") and a per-topic leaderboard category.
Anti-gaming: the bonus never applies to a work the reader imported or wrote
themselves.

### 0.4 Taste Profile & Gravity Engine (NEW — taste-gravitational system)

Beyond simple topics, Lorehaven supports a full **taste-gravitational system**
(`docs/spec-amendments/taste-gravitational-system.md`):

- **Multi-dimensional Taste Profile:** Admin defines dimensions (angst, pacing,
  prose density, canon compliance, trope preferences, …) each with a target
  and weight. The admin's taste is a position in this space.
- **Taste-Weighted Signals:** Every user action (kudos, bookmark, comment,
  completion) carries a signal weight proportional to that user's alignment
  with the instance taste profile. A bookmark from a 0.9-aligned user boosts
  a work's discovery score more than a bookmark from a 0.1-aligned user.
- **Taste Resonance Score:** Per-user alignment metric (0.0–1.0) computed
  from bookmark overlap, rating correlation, completion alignment, reading
  time ratio. Hidden from all users except as a qualitative label.
- **Signal Weight Mode:** Instance-configurable — `egalitarian` (all equal),
  `taste_weighted` (proportional to alignment), `admin_only`.
- **Anti-Echo-Chamber Valve:** X% of discovery slots reserved for taste-distant
  content. Prevents monoculture. Serves priority 4.

---

## 1. Architecture (spec §2)

- **Stack:** Rust/Axum, SQLx, Svelte 5, PostgreSQL or SQLite, Redis optional.
- **One instance per deployment** (multi-tenancy excluded by design).
- **Single-binary AGPL distribution**; frontend assets embedded.
- Caddy reverse proxy, systemd, local filesystem default.
- Optional: SMTP, Stripe, Ollama, ElevenLabs, ActivityPub, Redis.

---

## 2. Identity & privacy (spec §4, §7)

- **Accounts** share credentials, wallet, trust, invites.
- **Pseuds** are a writer's public faces — each has separate works, follows,
  messages, reading history, recommendations, dashboard.
- **Age policy:** unknown → declared_minor → declared_adult; no unrestricted
  checkbox. Spain-based operator context noted but not settled.
- **Content eligibility:** one shared function (`can_access_content`) applies
  to every reader, search, feed, notification, recommendation, API, bot,
  translation, and public stat.

---

## 3. The content model (spec §8, §15, §30, §32)

- **Works** have lifecycle (draft/scheduled/published/withdrawn/deleted),
  visibility (public/unlisted/restricted), completion state.
- **Chapters** carry revisions; optimistic concurrency via `version`.
- **Media** is generalized (§32): works, units, creators, distributors,
  collections — one queryable graph. Formats: prose, poetry, essay, book,
  fanwork, podfic, audiobook, video, comic, zine scan, …
- **Taxonomy:** fandoms, ships, characters, tags, moods, content notes —
  canonicalized via quorum, aliasable, mergeable.

---

## 4. Reading & feedback (spec §9, §12)

- Reader: progress sync, typography settings, spoiler reveal, history, goals,
  year-in-review, TTS.
- **Positivity pipeline:** every comment/review is classified before storage
  (positive / constructive / ambiguous / hostile). Hostile is never stored or
  shown. Constructive reaches only opted-in authors.
- Author tools: per-work comment policy, held-feedback queue, comment throttle,
  cooling-off batch mode, pause comments.

---

## 5. Gamification (spec §9.7) — **taste-aware**

- **Credits** for reading, reacting, reviewing, bookmarking, importing,
  translating, voting. Daily and monthly caps.
- **Topic bonuses**: an instance may award extra credits for finishing
  works in declared topics; public topics are named, private ones silent.
- **Quality multiplier** (1.0×–1.8×) from completion rate, positive feedback,
  rereads, bookmarks.
- **Taste-Weighted Signals** (NEW): the demand multiplier is replaced by a
  general taste-weighted signal system. Every user action carries a signal
  weight equal to their resonance score. Aligned users become amplifiers —
  their collective behavior shapes the recommendation algorithm even on fics
  the admin hasn't personally read. Config: `signal_weight_mode = egalitarian |
  taste_weighted | admin_only`.
- **Leaderboards:** daily/weekly/monthly windows, top-20 per category, opt-out
  available. Plus per-public-topic boards when topics are configured.
- **Badges:** milestone (one-time), recurring (×N), seasonal. Display is
  opt-in; no badge gates any feature. New: lifecycle badges (Completionist,
  Resurrectionist), Vanguard, Tastemaker, Herald, Ambassador.
- **Streaks:** reading and writing, private by default, reset without guilt.
  Streak freeze for 5 credits. **Flat milestone bonuses only** (e.g., +5 at
  7 days, +15 at 30 days) — never a persistent multiplier on per-action credits.
  (A multiplier amplifies noise as much as signal, diluting the taste-weighted
  economy.)
- **Anti-gaming:** time-on-page, alt-account isolation, pseud isolation,
  unique-source import credits, quorum dedup, min-reader thresholds. Plus
  resonance gaming protection (taste profile dimensions never exposed).

---

## 6. Community & governance (spec §14, §17, §18, §19)

- Comments through positivity gate; forums (categories, topics, replies);
  groups (open/closed/hidden); 1:1 messaging (block-aware); presence (SSE).
- **Trust levels** TL0–TL6 from behavior records only — never purchase.
- **Trust × Taste Coupling** (NEW): optional taste-alignment gate per level.
  Per-instance matrix. Democratic instances set all to 0.0. Curated boutiques
  set TL4+ high. Never disclosed to the user.
- **Taste Vanguard Role** (NEW): top-aligned users become curators. Selection
  method configurable (resonance_threshold, admin_appointment,
  community_election, contribution_volume, hybrid). Permissions: pin works,
  nominate, create clubs, trigger provisional bounties, post at reduced cost.
- **Reports → quorum review → sanctions** (rate-limit → shadow → suspend).
- Appeals, independence rules, audit log. Moderator identities hidden from
  reported accounts.

---

## 7. Economy (spec §20, §21)

- **Double-entry ledger** with idempotency keys, balanced transactions.
- **Quote → reserve → submit → complete → capture** flow for priced jobs.
- **Fair queues:** priority class + first-come within class; position observable.
- **Bounties** (NEW types): standard, crowdfunded, reverse, vanguard-provisional.
  Standing bounties auto-create when works match admin criteria. Vanguard
  provisional flow (3+ vanguard bookmarks → pending admin review, 14-day
  deadline, fail-closed).
- **Referral System** (NEW): taste-weighted tiers. Base payouts modest,
  taste-aligned bonuses generous. Anti-gaming: same-IP clusters flagged.
- **Negative invariant:** credits/subscriptions/bounties never write to
  trust_levels, operator_role, ranking weights. Tested, not assumed.

---

## 8. Discovery & engagement (NEW sections)

- **Fic Lifecycle Incentives** (§9.8): completion bonus (2× on final chapter
  of ≥3-chapter work), resurrection reward (3× on 180+ day stale work),
  WIP visibility neutral (no penalty, no boost).
- **Taste-Weighted Notifications** (§9.9): notify aligned users when matching
  content is published. Batching prevents spam. Max 3 per day. Popularity
  bypass for >0.9 popularity.
- **Onboarding Taste Quiz** (§0.4.2): optional quiz on registration. Computes
  initial taste vector for immediate resonance signal. Skippable.
- **Taste Probes** (§16.19): admin's discover surfaces works adjacent to but
  slightly outside their taste vector. Positive engagement expands profile;
  negative suppresses direction.
- **Dynamic Tag Gravity** (§0.4.6): tags gain/lose gravity based on admin
  engagement over 90 days. Admin can manually pin tags regardless.
- **Instance Presets** (§0.6): bundled config defaults (Curated Boutique,
  Open Library, Admin's Garden, Genre Haven, Experimental Lab).
- **Author Matchmaking** (§14.5): hint authors about understaffed areas of
  high instance demand. Hints, not requirements.
- **Reading Clubs** (§17.6): manual gravity override for spotlight works.
  Admin or vanguard creates, 1–14 days, flat participation bonus.

---

## 9. Imports, exports, interop (spec §6, §7, §13, §23)

- Source adapters with hand-off transport (no adapter-owned client), SSRF-safe
  fetcher, `robots.txt` compliance (operator override visible in catalogue).
- **Work body retention:** `cache` (default) or `aggregate` (metadata only).
- Exports: text, HTML, Markdown, EPUB, PDF, AZW3, MOBI. PWA offline reading.
- Public API with scoped tokens; RSS/Atom/OPDS per query; JSON-LD + Dublin Core.
- Federation: ActivityPub announce/notify (queries do **not** federate).

---

## 10. Translation & media (spec §22, §23, §25, §26)

- Translation jobs with segmentation, memory, glossary, review gates.
- TTS narration editions (Piper local-first, cloud adapters behind trait).
- **Derivatives:** request → build (OCR/transcode/TTS) — program on
  `DerivativeKind`, door reads one declaration.
- Archive mode: public-domain collections, optional controlled digital lending.

---

## 11. Spec-only, no code yet (spec §33, §34)

- **Milestone 27:** permission statements (podfic/translate/remix/AI),
  derivative lineage, exclusion registry.
- **Milestone 28:** trust-weighted ratings, anomaly detection, contested
  aggregate marking, quorum clearance.
- **Milestone 29:** "why am I seeing this" explanations, private attention
  report, trust-gated tag-wrangling queue.
- **Decision Services (§34):** pluggable calibrated classifier per task,
  deterministic fallback disclosed on `/api/v1/meta`. Never decides sanctions,
  money, trust.

---

## What "complete" means (spec §1, §25)

A feature is complete only when its migration, backend, permissions, frontend,
loading/empty/error/success states, tests, and documentation all exist and pass.
A screen with mock data is **not** a feature. Every claim in `docs/verification.md`
names the test that proves it.
