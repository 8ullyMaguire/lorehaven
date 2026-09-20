# Instance Topics — single-page design premise

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

## 0.4 Instance Topics (configurable, NEW)

An instance **may** declare the topics it is about — but is never required to.
An instance whose focus is a kneed not say so.

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

## 5. Gamification (spec §9.7) — **now topic-aware**

- **Credits** for reading, reacting, reviewing, bookmarking, importing,
  translating, voting. Daily and monthly caps.
- **Topic bonuses** (NEW): an instance may award extra credits for finishing
  works in declared topics; public topics are named, private ones silent.
- **Quality multiplier** (1.0×–1.8×) from completion rate, positive feedback,
  rereads, bookmarks.
- **Demand multiplier** (silent, 1.0×–1.5×) folds in admin taste — **never
  disclosed** in any label or breakdown.
- **Leaderboards:** daily/weekly/monthly windows, top-20 per category, opt-out
  available. Plus per-public-topic boards when topics are configured.
- **Badges:** milestone (one-time), recurring (×N), seasonal. Display is
  opt-in; no badge gates any feature.
- **Streaks:** reading and writing, private by default, reset without guilt.
  Streak freeze for 5 credits.
- **Anti-gaming:** time-on-page, alt-account isolation, pseud isolation,
  unique-source import credits, quorum dedup, min-reader thresholds.
- **Reading/writing goals:** private, opt-in, grant nothing.

---

## 6. Community & governance (spec §14, §17, §18, §19)

- Comments through positivity gate; forums (categories, topics, replies);
  groups (open/closed/hidden); 1:1 messaging (block-aware); presence (SSE).
- **Trust levels** TL0–TL6 from behavior records only — never purchase.
- **Reports → quorum review → sanctions** (rate-limit → shadow → suspend).
- Appeals, independence rules, audit log. Moderator identities hidden from
  reported accounts.

---

## 7. Economy (spec §20, §21)

- **Double-entry ledger** with idempotency keys, balanced transactions.
- **Quote → reserve → submit → complete → capture** flow for priced jobs.
- **Fair queues:** priority class + first-come within class; position observable.
- **Bounties, subscriptions, marketplace** (listings, commissions, gallery).
- **Negative invariant:** credits/subscriptions/bounties never write to
  trust_levels, operator_role, ranking weights. Tested, not assumed.

---

## 8. Imports, exports, interop (spec §6, §7, §13, §23)

- Source adapters with hand-off transport (no adapter-owned client), SSRF-safe
  fetcher, `robots.txt` compliance (operator override visible in catalogue).
- **Work body retention:** `cache` (default) or `aggregate` (metadata only).
- Exports: text, HTML, Markdown, EPUB, PDF, AZW3, MOBI. PWA offline reading.
- Public API with scoped tokens; RSS/Atom/OPDS per query; JSON-LD + Dublin Core.
- Federation: ActivityPub announce/notify (queries do **not** federate).

---

## 9. Translation & media (spec §22, §23, §25, §26)

- Translation jobs with segmentation, memory, glossary, review gates.
- TTS narration editions (Piper local-first, cloud adapters behind trait).
- **Derivatives:** request → build (OCR/transcode/TTS) — program on
  `DerivativeKind`, door reads one declaration.
- Archive mode: public-domain collections, optional controlled digital lending.

---

## 10. Spec-only, no code yet (spec §33, §34)

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
