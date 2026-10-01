---
title: "Handoff — Pawchive import repair: tags, authors, summaries, un-merged chapters"
date: 2026-09-25
status: complete
branch: fix/pawchive-tag-and-author-parsing
---

# Handoff — Pawchive import repair

Date: 2026-09-25. Repairs the damage from the 2026-09-19 session
(`20260919_210111_a6a6d7`), when ~70,000 Pawchive posts were imported.

## TL;DR

The import produced 10,071 works that had **no tags, no author names, and a
body dump where the abstract should be**. Two root causes, both in
`crates/scrapers/src/sites/pawchive.rs`. Fixed in the adapter, and the
existing rows repaired with an additive backfill. 23,096 additional works
were recovered from 40 works that had an author's entire archive merged
into one chapter.

Final state: **33,167 works, 123 real author names, ~190,000 tag links,
real prose summaries.**

## Root causes

### 1. Tags were never parsed

`tags` arrives as a *string* shaped like a set literal, and the form is
**mixed**: entries containing a space or comma are quoted, the rest are
bare.

```
{BIS,Commissioned,battletech,warhammer40k}     → all bare
{"Free Write","No Sex",Marvel,"Time Travel"}    → mixed
```

The old parser stripped the braces and kept the whole string, so the first
shape became **one** nonsense tag. The shape is now walked character by
character: a quote opens a tag closing at the next quote, otherwise a tag
ends at the next comma.

> A quoted-run regex is *not* sufficient — it silently drops the bare
> entries in a mixed set. A unit test for this exact case caught it during
> the work.

### 2. Author name was never fetched

Every work was titled `Pawchive user <uid>`. The display name is available
at `/api/v1/patreon/user/{uid}/profile`; the adapter now fetches it.

### 3. The import created one work per *author page*, not per post

45 works were titled `Pawchive user <uid>` and held an author's whole
archive in a single chapter (largest: 99,783 characters / 908 posts).
These had no per-post `library_items` at all.

### 4. Body text was copied into `works.summary`

The summary column held up to 530 characters of the story's opening,
truncated mid-sentence.

## What was repaired

| Repair | Scope | Method |
|---|---|---|
| Author names | 10,071 works | `library_items.author_text` ← profile API |
| Tags | 33,167 works, 326,867 links | taxonomy match + source tags |
| Summaries | most of 33,167 | authored `Summary:` line, else derived excerpt |
| Un-merge | 40 containers → 23,096 works | re-fetch each post from the API |

Scripts live in `/home/alvaro/code-local/research/pawchive/`:

- `pawchive_meta.py` — deterministic extraction (titles, series, chapter
  numbers, characters, relationships, fandoms, summaries)
- `backfill_pawchive.py` — writes tags/authors/summaries to production
- `resplit_containers.py` — re-splits a merged container into per-post works
- `probe_*.py` — the measurement scripts used to choose each threshold
- `run_*.sh` — wrappers that read the DB URL from the production config

All write paths are transactional per author with explicit conflict targets
(`taxonomy_nodes` → `(kind, norm)`, `work_tags` → `(work_id, node_id)`).
Every script has a `--dry-run` and was rollback-tested before committing.

### Front matter is not line-based

The single biggest parsing surprise. Pawchive bodies open with all their
metadata run together on one line, with no separator:

```
Patron RequestThemes: Consensual, Futa, Slutty, Dom/Sub, Free UseWord Count: 2,623Summary: Nanoha wakes Fate up with a morning bloom.
```

A `^`-anchored regex matches none of that. Markers must be found wherever a
lower-case letter or digit runs into the keyword, and each captured value cut
short at the next marker. Roughly two thirds of the archive is in this shape;
anchoring missed every one of those posts' summaries.

Two subtleties in that regex, both of which cost a debugging round:

- The boundary **lookbehind must not consume a character**. Written as
  `(?:^|\n|(?<=[a-z0-9)]))` the `\n` alternative eats one char and the search
  position moves past the boundary, so a later marker on the same line is
  never seen. Zero-width everywhere: `(?:^|(?<=[a-z0-9)])|\n)`.
- `_NEXT_MARKER` needs `re.I` like every other pattern here. Without it
  `Word Count:` (capitalised) is invisible and the themes field swallows the
  summary that follows it.

### The importer stores bodies with the newlines removed

The summary repair looked broken for three full runs. `works.summary` holds a
truncated copy of the post body, and the natural repair — "replace it when the
stored value is still a prefix of the body" — silently matched nothing.

The stored body and the freshly-fetched one differ at the very first line:

