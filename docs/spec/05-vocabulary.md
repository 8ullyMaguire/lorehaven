# 5. The canonical vocabulary

**Status:** reserved
**Milestone:** M1
**Source:** the corresponding section of the Darkhold specification

Normative. The mapping is what lets all three source projects remain readable.

| canonical term | definition | lorehaven | ficnexus | kindred |
|---|---|---|---|---|
| **work** | a creative object, abstract, independent of any site | `works` | `works` | `ao3_work` entity |
| **source** | one concrete copy of a work at one site | `story_identities` members / `external_records` | `fic_info` | (none — corpus rows are the source) |
| **source record** | the raw, uncurated metadata + body for one source | `external_records` + `import_chapters` | `fic_info` + body cache | `works` + `work_tags` rows in the mirror |
| **work identity** | a *proposed* grouping of sources believed to be one work | `story_identities` / `story_identity_members` / `identity_merge_proposals` | `works` + `fic_info.work_id` + `auto_merge_log` | (none) |
| **unit** | a part of a work: a chapter, a track, an episode | `chapters` + `work_units` | `fic_info.chapters` (a count) | (none) |
| **revision** | an append-only version of a unit's text | `chapter_revisions` | `fic_version_bump` | (none) |
| **creator** | a person or account that made something | `pseuds` (local) | `pseuds` + `author_profiles` | `ao3_author` entity (a *name string*, not an identity) |
| **pseud** | a public persona of an account | `pseuds` | `pseuds` | (none) |
| **canonical entity** | a taxonomy node: a tag, fandom, character, ship | `canonical_entities` / `taxonomy_nodes` | `tags` + `tag_types` | `ao3_tag` / `ao3_character` / … entities |
| **tag assertion** | a work's claim about an entity, with provenance and confidence | `work_tags` (+`work_tag_votes`) | `fic_tags` (+`fic_tag_votes`) | `work_tags` rows |
| **theme** | a named, browsable, subscribable group of works defined by a tag set and a title — the *overlay* on the taxonomy | (none) | (none) | (none) |
| **tag-family author** | the author who owns a family of tags; **implies a tag**, and every work carrying any of that family's tags appears when browsing that tag | (none) | (none) | (none) |
| **channel** | a named, curated, public taste profile — a subset of the instance theme with its own weights and reasons, followable by readers | (none) | (none) | (none) |
| **effective weight** | a tag's weight inside a theme: the owner's seed and the community's vote, blended (§12.12.2) | `work_tags.confidence` (a different thing) | (none) | (none) |
| **actor** | who is asking: `anonymous` (no account), `guest` (a row in `accounts` with an empty credential, D-6) or `pseud` (a signed-in reader acting as one of their pseudonyms). The actor carries the account id, the pseud id, the age state, the trust level and the **tier** (spec §20.2) |
| **trust level** | where a reader stands on the ladder: 0 anonymous · 1 new · 2 member · 3 trusted · 4 moderator · 5 senior reviewer · 6 trustee. The names, the unlocks and the levels are **configuration** (spec §14.4), so a route asks for the *privilege* it needs and never holds a level number. A **privilege** is what a level unlocks — the moderation queue, a scoped ban, a mood — and the vocabulary of privileges is closed |
| **trust event** | one entry in the append-only record of a trust change: the account, the level before and after, the **cause**, and what it was about. A level that changed with no event is a defect (spec §6.7) |
| **tier** | the compute tier a request is served on: **T0 house** (the precomputed house-theme ranking), **T1 personal** (the reader's own profile in lite mode, blended with the house floor), **T2 full** (the whole engine). A tier buys **compute**, and never rank (D-10, N-1) — T0 is not "no recommendations", it is a real recommendation and it is explained |
| **instance theme** | the published editorial position of the instance: a set of works, a set of authors and a set of seed tags — **weighted works ≫ authors > tags** — from which the house taste vector is derived. **Public.** | `instance_taste_settings` (private) | (none) | `owner_taste` (private) |
| **opposing tags** | the tags a theme is *against*, derived from the corpus by under-representation, and applied as a **demotion** — never a removal | (none) | (none) | (none) |
| **mood** | a reader-named, online-trained weighting vector over entities. **Reweights; never filters.** | `taste_profiles` (per-user) | preset enum | `profile` |
| **resonance** | how strongly a reader's learned taste agrees with the instance theme, discounted by the strength of their evidence | (none) | (none) | `Agreement` |
| **signal** | a fact about a work that leaves the instance | `exchange_signals` | (none) | (none) |
| **taste signal** | a fact about a *reader's* taste — never leaves the instance | `taste_signals` | `rec_user_signals` | arena tag weights + profile |
| **arena comparison** | a pairwise preference judgement | `arena_ballots` / `tasting_responses` | `arena_votes` | `arena_comparisons` |
| **block** | a reader's explicit rule: "never show me this again" | (mutes, scoped) | (blacklists — site-wide) | `arena_user_blocked_tags` |
| **trust level** | the single authority ladder | `trust_levels` 0–6 | `trust_level` 0–6 | (none) |
| **guest** | a real account with no credential | (none) | `visitor_state` cookie (a shadow) | (none) |
| **supporter tier** | a paid subscription that unlocks the expensive recommendation tier, and nothing else | (none) | (none) | (none) |
| **catalogue** | the read-only, exportable projection of works + sources + tags that the taste service ingests | (none) | (none) | the corpus mirror |
| **archive** | the whole instance | the instance | the instance | (none) |
| **author name** | the raw, uncurated author string on a source record — *never overwritten* | `works.authors` on a source record | `fic_info.author` | a name string on a corpus row |
| **author identity** | a *proposed* grouping of author names believed to be one person — the sibling of a work identity | (none) | (none) | `ao3_author` (a *name string*, not an identity) |
| **reserved author** | a placeholder that is not a person — `orphan_account`, `Anonymous` — excluded from every author-facing surface | (none) | (none) | (none) |
| **author opt-out** | an author's claim that their names must not be clustered, or that two names are not the same person | (none) | (none) | (none) |
| **tag de-prioritisation** | a published, versioned, instance-level list of tags with a reduced weight — and, on explicit opt-in, a blocked set | (none) | (none) | (none) |
| **expansion run** | one execution of the corpus-expansion workflow: a seed set, a configuration, a checkpoint, and a record of what it visited | (none) | (none) | a crawl with `crawl_progress` |

**Identifier scheme.** One primary key everywhere: **UUIDv7** (time-ordered, so index locality
follows creation). A source's own identifier lives in `work_sources.source_key`, unique per
`(source_site, source_key)`. A deterministic `content_hash` (sha256 of normalised
title+author+wordcount+site) is stored for dedupe candidates and is never a primary key. **No URL
is ever a primary key** — a URL changes, a `source_key` usually does not, and a cross-posted work
has five of them.

**Author-name normalisation.** Casefold, strip a trailing parenthesised pseudonym, collapse
whitespace and punctuation, strip leading/trailing dashes. `-Roulette- (Roulette_CV)` and
`-Roulette-` are the same author; `-Roulette-` and `Roulette` are **not**. This is the
**name-level** rule, and it is what an author identity is *proposed* from — it is never the
thing that decides (D-17).

---
