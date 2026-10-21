# 6. Domain model

**Status:** reserved
**Milestone:** M1
**Source:** the corresponding section of the Darkhold specification

## 6.1 Identity

- **account** — credentials, sessions, security state, trust, supporter subscription, invite quota,
  consent record. `status`, `email`, `email_verified_at`, `age_state`,
  `registration_mode_at_signup`, `is_guest`, `consented_at`, `consent_version`, `trust_level`,
  `folded_from_account_id`, `resonance`.
- **pseud** — everything public. `pseud.account_id` is the **only** link and is never serialised
  in any public representation.
- **public_pseud_link** — voluntary, mutual, opt-in. Absence carries no meaning.
- **settings** — per-domain tables (`privacy_settings`, `content_settings`, `search_settings`,
  `content_filters`, `notification_preferences`, `notification_routes`). **Never** one JSONB blob.
- Governance counts **accounts**, not pseuds.

## 6.2 Content

- **work** — `title`, `summary`, `language`, `content_rating`, `visibility`, `lifecycle`,
  `completion`, `format`, `discussion_mode`, `ai_training` (the author's statement),
  `owner_pseud_id`, `derived_activity`, `half_life_bp`, `contested_at`, `deleted_at`.
  - **What the implementation settled.** Five of these six columns already existed and
    the work was to bring them to this table's shape. **The defaults move and the rows
    do not**: a *new* work is a `fanwork` with a `thread_only` discussion, and the rows
    that predate the change keep `'prose'` and `'comments_only'` — which is what the
    migrations that introduced those columns said they should. A table rebuild would be
    the other way to change a default, and SQLite cannot alter one.
  - **`derived_activity` is never a penalty.** It says how long ago the work was
    published and it is not a score, a demotion, or a reason to show a work less. And
    **the author's state always wins** — a work the author has declared concluded stays
    concluded, and the nightly recompute must not overwrite it.
  - **`half_life_bp` is nullable, and "NULL = not yet scored" is a meaningful
    absence.** The scale runs 0..10000, where 10000 is the *maximum* — an evergreen
    boost of 1.5× — so a default of 10000 would give every unscored work the largest
    boost the mechanism can give, and the unscored works are the newest ones.
  - **The full engine follows the tier threshold.** This table's level-4 row says "the
    full engine (T2) **if `full_min_trust` is 4**", so `tiers.full_min_trust` decides
    where the full engine sits and the ladder's row follows it. There is no second
    configuration key to keep in step, and no way for them to disagree.
  - `lifecycle` ∈ draft · scheduled · published · withdrawn · deleted
  - `visibility` ∈ public · unlisted · restricted
  - `completion` ∈ in_progress · complete · hiatus · abandoned
  - `format` ∈ prose · poetry · essay · article · book · fanwork · translation · podfic ·
    audiobook · fan_film · video · fan_comic · comic · zine_scan · image · interactive · dataset ·
    other
  - `derived_activity` is **never a penalty**: active (≤3 mo) · slow (3–12 mo) · dormant (>12 mo) ·
    concluded (the author's state always wins).
  - `discussion_mode` ∈ thread_only (default) · comments_only (legacy, supported indefinitely) ·
    both (operator-warned).
- **unit** — `work_id`, `order_key`, `title`, `current_revision_id`, `word_count`.
- **revision** — `unit_id`, `document_json` (source of truth), `sanitized_html`, `plain_text`,
  `word_count`, `author_pseud_id` (derived). **Append-only**; restoring creates a new revision.
- **publication_event** — the event log, not a row timestamp, drives freshness and sort.
- **work_contributor** · **series** / **series_entry** · **work_relation** · **media_asset** /
  **media_reference** / **media_edition** / **media_rights** / **quality_signal** ·
  **collaboration_invite** · **work_feedback_preference** · **dmca_notice** / **dmca_counter_notice**.

## 6.3 Sources and cross-source identity

- **source_site** — `key`, `name`, `base_url`, `adapter_key`, `posture` (robots, rate, adult-gated,
  login-required), `reachability` (last verified, last failure kind).
- **work_source** — one copy of a work at one site. `work_id` (nullable until identity is
  established), `source_site_id`, `source_key`, `source_url`, `raw_title`, `raw_author`,
  `raw_summary`, `word_count`, `chapter_count`, `raw_status`, `fetched_at`, `body_state`
  (absent · cached · aggregate_only), `content_hash`. **The raw fields are never overwritten.**
- **work_identity** — a *proposed* grouping of sources. `state` ∈ open · proposed · verified ·
  rejected; `method` ∈ auto_merge · curator_proposal · signal_exchange; `confidence`.
- **work_identity_member** · **merge_proposal** / **split_proposal** / **proposal_vote**.
- **merge thresholds** — exact normalised title + normalised author and word count within 5 % →
  merge at high confidence; below it, a **proposal**, never a silent merge. Never merged on title
  alone, never on author alone. **Every merge is reversible** through a split proposal.

### 6.3.1 The author identity (D-17)

**The author's sibling of the work identity.** A work identity groups *sources*; an author
identity groups *names*. The same author is `-Roulette-` on one site, `Roulette_CV` on another,
and three pseuds on one account; a catalogue that claims cross-source identity for works and not
for authors has a hole in the middle of it.

- **author_identity** — `id`, `display_name`, `canonical_name`, `state` ∈ open · proposed ·
  verified · rejected, `method` ∈ source_linked · auto_merge · curator_proposal, `confidence`.
- **author_name** — `id`, `author_identity_id`, `raw_name` (**never overwritten**), `normalised_name`,
  `source_site_id`, `source_author_id`, `first_seen_at`, `last_seen_at`.
- **author_identity_member** — the grouping, one row per name.
- **author_identity_claim** — the **opt-out**: a claim that two names are *not* the same person,
  or that a name must not be clustered. `state` ∈ pending · accepted · rejected, with the claimant
  and the reason.
- **author_pseud_link** — within one source, the pseuds an account uses, as the **source itself**
  asserts them.

**The rules, and they are not the work identity's rules:**

1. **The source's own linkage is trusted; inferred linkage is not.** Where a source exposes an
   account→pseud link, the archive records it and the names are one identity. Where the linkage
   is only *inferred* from the normalised name matching, it is a **proposal** a curator votes on.
   **A name match alone never merges.**
2. **The opt-out is honoured, and it is honoured first.** A pending claim **blocks** the merge
   until it is decided. The default is not "merge and let the author object".
3. **The raw name is never overwritten.** The same rule as `work_source.raw_*`, and for the same
   reason: the record of what the source said is not the record of what the archive believes.
4. **The author page lists every work by that identity, from every source.** This is the owner's
   requirement — *"knowing a single author profile should allow you to find all his/her works"*.
5. **A reserved author is not a person.** `orphan_account` (1,889 works) and `Anonymous` (1,650
   works) are placeholders. They get a reserved identity, and that identity is **excluded from
   every author-facing surface**, from the instance theme's author seed, and from the author
   channels. An archive that lists "Anonymous" as a top author is reporting a hole in its own
   data as a fact about the world.
6. **An author's handle is not a tag.** `chastity` is an author on 25 works in the mirror *and* a
   freeform tag on 36 works *and* the base of `chastity device` (390 works). They are three
   things in three namespaces with three tables, and no query joins them by string equality.
7. **The identity is not a trust or a rank.** It is a grouping. It carries no score, no weight and
   no standing — D-1 and D-2 apply to it exactly as they apply to everything else.

**The measured shape of the problem** (AO3 mirror, 2026-10-19): the `works.authors` column is a
**single author string** on 112,934 of 112,935 works — exactly one work carries a multi-author
string. The strings are raw AO3 handles (`'000strangerdanger000'`, `'-  Scrap - Paper -
(multivers4l_m0r0n)'`), so the normalisation rule of §5 is load-bearing. The two largest
"authors" are the reserved ones, at 1,889 and 1,650 works — **3,539 works, 3.1 % of the corpus,
with no author at all.**

## 6.4 Taxonomy

- **canonical_entity** — `id`, `kind` (freeform · fandom · character · relationship · author ·
  rating · warning · mood · category · theme), `name`, `slug`, `canonical_name`, `parent_id`,
  `review_status` (unverified · verified), `signal_count` (review priority only — never a demand
  weight, never a rank, never a count of readers).

**The entity floor, and why it is not optional.** A canonical entity is created only when its tag
clears a **work-count floor**; below the floor the assertion is still stored, still searchable and
still countable, but it is not a node in the tree. Measured against the AO3 mirror:

| floor | tags that qualify | % of 634,231 |
|---:|---:|---:|
| 1 work | 634,231 | 100 % |
| 2 works | 142,788 | 22 % |
| 5 works | 56,197 | 8 % |
| 10 works | 30,694 | 4 % |
| 20 works | 16,975 | 2 % |
| 60 works | 6,612 | 1 % |

**97 % of the freeform namespace is singletons** — 587,258 of 634,190 names are on exactly one
work. Without a floor, "browse the taxonomy" is 634,231 rows of one-off user phrases, and every
count on the page is a count of one. The default floor is **5 works**, configurable per kind, and
it is the same distinction the AO3 recommender already drew: *"catalog size says an entity exists;
co-bookmarker evidence says someone in the comparison set has read it."*

**Fandom is inferred, never filed.** Only **51 of 112,935** works carry a `fandoms` tag (44
`characters`, 39 `relationships`), while the freeform namespace carries 3,890,504 rows over
112,481 works. So a fandom is recognised by the **shape of the freeform name**, never by the
`tag_type` column — a `tag_type` cap excludes nothing while reporting success.
- **entity_alias** — `entity_id`, `alias`, `source` (author · curator · imported · exchanged).
- **entity_edge** / **entity_closure** — a materialised closure for `+children` expansion.
- **work_tag** — `work_id`, `entity_id`, `source` (author · curator · imported · exchanged ·
  inferred), `confidence` (0..1), `role` (primary · secondary · background), `prominence`.
- **work_tag_vote** — one vote per (work, entity, pseud), CHECK ∈ {−1, 0, 1}. Feeds
  `work_tags.confidence` **only** — never gravity, never a rank, never a reader count.
- **character_assertion** — prominence + roles + attributes.
- **relationship_assertion** — participants (a *set*, >2 supported) + kind + prominence + dynamics.
- **mood** / **work_mood** / **content_warning** / **work_warning** / **rating_check_flag**.
- **theme** — a named group. `id`, `slug`, `title`, `summary`, `owner_account_id` (nullable — a
  community theme has an owner), `tag_set` (the entities that define it), `visibility`,
  `review_status`, `subscriber_count`, `created_at`. **The six first-class themes of §2.2 are
  seeded themes**; a theme is data, not code, and a new one is created by a curator vote.
  - A theme carries **three weights per tag**, and they are different things:
    `seed_weight` (the owner's initial weight, from §2.2's measured tag sets),
    `voted_weight` (the community's current weight, §12.12), and `effective_weight` (the blend of
    the two that the recommender actually uses).
  - **Adding or removing a tag from a theme is a quorum decision** (§14.7). **Re-weighting a tag
    inside a theme is a vote** (§12.12). The two are different powers and never mix.
  - The six first-class themes of §2.2 are the **seed** of this mechanism: their tag sets and
    their starting weights are the measured ones, and the community's vote moves them from there
    (§12.12.2).

**The tag-family author rule.** A tag whose name carries an author attribution — `waifu catalog -
swiftrosenthal`, `celestial grimoire - gothicjedi666` — is BOTH a tag and a channel. Therefore:

1. **It implies a tag.** Browsing `waifu catalog` returns every work carrying *any* tag from that
   family, not just the exact string.
2. **It is a first-class channel.** The author is credited; the channel is listed in the
   directory; it can be followed.
3. **The author's own works seed it.** A tag-family author channel's taste vector is computed
   from the works they wrote, on the same principle as any author channel (§12.2).
4. **If the family cannot be resolved to a canonical entity, it is flattened** into ordinary
   freeform tags — no half-state.

## 6.5 Reader state

`bookmark` · `note` · `shelf` + `shelf_entry` · `reading_progress` · `reading_event` ·
`reading_session` · `reading_aggregate` · `did_not_finish` (private, reversible) · `saved_search` ·
`search_alert` · `author_watch` · `reading_goal` · `typography_preference` ·
`reader_sort_preference` · `block` · `taste_profile` + `taste_profile_version` · `taste_vector` +
`work_taste_vector` · `arena_comparison` · `arena_rating` · `arena_tag_weight` · `exposure_event` ·
`similarity_suggestion` + `similarity_vote` · `recommendation_slot` · `interaction_warmth` ·
`mood` + `mood_vector` + `mood_training_event` · `channel_subscription` ·
`supporter_subscription` · `resonance`.

## 6.6 Community

`forum_category` · `forum_topic` (+ immutable `topic_slug`) · `forum_post` · `poll` + `poll_vote` ·
`forum_vote` (typed) · `forum_meta_vote` · `topic_read_state` · `topic_tag` · `topic_work_link` ·
`critique_queue` · `prompt_post` · `group` + `group_member` · `conversation` + `message` ·
`collection` + `collection_entry` · `challenge` + `prompt` + `signup` + `assignment` + `claim` +
`fulfillment` · `mentorship` · `sprint` · `wishlist_item` · `request` + `request_answer` +
`request_vote` · `bounty` + `bounty_contribution` · `commission` · `notification` +
`notification_preference` + `notification_route` · `push_subscription` · `feed_token`.

## 6.7 Governance

`trust_level` · `trust_event` · `trust_metric` · `role_assignment` · `report` · `case` ·
`jury_panel` + `jury_panel_member` + `jury_ballot` · `sanction` · `appeal` · `audit_event` ·
`quorum_record` · `process_feedback` · `shadowban_action` · `abuse_counter` · `security_event`.

## 6.8 Taste and the instance theme

`instance_theme` (the singleton: title, summary, works, authors, tags, published flag, version) ·
`instance_theme_work` · `instance_theme_author` · `instance_theme_tag` · `instance_theme_version` ·
`theme_opposition` (the derived opposing tags, per theme, with their strength) ·
`house_taste_vector` (derived; the published weights) · `theme_entity` (the six themes) ·
`theme_tag` · `channel` · `channel_subscription` · `taste_profile` · `taste_profile_version` ·
`taste_vector` · `work_taste_vector` · `arena_comparison` · `arena_rating` ·
`arena_user_tag_weight` · `arena_session` · `permitted_signal` (an **exhaustive** list) ·
`exposure_event` · `work_coordinate` (four arithmetic measures over text — byte-for-byte
reproducible) · `demand_weight` + `demand_weight_history` + `demand_diversity_state` ·
`recipe` + `recipe_version` · `editorial_pick` · `resonance_score` · `taste_leakage_event`.

## 6.9 The four instance-level policies

Each is set **once, by the operator, at instance level**, and nothing — no work, uploader,
importer, extension or federated peer — may raise it. Each has a **distinct refusal** that names
the policy. The vocabularies deliberately do not share names.

| policy | values | default | the refusal names |
|---|---|---|---|
| **accessibility mode** | `public` · `walled_garden` · `private` | `public` | "this instance is a walled garden" |
| **body retention** | `cache` · `aggregate` | `cache` | "this instance does not retain work bodies" |
| **media hosting** | `host` · `reference` · `catalogue` | `reference` | "this instance references media rather than hosting it" |
| **generated content** | `forbid` · `disclose` · `allow` | `allow` (D-4/N-4 — the archive indexes and recommends AI-generated works, labelled) | "this instance forbids generated content" |

---

## 6.10 The instance tag de-prioritisation (D-18)

**A published, versioned, instance-level list of tags the instance does not want to see as much.**
It is a **taste** statement, not a content policy: the work stays, the tag stays, the tag is
simply not promoted.

**The default list**, from the owner's ruling, is these nineteen:

```
M/M · Slash · Gay · MPreg · LGBT · LGBTQ · Trans Character · Hurt/Comfort · Fluff · Humor ·
Angst · Slow Burn · Slow Romance · RPF · Real Person Fiction · Reader · Omega · Needs a Hug ·
Bestiality
```

**The two modes, and the default is the one that breaks nothing:**

| mode | what it does | what it does *not* do |
|---|---|---|
| **`weight`** (default) | the tag's contribution to the house taste vector, to the theme vectors and to the ranking's tag-overlap signal is scaled by `weight` (default **0.0**) | it removes nothing. The work is **reachable**, and it is reachable in exactly the same set under every sort |
| **`block`** (explicit operator opt-in) | the works carrying the tag are excluded from the **browse and recommendation** surfaces | it does not delete the work, does not remove it from search, does not remove it from its own page, and does not remove it from a reader's own library. **The blocked set is published in `/api/v1/meta`** so the sort contract can exempt exactly that set and nothing more |

**The rules:**

1. **The list is public.** A hidden editorial preference is a private preference (D-8's logic).
   It is published on a page and in `/api/v1/meta`.
2. **The list is the operator's, and it is not voted.** The community votes on the *themes'*
   weights (§12.12), not on the instance's de-prioritisation. This is the same boundary as
   §12.12.1's "the instance theme's seed stays the owner's".
3. **It applies wherever the tag would be promoted** — the house taste, the theme vectors, the
   tag-overlap signal, the trending score. It does **not** apply to a reader's own bookmark,
   rating, library item or review: **de-prioritising a tag must not empty a reader's library.**
4. **An entry matches on the normalised name**, so it works before the tag has a canonical
   entity, and it matches the entity once one exists. The **fandom rule's base blocklist**
   (§11.8) is a separate, curated list and is not the same thing.
5. **It is versioned**, so a recommendation made last month is explainable against the list that
   produced it — the same rule as the instance theme (§12.1) and the channel seed (§12.10).
6. **The sort contract is amended, narrowly and explicitly.** §11.7's assertion becomes *"the
   reachable set is identical under every sort, modulo the published blocked set"* — and the test
   compares against **the blocked set named in `/api/v1/meta`**, not a set the test invented. A
   test that exempts a set it made up is a test that proves nothing.
