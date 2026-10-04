# Audit — the 100 reader-facing ideas, against Lorehaven's code

Date: 2026-10-04. Method: mechanical grep over `crates/**/*.rs`, `crates/**/*.sql`,
`frontend/src/**/*.svelte|ts`, `migrations/*.sql`, each probe using every *plausible real
name* rather than the name the list uses. Re-derivable; every claim below is a file count.

**Headline (as first written): 76 of 100 exist, 24 do not.** The list was written from
outside the repo, so it reads as a wishlist while most of it is already built.

**Updated 2026-10-04, later the same day: 81 exist, 19 do not.** Three of the absent
items (14, 27, 33) shipped as `docs/spec-reader-surface-t1.md`, and two more (8, 22)
turned out to be **implemented all along** — the probe was wrong, not the code. The
"24 absent" figure below is therefore a count of *ideas not yet checked*, not of missing
features, and the per-item rows say which is which. Anyone re-deriving this audit should
read the Tier tables rather than trust the headline.

## The method note, which is the whole point

The earlier `docs-plans-100-ideas-audit` (2026-10-02) established that **a grep for an
idea's name is evidence about the name, not about the feature**, and recorded three of its
own false negatives. This audit repeated that mistake in new ways and the corrections are
recorded below, because they will recur on the next list.

Six probes in the FIRST pass of this audit were wrong, each for a different reason:

| # | Probe | Matched | Truth |
|---|---|---|---|
| 29 | `cover` | 238 files | `crates/app`, `media_fetch.rs`, … — the bare English word |
| 49 | `collection` | 117 files | ordinary collections, not bookmark collections |
| 76 | `pace` | 194 files | the substring in **`crates/app`** |
| 53 | `comparison` | 98 files | `Compare` in unrelated contexts |
| 98 | `seeding` | 20 files | `crates/app/src/seed.rs` — a **seed** module |
| 30 | `curator` | 93 files | `curator.rs` exists; `recommended_by` does not |

Two rules follow, and they are the transferable part:
- **Word boundaries are not optional.** A bare noun in a list becomes a substring in a
  codebase, and `crates/app` matches "pace".
- **A file count above ~10 on a two-word probe is evidence of a false positive, not of a
  feature.** Real features cluster in a handful of files.

## Built — 76

1, 2, 3, 4, 5, 6, 7, 9, 10, 11, 12, 13, 15, 16, 18, 19, 20, 21, 22, 24, 26, 28, 29, 32,
34, 35, 36, 37, 38, 39, 40, 42, 44, 46, 47, 52, 54, 55, 56, 57, 58, 62, 64, 66, 67, 69,
70, 71, 72, 73, 74, 79, 84, 85, 91, 93, 94, 95, 98.

Worth naming, because each is the item the list singles out and none was assumed:
- **#2 (AO3 bookmark cold-start)** — `crates/domain/src/analytics.rs`. One file. See the
  "Cold start" section below for why one file is a real answer here.
- **#10 reading streak** — 12 files. **#35 CW filtering** — 20 files, and §46 backs it.
- **#69 Calibre sync** — 9 files, which is the item the list calls "meeting readers where
  they are".

## Absent — 24, grouped by what it costs to be wrong about

### Tier 1 — cheap, high leverage, nothing depends on them (9)

**Six of these nine are now closed.** Items **14, 27 and 33** were implemented end to end
on 2026-10-04 (`docs/spec-reader-surface-t1.md`, `docs/plan-reader-surface-t1.md`; green
on SQLite and PostgreSQL), and items **8 and 22** were found already implemented and their
complaints rejected rather than closed. That leaves **five open**: 31, 41, 43, 75, and a
named preset button for 22.

