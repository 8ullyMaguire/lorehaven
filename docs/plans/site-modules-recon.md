# Site-modules recon — could Lorehaven be a drop-in replacement for the popular reading/writing sites?

Written 2026-09-15 from the requirements ledger (`docs/requirements.csv`),
not from memory. Question under decision: turn Lorehaven into an
"everything app" usable as a drop-in replacement for ~25 external platforms?

## Verdict

**No to "drop-in replacement for any of those sites"; no to 19 per-site
configuration profiles. Yes to naming the handful of modules the list
genuinely adds on top of what the spec already covers — most of them are
already ledger rows.**

Reasons against the everything-app framing:

1. "Drop-in replacement" is a community-migration promise (URL schemes,
   account imports, API fidelity), and it must be kept per site. That is 19
   independent fidelity targets re-tested on every feature. The repository
   is still paying down a single dialect boundary (48 PostgreSQL test
   failures outstanding in `~/.hermes/plans/2026-09-15-lorehaven-pg-parity.md`);
   multiplying the matrix by 19 would freeze feature work.
2. Some list members are product theses, not features. Wattpad's
   algorithmic trending, KakaoPage's wait-or-pay, and Pixiv/FANBOX's
   subscription-gated access change who the platform serves and how it
   earns. A flag cannot contain a business model; adopting them is forking
   Lorehaven into a different product.
3. The codebase already answered the compat question: **M18-02
   (spec §23.11)** — compatibility surfaces, should an instance ever need
   one — is `unsupported` by design: own URL prefix, provenance-marked,
   contract-pinned by test, deletable. Any "act like Wattpad's API" wish
   belongs to that row, not to a per-site profile system.
4. Ingest/preservation is a different program. ArchiveBox/Kiwix/Internet
   Archive land on **FicNexus** (scrapers + revision cache + preservation
   batches M6-10, aggregating-instance mode M6-15), not on the
   publish/read/community core. Lorehaven instances can choose to be
   aggregating (metadata + canonical links, no bodies) — that row already
   frames the capture module.

## The map: each site → where it already lives, or why not

