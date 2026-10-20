# 11. Search and discovery

**Status:** reserved
**Milestone:** M7
**Source:** the corresponding section of the Darkhold specification

## 11.1 The engine

- **SQLite:** FTS5 plus a trigram table for fuzzy matching.
- **PostgreSQL:** native full-text search plus `pg_trgm`.
- **Optional:** `pgvector` for semantic similarity, behind the same query planner.
- **No third engine** (D-4).

**One query language across every surface**, and the query string is a shareable URL.

## 11.2 The query language

Boolean `AND`/`OR`/`NOT`, quoted phrases, fielded terms (`title:`, `author:`, `tag:`, `fandom:`,
`character:`, `ship:`, `theme:`), parentheses, and the two AO3-grade semantics:

- **`main_char_attr`** — the highest-scored character tag is X **and** the freeform attribute Y is
  present. The scoring model: the first-listed character scores 10, others 1; the first-listed
  ship scores 5, others 1.
- **Character attribute predicates** — `char_attr:BAMF`, `char_attr:!vampire`, `char_attr:>5`.

Plus typo tolerance, tag filters, and numeric bounds on kudos, comments, words and chapters.

## 11.3 Body full-text

Where the retention policy is `cache`, the reader can search **inside** the text of the works they
can read: "fics where X says Y", with `<mark>` snippets. The index is maintained at body-write
time and backfilled by a bin.

## 11.4 Taxonomy and theme browse

Every canonical entity is browsable, queryable and linkable. `theme:` is a first-class field, so a
theme is a search dimension as well as a browse dimension.

**The closure table** makes `+children` expansion a single join, and every browse surface returns
the **same** `MediaRecord` shape the API returns (lorehaven ADR 0019: "if the UI can show it, the
API can return it").

## 11.5 The "more like this" ladder

1. **Next in series** — the explicit relation.
2. **Same author, other works.**
3. **Theme neighbours** — the works sharing the most-weighted tags of this work's themes.
4. **Readers also bookmarked** — item-item co-bookmark similarity.
5. **Taste-matched** — the reader's own profile, where one exists.

Each row carries its reason. **A recommendation without a reason is an advertisement.**

## 11.6 Trending and half-life

- **Trending** is time-decayed, on tags and on works.
- **Half-life** is how long a work keeps being read after publication, in basis points, recomputed
  nightly. It is a **ranking input only** — never a public number, never a badge, never a thing a
  reader is asked to optimise.

## 11.7 The sort contract

One `sort` vocabulary across every browse surface:

```
for-you | new | updated | top | trending | best-match | az
```

**`for-you` is a permutation, never a filter.** The set of reachable works under `for-you` is
*identical* to the set under every other sort; only the order differs. This is **asserted by
test** — set equality, not eyeballing. A taste-skewed default that removed results would be a
shadowban applied by ranking instead of by moderation.

**Themes and moods obey the same contract** (N-16): a theme or a mood reweights; it never makes a
work unreachable. The one exception is a *reader's own explicit filter*, which is a deliberate,
visible, reversible choice by that reader and not a default.

---

## 11.8 The inferred fandom — a candidate filter, never a classification

**The problem, measured.** Only **51 of 112,935** works carry a `fandoms` tag; 44 `characters`;
39 `relationships`. The freeform namespace carries 3,890,504 rows over 112,481 works. So the
filed namespaces are nearly empty and **the freeform names are the taxonomy**.

**The consequence, and it is not cosmetic.** The per-fandom diversity cap is the mechanism that
prevents the engine's documented monocrop — *"offline output is fandom-monocropped (26/30 Harry
Potter) even with `--max-per-fandom`, because the offline cap is a known no-op and the seed set is
HP-dominated."* **A cap that cannot group Harry Potter does not prevent an HP monocrop.** And the
shape heuristic kindred uses — the media-qualifier suffixes — does **not** group it:

| name | works | recognised by the shape heuristic? |
|---|---:|---|
| `harry potter - j. k. rowling` | **19,694** | **no** |
| `harry potter` | 15,672 | no |
| `harry potter - all media types` | 12 | yes |