```
db : 'Poll WinnerThemes: Slutty Sex, Rough Sex, Fucked SillySummary: It turns out…'
api: 'Poll Winner\nThemes: Slutty Sex, Rough Sex, Fucked Silly\nSummary: It turns out…'
```

The importer's text extraction strips newlines; `html_to_text` keeps them. Every
stored body is 56–76 characters shorter than its source, and the very first
character compared is already wrong, so a prefix test can never pass no matter
how long the strings are. The guard also has a floor (`length >= 80`) so a
short agreement between two unrelated texts is not mistaken for a match.

Both sides are now reduced with `regexp_replace(..., '\s', '', 'g')` before
comparison. Finding this needed a character-level diff of one row against its
API source — the length gap in a per-post sample pointed at it, not any
aggregate.

## The 135M model was tested and rejected

`Fu01978/SmolLM2-135M-Instruct-AO3` is a 135M-parameter creative-writing
model, not a tagger. Run on CPU it produced repetition loops and failed
basic arithmetic sanity checks. Deterministic extraction replaced it and
scores better: measured over 300 live posts, the final extractor emits
**zero** known-noise tags where the first version emitted 202.

## Extraction quality (measured, not assumed)

| Field | Coverage | Notes |
|---|---|---|
| characters | 88.9% | 1,662-word English blocklist; multi-word names trusted |
| fandom | 72.7% | acronyms + qualified names only; bare English words rejected |
| series | 62.7% | from title patterns (`Series! 35`) |
| source tags | 69.1% | author-supplied, preserved verbatim |
| relationships | 5.7% | requires the exact `A/B` form; genuinely rare |
| any abstract | 96.9% | 4.3% authored, 92.5% derived, 3.1% none |

The character filter is the important one. The taxonomy conflates trope tags
("Forced", "Breasts") with character names, and the first version promoted
ordinary English — `Felt`, `Link`, `Long`, `System`, `Abuse`, `Alone` — into
the character field. A 1,662-word blocklist of common English words is
applied to *single-word* entities only; no English phrase is a character
name, so multi-word entries are unaffected.

## Schema facts that cost time

- `works.owner_pseud_id` is **NOT NULL** — new works must reuse an existing
  pseud.
- `chapter_revisions` requires `document_json`, `sanitized_html` and
  `created_by_pseud_id`, and has an FK to `chapters.chapter_id`: **the
  chapter row must be inserted before its first revision.**
- `library_items.status` CHECK allows only
  `ongoing | complete | hiatus | cancelled | unknown`.
- `library_items.updated_at` is **`text`**, not `timestamptz` — cast before
  comparing.
- `taxonomy_nodes.kind` has no CHECK constraint, and the whole existing
  30,411-node taxonomy uses `kind='tag'`. Keep writing `'tag'`; inventing
  a `'fandom'` kind is not what the app expects.
- `work_contributors`, `canons` and `work_moods` exist but were not used —
  `work_tags` is what the app actually reads.
- A CTE is required to join to the UPDATE target: Postgres will not let
  `UPDATE works w ... FROM ... JOIN library_items li ON li.work_id = w.id`
  reference `w`.

## Two environment gotchas

**`cargo test` corrupts its own crate metadata under parallel load**, with
`E0463: can't find crate` and `E0786: found invalid metadata files`. It is
not a code error and not a registry corruption — `cargo build` is fine and
individual test targets pass. Fix:

```bash
CARGO_BUILD_JOBS=2 cargo test -p lorehaven-scrapers
```

**Pawchive's API returns 403 without a User-Agent.** Use:
`Mozilla/5.0 (X11; Linux x86_64) lorehaven-import/1.0`.

## Committed

`fix/pawchive-tag-and-author-parsing` (branched from detached HEAD at
`921a59d`). One commit, `f959531`. 459 tests pass, clippy clean, fmt clean.
The commit also clears four pre-existing clippy warnings in `sanitize.rs`,
`sites/chyoa.rs` and `tests/chyoa_fixtures.rs`, per the project rule about
not leaving a red gate behind.

**The branch is not pushed and not merged** — the repo was on a detached
HEAD when work started. Push and open a PR, or merge locally, as preferred.

## Not done

- The 14 author-page works that were *not* large (315–750 chars) were left
  alone: at that size they may be a genuine single post rather than a
  merged archive. 9 non-withdrawn author-page works remain for this reason.
- Relationships are thin (5.7%). Pawchive posts rarely name the pairing in
  a form the taxonomy recognises; the extractor only accepts the `A/B`
  pattern to avoid inventing pairings.
- No local-fic import was touched in this session — the repair is scoped to
  the Pawchive rows.
