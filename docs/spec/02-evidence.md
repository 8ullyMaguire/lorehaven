# 2. Evidence base

**Status:** reserved
**Milestone:** M0
**Source:** the corresponding section of the Darkhold specification

All figures re-derived 2026-10-19 from the working trees and the local corpora.

## 2.1 The reader profile — Darkhold's own taste, measured

The owner's AO3 account is `Leather_Release_9057`. Confirmed: the 13 works he wrote are in
`~/mnt/thinkcentre/personal/literature/fanfiction/` (e.g. `01_work_80053971.txt`,
`the-proving-ground.epub`), and `generation.log` names his own `ao3/fanfic_generator.py`.

**54 works: 13 written, 41 bookmarked.** Measured from the 1.7 GB AO3 mirror
(`~/.hermes/profiles/sysadmin/cache/scratch/ao3_metadata.db`, 112,935 works · 634,231 tags ·
3,891,300 work_tags · 7,750,334 co-occurrence edges).

| dimension | measurement |
|---|---|
| **fandoms** | harry potter 19 · star wars 6 (+4 prequel) · highschool dxd 6 · percy jackson 3 · marvel 3 · original work 3 · worm 2 · solo leveling 2 · naruto 2 · gundam seed 1 · warhammer 40k 1 · jujutsu kaisen 1 · my hero academia 1 |
| **categories** | f/m 39 · f/f 12 · multi 11 · gen 5 · m/m 1 |
| **ratings** | explicit 32 · mature 12 · teen 1 · general 1 |
| **archive warnings** | creator chose not to use archive warnings 19 · graphic violence 15 · underage sex 12 · major character death 10 · rape/non-con 9 · no archive warnings apply 4 |
| **top freeform tags** | harems 13 · rough sex 12 · explicit sexual content 12 · canon divergence 11 · smut 10 · humiliation 9 · dom/sub 9 · dubious consent 8 · master/slave 8 · ai-generated text 8 · sexual slavery 7 · femdom 6 · master/pet 6 · incest 6 · bdsm 6 · chastity device 4 · mind control 4 · porn with plot 4 · gamer 4 · power dynamics 4 · system 3 · isekai 3 · waifu catalog 3 · brainwashing 3 · torture 3 · corruption 3 · anti-hero 3 · multiverse 3 |
| **favourite authors** | CambrianBeckett **4** · The_Dark_Wolf_Shiro 2 · Robs511 2 · Bronz_Deck 2 · PercyPendragon3 2 · then 31 authors with 1 each: esama, NidoranDuran, Illuviar, Shaboobamon, dirtyuncle, LostInTranscription, StevenStevenson9000, MayorHaggar, PotterSmut12, TheDirtyMind, Diegomas, RobWilsonWriting, WD_ONeill, marooon, Captain_Valkyrie, yungthiccking, aTasteofDarkness, StarWaves, Noodlehammer, Arctis_Shox, IsekaiTails, JollyHippopotamus, CarnalCharnal, RaccoonLeague, Evoxius, Accel14, Sir Lucifer Morningstar, Jesse_Desire, HarryPotterFanFicArchive_Archivist |
| **tag-family authors** | `waifu catalog - swiftrosenthal` 3 · `celestial grimoire - gothicjedi666` 1 · `celestial system cyoa - lord roustabout` present in his own writing |

**The 10,266-EPUB Questionable Questing archive** (1.5 GB) confirms the register by filename
keyword: anime/manga 1,723 · **villain/evil 1,104** · **gamer/system 997** · **power/progression
994** · **femdom/power 926** · military/war 676 · smut/explicit 451 · dungeon/quest 425 ·
romance 354 · **isekai/otherworld 352** · horror/survival 257.

**And the Machiavelli reading list** (`literature/machiavelian_reading_list.txt`, 50 books: Greene,
Machiavelli, Cialdini, Sun Tzu) — the non-fiction register is *power, strategy, influence and
self-mastery*. It is the same taste expressed in another medium, and it is the reason "the dark
side of fanfiction" is the right tagline and not a joke.