| Site | Module claimed | Verdict in Lorehaven |
|---|---|---|
| AO3 | archive, tags, warnings, collections | Already the model: M3 works/chapters, M10 taxonomy (fandom/ship/character/tag/warning/mood), M13 collections, M7 exports |
| FanFiction.net | threaded per-chapter reviews, beta roles, per-instance adult-content policy | M12 comments + forums, M3-07 `beta_reader` contributor role, M2-07/08 age-policy + content-eligibility service (the policy toggle FFN expresses by site policy is here an instance configuration) |
| SquidgeWorld | AO3 fork as separate instance | Not a feature — it is what Lorehaven already is: one codebase, per-instance policy/data. Nothing to add |
| Pixiv / FANBOX | illustrated fiction, creator subscription tiers | Illustrations need media attachments on chapters (editor schema M3-04 is bounded prose) — **proposed module**. Subscription gating conflicts with the free-archive premise; M21's per-work pricing + subscriptions (M21-06) is the compatible slice |
| Royal Road | chapter-release scheduling, chapter comments, follow/favorites | Works/chapters/serial reading exist (M3/M4); follow = M8-01 update checks; chapter comments = M12. **Missing: scheduled chapter release (drip-publish) — proposed module** |
| Scribble Hub | multi-instance cross-posting | Supported direction-wise: M6 adapters import *from* sources (royalroad, ao3, …), M18 federation + RSS/Atom feeds push out. One-work-many-archives publishing is a workflow idea, not new storage |
| Wattpad | shared reading lists, inline paragraph comments, votes, trending | Reading lists = M8 shelves; inline paragraph comments and vote/like mechanics = **proposed module** (cheap; anchors exist from M4-04); algorithmic trending deliberately not adopted (M11's discovery is diversity-governed, operator-influenced, privacy-first — a trending hack would contradict M11-02/03 design) |
| FictionPress | on-site drafts with analytics, feedback-first community | Drafts = M3; feedback-first = the M9 positivity layer, which goes further (held critique, receipts) |
| Commaful | bite-sized, visual short-form | Reader typography controls exist (M4-08). Visual page-turn reading = **proposed module**, low priority |
| Booksie | low-friction publishing, catalog browsing | Already lower friction than AO3 (no invites, M3 publish flow) |
| Hardcover | public GraphQL API, per-book privacy, half-star ratings | API = M18 public API (REST + scopes; GraphQL not adopted — one query language is enough); per-object privacy is everywhere (M4/M8); half-star = aggregate-method change in M4-06, **proposed tweak** |
| Fable | book clubs, synchronized schedules, marginalia | Clubs ≈ M12 groups + M13 events; synchronized reading schedules = **proposed module**; marginalia = M4-11 notes made shareable inside a club — **proposed module** (privacy design needed: notes are private by default) |
| StoryGraph | mood/pace tags, content warnings, reading stats | Mood = M10-06; warnings = M10 taxonomy; **missing: reading-stats visualizations (M8/M4 data exists, no dashboard) — proposed module** |
| ArchiveBox | external content capture | FicNexus's job (scrapers, M6 revision cache); Lorehaven's M6-15 aggregating-instance mode is the policy side. No duplicate in Lorehaven |
| Kiwix | offline ZIM serving | M7 PWA covers offline reading of fetched content; ZIM export = **proposed module**, niche, cheap-ish exporter target behind M7 |
| WebNovel (Qidian Int'l) | chapter-unlock monetization | M21 pays for whole works today; **chapter-level gating (unlock or subscription-early-access) = proposed module** — it reuses entitlements/ledger, adds per-chapter price resolution on the read path |
| KakaoPage | wait-or-pay unlocks | Not adopted — dark-pattern monetization contradicts the platform's stance; the fair-queue/credit system (M15) is the ethos-compatible alternative |
| Tapas | season-style series organization, ink-locked chapters | Series/seasons = works/chapters + collections (M13) suffice; ink-lock = same as WebNovel chapter gating module |
| DeviantArt | multi-format literature (poetry, prose, scripts) | M3-04's bounded prose schema is the blocker; poetry/script node kinds = **proposed module**, significant document-schema work; M16 gallery items exist for non-prose artifacts |
| Medium | publications/collections, tag distribution | Collections (M13) + taxonomy distribution (M10) + feeds (M18) cover the substance |
| Goodreads / StoryGraph (from the earlier list) | tracking/cataloging | M8 library (shelves, statuses, private tags, saved views) + M4 ratings/history; the gap is the StoryGraph stats dashboard above |
| Questionable Questing / Literotica (from the earlier list) | niche archives with per-instance policies | Import-side coverage belongs to FicNexus adapters (M6 pattern: fixture + live-verified adapter per site); hosting-side nothing new — policy toggles cover the content-profile difference |
| Internet Archive | preservation | FicNexus preservation batches (M6-10, §14.5 permission basis) + M6-15. Lorehaven stays the reading surface |

## What adoption would actually look like (if you say go)

Not 19 site profiles — **one instance-configuration layer with ~6 flags**,
most of which already have a home in config/admin:

1. `monetization` (per-work pricing on/off instance-wide) — exists implicitly; make it explicit for operators who want a pure free archive.
2. `chapter_gating` (new M22: per-chapter unlock / early access for subscribers on top of M21 entitlements).
3. `scheduled_release` (new M23: drip-publish chapters; reuses M5 jobs).
4. `stats_dashboard` (new M24: reader stats visualizations over existing history/library rows; k-anonymity rules from M19 apply).
5. `inline_paragraph_comments` + `votes` (new M25; anchors exist, M12 carries enforcement).
6. `offline_export_zim` (new M26; exporter behind M7).

Rejected permanently: algorithmic trending, wait-or-pay, subscription-walled
archives, compatibility facades by default (M18-02 governs those).

## Order of business

These are proposals. None of it outranks the open ledger debt: the
PostgreSQL parity plan (48 failures), M21-04 processor flow, M21-07 alert
scheduler, M21-08 robots generator, M6-13/M6-15 carried-forward rows, and
M20's performance/accessibility/i18n bounds. If you adopt modules 2-6,
they become M22-M26 rows in `docs/requirements.csv` and get milestone plans
like the others; flags 1 is configuration plumbing inside that work.