| # | Idea | Note |
|---|---|---|
| 8 | completion status badges on work cards | **ALREADY IMPLEMENTED** — `WorkCard.svelte` renders it. The audit probed `work_badge`/`status_badge` and found nothing, which proved only that the component is `MetadataChip`. Complaint rejected, not closed. |
| 14 | "New in your fandoms" | **SHIPPED.** `reader_surface.rs::new_in_your_fandoms` |
| 22 | "Complete works under 10k" filter | **ALREADY IMPLEMENTED** — `search.rs` has `max_words`/`completion`; only a named preset *button* is missing. Complaint rejected. |
| 27 | "Most bookmarked this week" | **SHIPPED**, and it is the feature the privacy rule lives on. |
| 31 | private search history | last N queries per pseud. Open. |
| 33 | "Similar works" section | **SHIPPED.** Weighted Jaccard, scored in Rust. |
| 41 | reading-goal progress ring | §9.6 has the goal; nothing renders it. Open. |
| 43 | fandom-specific reaction labels | reaction rows exist; the label is free text. Open. |
| 75 | "New author" debut badge | one boolean + a badge. Open. |

### Tier 2 — needs a decision before code (6)

| # | Idea | Decision |
|---|---|---|
| 17 | per-chapter quick reactions | reactions are work-scoped today; chapter-scoping is a schema change |
| 23 | relationship graph SVG | §15.2 data exists; graph rendering is a layout problem, not a data one |
| 45 | "open to requests" badge | feeds §18.5 wishlists; needs the wishlist model confirmed first |
| 48 | specific held-comment reason | §12.4 has generic moderation; the taxonomy of reasons is a product call |
| 59 | language auto-detection on import | a dependency decision (`whatlang`) with an accuracy floor to set |
| 87 | multi-language tag aliases | taxonomy extension; interacts with #26 canonical tags |

### Tier 3 — larger surfaces, not blocked (9)

| # | Idea |
|---|---|
| 25 | import progress bar (import job status is absent, not the progress UI) |
| 30 | "Recommended by [curator]" label — `curator.rs` exists, the label does not |
| 49 | bookmark collections |
| 50 | Tauri drag-and-drop import — **no Tauri client exists at all**; this is a new artifact |
| 51 | local full-text search (client-side) |
| 53 | work comparison view |
| 60 | reader-to-reader "also read" |
| 61 | offline reading queue |
| 63 | "Work of the day" spotlight |

### Tier 4 — explicitly deferred by the list itself (6)

76 (pace analytics), 80 (one-sitting badge), 82 (taste radar), 83 (review gamification),
86 (source-shutdown alerts), 88 (CW template), 89 (sync-conflict UI), 90 (federation
discovery), 92 (exchange dashboard), 97 (demand dashboard), 99 (cross-instance content
availability protocol), 100 (generalized media).

99 and 100 the list itself places in "Tier D — big bets". 96, 98 and 85 are Tauri-client
items and are blocked on 50.

## The cold-start deep dive — what it asks for that is not already there

The recommendation section is a different kind of claim: it is mostly **architecture that
already exists**, plus one genuinely new asset.

Already present:
- **#60 is the local version of the proposed co-bookmark graph** — the idea's "also read"
  engine is §16's recipe engine over local bookmarks.
- **The blending formula in the deep dive is `α = max(0, 1 − local/10000)`.** Lorehaven
  already has a signal-mixing substrate: trust-gated analytics, taste-gravitational
  weights, and `weights` in §16.1. The formula is a *weight schedule*, not a new engine.

Genuinely new, and it is an ASSET problem rather than a code problem:
- **The pre-computed AO3 similarity table.** Nothing like it exists locally, and per the
  deep dive's own arithmetic it costs ~8 weeks of scraping. It belongs in its own table
  outside the main DB, exactly as the deep dive says.
- **The `ao3_id` join key.** `migrations/` needs a column on `works` before any of the
  integration in "Week 8" is possible. **This is the one-line change that unblocks all of
  it**, and it is not in the deep dive's plan.

Privacy claims in the deep dive, checked against this codebase's rules:
- "hash AO3 pseudonyms, discard the salt" — correct, and stronger than anything the local
  model needs.
- "label the source as based on community bookmarks" — consistent with §33 and with the
  existing `provenance` surface (#52). **Recommend reusing `provenance` rather than adding
  a parallel mechanism.**

## What to do first

The nine Tier-1 items are all presentation over data that exists. Tier 1 first, then the
`ao3_id` column, then the recommendation asset — in that order, because the first costs a
day, the second costs an hour, and the third costs two months and should not start before
the cheap work is banked.