## 2.2 The six first-class themes, and the evidence for each

The owner selected six theme clusters. Each is a **named, browsable, subcribable group**, not a
loose tag. The measurement is how many of the 54 works carry at least one tag from the cluster.

| theme | works | % | the tags that carry it |
|---|---:|---:|---|
| **Dominion / Ownership** | **20** | 37% | humiliation 9 · dom/sub 9 · master/slave 8 · sexual slavery 7 · femdom 6 · master/pet 6 · possession · ownership · chastity device 4 |
| **Power Fantasy / Progression** | **14** | 25% | powerful harry potter 2 · bamf uzumaki naruto 2 · overpowered harry potter 2 · training 2 · overpowered MC · level-up · kingdom building · army building |
| **Dark / Evil Protagonist** | **13** | 24% | anti-hero 3 · dark harry potter 2 · morally grey 2 · alternate universe – dark 2 · crazy MC · dark grey MC · power hungry · villain protagonist · amorality |
| **Isekai / Otherworld** | **13** | 24% | self-insert 4 · multiverse 3 · isekai and transmigration 3 · isekai 2 · crossover 2 · alternate universe – harem |
| **Corruption / Mindbreak** | **12** | 22% | master/slave 8 · sexual slavery 7 · master/pet 6 · mind control 4 · brainwashing 3 · slavery 3 · corruption · non-consensual mind control |
| **Gamer / System** | **11** | 20% | gamer 4 · system 3 · video game mechanics 2 · gamer system 2 · gamer harry potter 2 · celestial grimoire 2 |

**The clusters are not equal in size, and the seed strings are too narrow to browse.** Measured
against the mirror, the works carrying at least one *literal* seed tag:

| theme | works | % of catalogue | + 24 PMI neighbours (cooc ≥ 10, works ≥ 50) | % of catalogue |
|---|---:|---:|---:|---:|
| Gamer / System | 611 | 0.5 % | **10,567** | 9 % |
| Power Fantasy / Progression | 2,383 | 2 % | 4,365 | 4 % |
| Dark / Evil Protagonist | 2,668 | 2 % | 5,304 | 4 % |
| Dominion / Ownership | 12,946 | 11 % | 12,268 | 10 % |
| Isekai / Otherworld | 8,918 | 7 % | 12,866 | 11 % |
| Corruption / Mindbreak | 5,262 | 4 % | 6,716 | 5 % |
| **union of all six** | **25,235** | **22 %** | — | — |

**A theme's tag set is its owner's seed plus its PMI neighbours, not a literal string list.** The
seed is the owner's editorial choice and is protected; the expansion is mechanical, derived from
the co-occurrence graph, and is what makes the cluster browsable and worth voting on. The
expansion recipe is kindred's, and the order matters:

- **Rank by PMI, not by co-occurrence count.** Ranking by raw count returns ubiquitous tags —
  `f/m`, `harry potter`, `gen`, `multi` — whose PMI is *negative*, meaning they co-occur
  **less** than chance predicts. Ranking the same seeds by co-occurrence and expanding to the
  top 24 puts those tags in every theme, and the clusters swell to 38–99 % of the catalogue.
- **Apply a support floor first.** Raw PMI is unbounded at small counts, and the top PMI
  neighbours of the gamer seeds are three-cooccurrence tags on one work. The floor is
  `cooccur_count ≥ 10` and `works ≥ 50`.
- **With both applied**, the neighbours are semantically right: `chaos gacha`, `overpowered
  harry potter`, `the gamer ability`, `litrpg`, `dark lord harry potter`, `maledom/femsub`,
  `cuckoldry`. And the clusters land in a **3–11 % band** instead of 0.5–11 %.