So the largest fandom in the corpus — and the owner's own dominant one, 19 of his 54 works — was
**uncapped**.

### 11.8.1 The rule

A tag is a **candidate fandom** when its name has a fandom-qualifier shape and its base is not a
known AO3 relationship or category tag. Three parts:

1. **The shapes** (kindred's, extended): `X - all media types`, `X - fandom`, `X (fandom)`, and
   `X (<media>)` for the 22 media qualifiers — anime, manga, visual novel, video game, book,
   movie, tv show, cartoon, comics, play, podcast, doujin, band, musical, novel, and the combined
   forms.
2. **Plus the author-qualified form** `X - <qualifier>`, which is AO3's older fandom naming:
   `harry potter - j. k. rowling`, `parahumans series - wildbow`,
   `percy jackson and the olympians - rick riordan`, `a song of ice and fire - george r. r. martin`.
3. **Minus a closed list of bases** that are AO3 relationship or category tags rather than
   fandoms: `alternate universe`, `threesome`, `foursome`, `post-traumatic stress disorder`,
   `underage sex`, `oc`, `abandoned work`, `creator chose not to use archive warnings`,
   `no archive warnings apply`. **The blocklist is on the BASE, not on the qualifier** —
   `alternate universe` alone has **44+ distinct qualifiers** in the corpus (`- royalty`,
   `- muggle`, `- dark`, `- magic`, …) and no qualifier list can be complete.

**The canonical key** is the base, lowercased, with AO3's `& related fandoms` / `& related`
suffix stripped. **A base containing `/` or `&` is a relationship tag, not a fandom** and is
excluded.

### 11.8.2 What it produces, measured

| | before (shape only) | after (the full rule) |
|---|---:|---:|
| candidate fandom tags | 3,258 | **18,654** |
| canonical groups | 3,044 | **16,597** |
| works carrying one | 52,033 (46 %) | **82,608 (73 %)** |
| groups with ≥ 50 works | 179 | **264** |
| **the `harry potter` group** | **9 groups, largest 19,714** | **1 group, 19,880** |

The top canonical groups: `harry potter` 19,880 · `boku no hero academia` 5,050 · `batman` 4,234
· `a song of ice and fire` 3,277 · `game of thrones` 2,812 · `naruto` 2,345 ·
`percy jackson and the olympians` 2,332 · `parahumans series` 2,041 · `star wars` 2,018 ·
`spider-man` 1,579 · `pocket monsters | pokemon` 1,526.

**The monocrop is now preventable**, because the group that dominates — 19,880 works, 18 % of the
corpus — is the one the cap acts on.

### 11.8.3 The doctrine that keeps this safe

kindred's own docstring is the rule, and it is quoted here because it is the correct shape:

> *"this is a CANDIDATE FILTER, never a classification to be trusted. A guessed list used as input
> to a relationship graph invents relationships. The same list used to *narrow* a set, with counts
> attached to every row, stays checkable — you can see what it matched and argue with it."*

So:

- **A fandom group is used to NARROW, never to filter.** It groups the diversity cap and nothing
  else. No work is ever removed because of its inferred fandom (N-16).
- **Every group carries its count**, and the group key is shown in the recommendation's reason
  string, so a reader can see what it matched and argue with it.
- **A work with no candidate fandom is not capped.** Grouping every unrecognised work into a shared
  `unknown` bucket *"would silently cap every non-fandom work in the corpus at the same number, and
  that IS the monocrop, just wearing a different label."*
- **The base blocklist is curated data, not code** — a versioned file with a test, because AO3 adds
  fixed phrases and a hand-written list is a thing that rots. The `alternate universe` collision is
  the evidence: 44+ qualifiers, and no qualifier list can be complete.
- **The heuristic is measured, not assumed.** A change to the blocklist changes the group counts,
  and the counts are asserted, so a careless edit that halves the fandom coverage fails the gate
  rather than quietly disabling the cap.

---
