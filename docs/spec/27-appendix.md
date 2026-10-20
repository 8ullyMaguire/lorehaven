# 27. Appendix — the measured numbers this spec inherits

**Status:** reserved
**Milestone:** M0
**Source:** the corresponding section of the Darkhold specification

Every figure below was measured on real hardware against real data, and re-verified on 2026-10-19
where it was cheap to do so. **None is an estimate.**

**The reader profile (§2.1).** 54 works — 13 written, 41 bookmarked. 19 HP · 6 Star Wars · 6
Highschool DxD · 3 Percy Jackson · 3 Marvel · 2 Worm · 2 Solo Leveling · 2 Naruto. 39 f/m ·
12 f/f · 11 multi. 32 explicit · 12 mature. 19 "creator chose not to use archive warnings" · 15
graphic violence · 12 underage sex · 10 major character death · 9 rape/non-con. Top freeform:
harems 13, rough sex 12, canon divergence 11, smut 10, humiliation 9, dom/sub 9, dubious consent 8,
master/slave 8, ai-generated text 8, sexual slavery 7, femdom 6, master/pet 6, incest 6, bdsm 6.
Top authors: CambrianBeckett 4, then five at 2, then 31 at 1. The QQ archive of 10,266 EPUBs:
villain/evil 1,104, gamer/system 997, power/progression 994, femdom/power 926, isekai 352.

**lorehaven** (2026-10-05): 703 ledger rows across 48 areas; 312 fully tested, 240 locally tested,
120 verified e2e, 26 planned, 4 unsupported, 1 rejected. 112 migrations per dialect, identical
filenames. 75 route modules. 12 source adapters. SQLite workspace 4160 pass / 0 fail across 210
suites; PostgreSQL 4126 pass / 3 fail at the earlier commit, fixed at the later one; clippy 0
warnings; fmt clean; vitest 543/543 across 75 files; Playwright 126/126 in 10.4 minutes against the
release binary with the embedded bundle.

**ficnexus** (2026-09-26): 935 unit tests pass, 0 fail. **DB-gated: 39 of 57 suites pass, 18
fail** (329 pass / 39 fail). 486 route registrations, 442 distinct paths. 138 tables in the initial
migration. 107 FanFicFare-parity sites across 56 adapter modules. The numbers before 2026-09-26 are
void — they were measured against a 42-table-short database.

**kindred** (measured on thinkcentre, ARM64, against a 1.7 GB mirror): 112,935 works · 634,231 tags
· 3,891,300 work_tags · 7,750,334 co-occurrence edges over 123,047 distinct tags · 6,261 readers.
`ingest` 181.6 MB in 1 m 31 s; `embed` (dim 32) 203.2 MB in 3 m 10 s; `serve` full 184–186 MiB
against a 220 MiB cap; `serve` lite 25–26 MiB against 60 MiB; `recommend` (n=5) 15 MB, 125 ms. The
CSR graph is 26.8 MB after capping adjacency during construction, against a 118.3 MB unpruned
estimate. The embedding settles at float32 epsilon (1.19e-7) against a dense reference. A snapshot
over the real corpus: 4,065 affinity rows across 256 shards, **0 of 6,261 usernames published**, 0
work ids, 255 tag ids (shard index keys and two tags literally named "15" and "200"). The binary
is 14 MB; 24 templates; zero JavaScript.

**The four numbers that decided the architecture:**

1. kindred's tag graph was 3.0 GB as a Python dict-of-dicts and **124 MB as a CSR array in Go** — a
   24× cut from the representation alone, with no algorithm change. *→ §8.2's budget discipline.*
2. lorehaven's SQLite password-update no-op was **silent** and every test passed. *→ N-8, and
   §23.3's "the second engine is the cheapest available check on the first one".*
3. kindred's arena was **inert in production and silent about it**. *→ §12.7's degradation
   honesty.*
4. the AO3 recommender's offline output was **fandom-monocropped** — 26 of 30 Harry Potter —
   because the per-fandom cap was a no-op and the seed set was HP-dominated. *→ §9.8.2's
   measured per-fandom cap, and §12.7's house taste as a diversity floor.*

**The corpus, measured (2026-10-19).** 112,935 works in the mirror · 111,686 carry a title, an
author, a summary and at least one tag — the import set · 634,231 tags, of which **587,258
(97 %) are on exactly one work** and 56,197 clear a 5-work floor · 3,891,300 tag assertions over
112,481 works · **only 51 works carry a `fandoms` tag**, 44 `characters`, 39 `relationships`
— the freeform namespace *is* the taxonomy · ratings: 42,968 Explicit, 25,190 Mature, 27,562
Teen, 8,419 General, 8,796 Not Rated · 53,989 complete · 42 works with `word_count = 0`.

**The author namespace, measured** (AO3 mirror, 2026-10-19): `works.authors` is a **single
author string** on 112,934 of 112,935 works — exactly one work carries a multi-author string.
The strings are raw AO3 handles. The two largest are **reserved authors**, not people:
`orphan_account` **1,889** and `Anonymous` **1,650** — 3,539 works, **3.1 % of the corpus, with
no author at all**. The seed of the owner's expansion, **`Chastity`**, has **25** works in the
mirror. The string `chastity` is simultaneously an author on 25 works, a freeform tag on 36
works, and the base of `chastity device` (390 works) — **three things in three namespaces**, and
no query joins them by string equality.

**The six themes, measured.** Seed populations 611 / 2,383 / 2,668 / 12,946 / 8,918 / 5,262 works
(Gamer, Power Fantasy, Dark MC, Dominion, Isekai, Corruption) · **expanded with 24 PMI neighbours
at `cooc ≥ 10, works ≥ 50`: 10,567 / 4,365 / 5,304 / 12,268 / 12,866 / 6,716** · union of
the literal seeds 25,235 works (22 %) · the owner's 41 bookmarked works, all with a summary and
≥ 1 tag, are the held-out evaluation set.

---

*End of specification. Next artifacts, in order: the rename (M0), the ledger triage (§24.1), the
theme seed (§24.4).*

**The implementation plan that executes this specification is
`~/secondbrain/10-Projects/darkhold-implementation-plan.md`** (255 task cards, M0–M32). The
plan is the build order; this document remains the design. Where they differ, the plan carries
the numbers and this document carries the reasons.