**Coverage: 41 of 54 works (76%) carry at least one selected theme.** The 13 uncovered are the
domestic and survival registers — *Business*, *The Strange Case of Love*, *Neglect and Attention*,
*Value loyalty above all else*, *Psycho of the Dead*, *A Shadow's Requiem*, *Pink Drinks*. They
are **not** discarded; they live under `freeform` and remain reachable, which is exactly what
"themes are a browsable overlay, not a filter" (§11.7) means.

**Harem / Collection was not selected as a theme** even though `harems` is his single most common
freeform tag (13/54). It lives *inside* Gamer/System and Isekai/Otherworld, because the "waifu
catalog" mechanic is a system, not a subject. Recorded here so the omission is a decision and not
an oversight.

**The overlapping tags, and the rule that settles them.** *Dominion / Ownership* and *Corruption /
Mindbreak* share `master/slave` (8/54), `master/pet` (6/54) and `sexual slavery` (7/54); *Gamer /
System* and *Isekai / Otherworld* share `self-insert` and `multiverse`. The rule is:

> **A tag's primary theme is the theme whose seed claims it. A tag may be secondary in every other
> theme, and the secondary is derived — never voted.**

The assignment follows the tags' own meaning, and the two overlaps resolve in opposite
directions:

- **`master/slave`, `master/pet`, `sexual slavery` → primary: Dominion / Ownership.** These name a
  *relationship of possession*. Corruption / Mindbreak is about the **arc** — the protagonist
  losing themselves — and it holds these three as *secondary* tags, so a work carrying only
  `master/slave` still lands in Dominion and a work carrying `master/slave` **and**
  `mind control` / `brainwashing` / `mind break` / `corruption` lands in both, weighted toward
  Corruption by the arc tags.
- **`self-insert`, `multiverse` → primary: Isekai / Otherworld.** They name the *setting
  mechanism*, not the register. Gamer / System holds them as secondary, because a gamer fic with
  a self-insert is an isekai that happens to have a system.

**A work's theme membership is therefore derived, not asserted**, and a work with no seed tag in
any theme is a work in no theme — which is a legitimate state, and the reason 13 of the 54 are
outside the six (see §2.2's coverage note).

## 2.3 lorehaven — `~/code-local/rust/lorehaven` @ `b40d92b`

| | measured |
|---|---|
| requirement ledger | **703 rows**, 48 areas — 312 `implemented-fully-tested`, 240 `implemented-locally-tested`, 120 `implemented-verified-e2e`, 26 `planned`, 4 `unsupported`, 1 `evaluated-and-rejected` |
| schema | **112 migrations per dialect** (SQLite + PostgreSQL), identical filenames, parity-tested |
| HTTP surface | 75 route modules |
| ingestion | 12 native source adapters (ao3, chyoa, efiction, ffnet, ficbook, pawchive, royalroad, scribblehub, syosetu, wattpad, xenforo) |
| code shape | 7-crate Rust workspace: `domain` (no I/O) · `db` · `app` · `scrapers` · `decisions` · `lore-metadata` · `test-support`; Svelte 5 + TS frontend embedded in the binary |
| stack | Rust 1.82+ / Axum 0.8 / SQLx 0.8 hand-written SQL / SQLite **and** PostgreSQL both first-class / no ORM / no Elasticsearch (ADR 0026) |
| test state (2026-10-05) | SQLite workspace 4160 pass / 0 fail (210 suites) · PostgreSQL 4126 / 3 · clippy 0 warnings · fmt clean · vitest 543/543 (75 files) · Playwright 126/126 |
| strengths | the requirement ledger; evidence-not-status; the two-engine gate; known-gaps pinned by tests; the 404-not-403 doctrine; schema-level privacy; the config contract |
| weaknesses | the spec monolith; 360 of 703 rows below the full-stack bar; one-sided route guards (a declared router was never mounted — 5 endpoints 404'd for the feature's whole life); unguarded SQL string interpolation; non-atomic money paths; "a 500 is not an empty state" |

## 2.4 ficnexus — `~/code-local/rust/ficnexus` @ `c3a67cb`

| | measured |
|---|---|
| code size | ~110,400 LOC Rust; 486 route registrations, 442 distinct paths |
| schema | 45 migration files; `001_initial.sql` alone carries **138 tables** |
| ingestion | **107 FanFicFare-parity sites** across 56 adapter modules (config-driven families: 19 eFiction + 26 eFiction-variant + 8 XenForo + 3 StoriesOnline + OTW siblings + bespoke) |
| stack | Rust / Axum 0.8 / SQLx 0.9 / PostgreSQL 16 + pgvector / Redis (Lua token bucket) / Ollama / SvelteKit 5 SPA / 11 binaries |
| what lorehaven lacks | 107-site FFF parity; body full-text quote search with `<mark>` snippets; `main_char_attr`; OPDS catalog; 7+ export formats; the 10-strategy recommender registry with RRF fusion; the `qa/api-walk.js` router-parsing audit |
| test state | unit 935 pass / 0 fail · **DB-gated: 39 of 57 suites pass, 18 fail** |
| weaknesses | 8 parallel authority columns on `users` + 3 XP ledgers; `tag_types` and `locales` shipped empty on every migration-built DB; the host cannot reach AO3 (404 + challenge) or FFN (403 Cloudflare); `works` dedupes on title+author only; a doc claims 15 languages, the code has 6 |

## 2.5 kindred — `~/code-local/go/kindred` @ `6db3bfd`

| | measured |
|---|---|
| code size | 62,410 LOC Go, 177 files (102 test files), 23 `internal/` packages, 24 server-rendered templates, **zero JavaScript** |
| binary | static, CGO-free, **14 MB**; target host = Raspberry Pi with 512 MB free |
| memory budget (measured) | `serve` full **184–186 MiB** against a 220 MiB cap · `serve` lite **25–26 MiB** against a 60 MiB cap · `ingest` **181.6 MB** in 1 m 31 s · `embed` (dim 32) **203.2 MB** in 3 m 10 s · `recommend` (n=5) 15 MB, 125 ms |
| index | CSR tag graph 26.8 MB after capping adjacency during construction (vs a 118.3 MB unpruned estimate) · 32-dim spectral embeddings ≈ 80 MB · co-bookmark index ≈ 23 MB over 12,812 pairs |
| strengths | the memory discipline (`VmHWM` gate, `make budget`, `/healthz` 503 when over cap); the CSR representation decision that cut 3.0 GB → 124 MB with no algorithm change; "a signal added only when it has data is a signal absent without saying so"; Glicko-2 with draws; the weighted-influence contract; the k-anonymity snapshot with **0 of 6,261 usernames published** |
| weaknesses | the arena was inert in production and silent about it; `ParseSeed` truncated `?seed=ao3_work:12abc` to work 12, so the ranking was computed for an entity the request never named; 3 delta-snapshot bugs that every unit test passed because they were in the writer; a stale metadata row the code deliberately refuses to read |

## 2.6 What each is trusted for

| source | trusted for | **not** trusted for |
|---|---|---|
| lorehaven | the ledger discipline, the domain model, the governance design, the reading experience, the config contract, the 404-not-403 doctrine, the two-engine gate | its completeness claims (360 rows below the bar); its spec as a structure to copy (it is a monolith) |
| ficnexus | the adapter architecture and the 107-site list; the works↔sources split; body full-text search; `main_char_attr`; the RRF registry pattern; the router-parsing QA audit | its data model; its trust/progression split (it documents the mess itself); its test state |
| kindred | the memory discipline; the recommender and arena algorithms; the snapshot privacy model; the degradation-honesty rule; the "measure, don't estimate" rule | its Go implementation — a reference implementation, not a port target (§8.3) |

---
