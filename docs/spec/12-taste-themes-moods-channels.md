# 12. Taste, themes, moods and channels

**Status:** reserved
**Milestone:** M13, M14, M17, M18
**Source:** the corresponding section of the Darkhold specification

This is the module kindred contributes and the part of Darkhold that is least like the other two.
It is also where the owner's answers changed the design most.

## 12.1 The instance theme — a published editorial position

**The instance theme is public.** It is the site's stated identity, shown on the front page, and it
is the gravity pull for every reader's recommendations.

**How it is chosen: from works and authors, not tags** (D-11). The owner's words: *"I prefer to
choose my favorite fics/authors over tags to choose the instance theme, the content the algorithm
pulls users towards."*

So the theme is a set of **works** and a set of **authors**, and the tag vector is *derived* from
them:

```
instance_theme
  ├── instance_theme_works      (the chosen works — the PRIMARY signal)
  ├── instance_theme_authors    (the chosen authors — a secondary signal)
  ├── instance_theme_tags       (the chosen seed tags — a tertiary signal)
  └── house_taste_vector        (DERIVED from all three, in that order of weight)
```

**The three tiers, and why the order is the design (D-11).**

> **Works indicate a theme better than authors, and authors better than tags.** A work is a
> complete statement of what an instance is about; an author is a statement about a body of
> work; a tag is a statement about a *category*, and categories are the most widely shared
> thing in a catalogue. So the seed tag is real input — the operator chooses it, deliberately,
> and it carries weight — but it is the **weakest** of the three signals, and the **derived**
> co-occurring tags are weaker still.

```
house_taste(e) = α_w · w_work(e) + α_a · w_author(e) + α_t · w_tag(e)
```

with `α_w = 0.60`, `α_a = 0.25`, `α_t = 0.15` by default. **The ordering is the design and
the numbers are configuration** (`[theme] signal_weights`). The three terms are:

| term | what it is | how it is computed |
|---|---|---|
| `w_work(e)` | the entity's weight **in the chosen works** | the entity's normalised term frequency across the theme's works, averaged over them |
| `w_author(e)` | the entity's weight **in the chosen authors' work** | the same measure over the works of the chosen authors |
| `w_tag(e)` | the entity's own weight **in the theme** | the seed weight for a chosen seed tag; the PMI rank's weight for a derived neighbour; **0 for an entity that is neither** |

**The seed tags are chosen, the neighbours are derived.** The operator picks the seed tags the
same way they pick the works — the picker shows the derived tag distribution updating live, and
**the tags are the consequence of the choice and one of its three inputs at once**. The 24 PMI
neighbours (§2.2) remain a mechanical expansion and are never chosen by hand.

### 12.1.1 The opposing tags

**A theme knows what it is against.** The same measurement that produces a theme's members
produces its opposites, and a work carrying one is **shown less** — demoted, never removed
(N-7, N-16).

```
lift(t, T) = (works in T carrying t ÷ works in T) ÷ (works in the corpus carrying t ÷ works in the corpus)

opposing(t, T) = { t : lift(t, T) ≤ ½  and  works(t) ≥ the entity floor (§6.4) }
strength(t, T) = 1 − lift(t, T)          ∈ (0, 1]
```

**And the penalty is applied where the theme acts:**

```
score(work) −= β · Σ  strength(t, T)      over the opposing tags t the work carries
```

with `β` the theme's opposition weight, default `0.35`.

**The rules, and each one is there because of a specific failure it prevents:**

1. **The opposition is measured against the theme's EXPANDED set — its seed plus its 24 PMI
   neighbours — not against the literal seed strings.** This is the whole defence against the
   ubiquitous-tag trap. A tag under-represented across **611** works is mostly noise; a tag
   under-represented across **12,866** works is a register. The corpus's ubiquitous tags — `f/m`
   (55,765 works), `m/m` (29,290), `multi` (17,746), `gen` (21,850) — have a *negative* PMI
   against any small cluster, because a small cluster is a tiny sample, and they are exactly
   the tags that must not be mistaken for opposites. **The trap is handled by the size of the
   measurement, not by a special case.**
2. **The opposition is a penalty, never a negative weight in the vector.** §12.12.3 asserts
   that the share of a theme's recommendations carrying a tag is within ±0.05 of the tag's
   normalised effective weight — and a negative weight makes that share negative, so the
   assertion is undefined. **The penalty is outside the vector**, the vector stays
   non-negative, and the assertion stands.
3. **It is a demotion, not a removal.** A work carrying an opposing tag is reachable, and the
   reachable set under every sort is unchanged. **A theme that pushed a work out of the set
   would be a shadowban applied by ranking instead of by moderation** (N-7).
4. **It is derived, per theme, and published.** A theme's opposing set is part of that theme's
   published weight profile (§12.12.6), so a reader can see what a theme is against — the same
   way they can see what it is for.
5. **It is not the de-prioritisation list.** §6.10's list is **operator-set and
   instance-wide**; the opposing set is **derived, per theme, and from the corpus**. They are
   different mechanisms and they compose: the operator's list scales the vector, and the
   derived opposition applies the penalty. Neither is derived from the other.

**The selection UI** is a picker over the archive's own catalogue: choose works and authors, see
the derived tag distribution update live, and publish. Tags are shown as *the consequence* of the
choice, never as the mechanism.

**Seed from the owner's own library.** The 54 works of §2.1 — 13 written and 41 bookmarked, one
merged profile at equal weight — are the default seed. A natural initial state is the 41
bookmarked (taste) plus the 13 written (aspiration), and the operator can re-weight by removing
any of them.

**The theme is public in three places:** the front page ("this instance's theme: *X*"), the
`/api/v1/meta` payload, and the reason string on every recommendation the theme influenced. It is
**versioned**: `instance_theme_version` records every change, so a recommendation made last month
is still explainable against the theme that produced it.

**The theme is not the operator's private taste.** The distinction is the spine of this section:

| | public? | who it belongs to |
|---|---|---|
| **the instance theme** | **yes** — front page, API, every reason string | the instance's editorial identity |
| **a reader's taste profile** | **no** — never, to anyone, including the operator | the reader |
| **a reader's moods** | **no** — the names may be private; the vectors are always private | the reader |
| **a channel's seed** | **yes** — a channel is a public object with a public seed | the channel's owner |

The lorehaven rule (N-2) protects the second row. It says nothing about the first, and the first is
what makes a stranger's first five minutes useful.

## 12.2 The house taste and the reader's taste

The house taste is a **prior**, not a filter. It is the theme's derived vector, applied as one term
in the blend, and it **decreases** as a reader accumulates their own evidence — the same decay
shape as ficnexus's curator prior, `α = floor + (1−floor)·exp(−n/τ)`, with `floor` and `τ` as
configuration. A reader with no signal of their own gets the house taste almost entirely. A reader
with a hundred comparisons gets their own taste with the house as a floor under it.

**The house taste is also the diversity guard.** A reader's own profile may be narrow; the house
taste contributes a floor of breadth so the recommendation set is never a monocrop. This is the
opposite of the engine's documented failure (§9.8), and it is a *floor*, not a filter — it adds
works to the candidate pool, it never removes them.

**Reader taste is built from three sources, kept apart:**

1. **Explicit** — ratings, reviews, kudos, bookmarks, reading completion, "did not finish".
2. **Behavioural** — what was opened, kept, returned to, and compared favourably.
3. **Arena** — pairwise comparisons, which are the highest-signal and most expensive source.

## 12.3 Moods — named by the reader, learned online

A **mood** is a reader-created, named, online-trained weighting vector over entities.

**The owner's definition:** *"moods to be named by me in app, and allow me to get different
recommendations for each mood but trained as I use the site, not pretrained."*

Therefore:

- **Named by the reader.** There is no shipped list. The reader types a name. There is no preset
  (`adventurous|comfort|intellectual|emotional` is refused — D-12), and no model is shipped.
- **Learned online.** A mood's vector starts at **zero**. It grows only from the reader's own
  behaviour **while that mood is active**. There is no pretraining, no seed vector, no transfer
  from the house taste.
- **Active for a session.** A mood is toggled on; the training signal is attributable to the mood
  that was active. The UI shows which mood is active and offers a one-click "name this mood" from
  the current session.
- **Reweights, never filters** (N-16). A mood changes the *order*; the reachable set is identical.
  This is asserted by test, exactly as the sort contract is.
- **Degrades explicitly.** A cold mood has a zero vector. The page says *"this mood has no learned
  signal yet"* — it does not silently fall back to the house taste and present it as the mood's
  answer. Silence — neither note nor error — is a bug.
- **Training events are the evidence.** `mood_training_event` records (mood, work, action,
  weight, at). The vector is a derived aggregation over those rows, so a mood can be inspected,
  corrected, deleted and exported like any other reader state.

**The learning rule**, stated once so it can be tested:

```
For each event e on work w while mood m is active:
    m.vector[entity] += e.weight × decay(now − e.at)

where e.weight is:
    kept / finished / rated-high / compared-favourably   →  +1.0
    opened                                             →  +0.2
    skipped / closed fast / "not for me"                →  −0.5
    blocked                                            →  excluded entirely, and the mood
                                                         vector for that entity is set to
                                                         −∞ (a block is a rule, not a weight)

and decay is exp(−Δt / τ_mood), τ_mood = 30 days by default.
```

**Who gets moods.** The owner always. Beyond that, the gate is configuration, and the owner's
instruction is that **subscribed users and/or users with trust level ≥ N** get the ability. So:

```toml
[discovery.moods]
enabled = true
min_trust_level = 3            # N — the owner chooses
supporter_only = false          # true = supporters only, ignoring trust
max_moods_per_account = 12
```

The union of the two conditions is the rule: a reader needs trust ≥ N **or** a supporter
subscription. Both knobs are the owner's to set and both are visible in the instance metadata.

**Moods are the one place the expensive tier is worth paying for**, because training a per-mood
vector is a per-reader computation that does not amortise across readers the way the house taste
and the graph do.

## 12.4 The arena

**Glicko-2, with draws.** Elo was rejected: no cold-start advantage, no confidence measure, no
inactivity decay — and this corpus is heavily recent, which is exactly the population rating
deviation exists for.

Constants: `SCALE = 173.7178`, `TAU = 0.5`, `MU_INIT = 1500`, `PHI_INIT = 350`,
`SIGMA_INIT = 0.06`, `Draw = 0.5`. The five places the published algorithm is easy to get wrong —
the `v` formula, the `phi'` term, the volatility bracket's asymmetry, and the scale — are each
pinned by a test against Glickman's own worked example.

**Arenas for every entity kind**: work, tag, character, relationship, fandom, author. The
allowlist is defined once. An unknown kind is a **400 naming the allowlist**, not a silent
fall-through to works.

**Pairing strategies**: random (weighted so under-rated works appear — without it a work can never
earn its first comparison), max-information, close, and **explore** (deliberately presents a
pairing the reader's learned weights say they will dislike).

**The one hard rule on arena learning:** a tag present on *both* compared works tells you nothing
and must not be recorded as a negative. Rarity weighting: `rarity(t) = −log10(p)/4` squashed to
(0, 1], because tag frequency spans five orders of magnitude.

## 12.5 Theme-weighted consensus

The instance theme's taste scales the influence of a reader's comparison. This is kindred's
owner-weighted consensus, with the difference that the instance theme is **public** (D-8) and the
**reader's** agreement with it is **not**.

- **Agreement** is the cosine between the voter's evidence-damped weights and the house taste
  vector. A voter with fewer than `minEvidence` (5) comparisons scores **0** — "the conservative
  default: the unproven voter does not move the global signal."
- **Influence is `agreement²`**, and below the floor (agreement ≈ 0.22) the comparison is **dropped
  from the period entirely**. Not clamped. *Weight zero means UNSET in the rating maths, so a
  clamped zero would be counted at full influence — exactly backwards.*
- **The influence curve**, with the floor stated so the maths is checkable:

  | agreement | influence |
  |---|---|
  | 0.00 | 0.00 |
  | 0.22 | 0.05 |
  | 0.50 | 0.25 |
  | 0.71 | 0.50 |
  | 1.00 | 1.00 |

- **The page says so.** A sentence naming the house theme is printed on the arena and the
  leaderboard when a theme is published, and **absent when it is not** — including on an empty
  board. "The condition is the wiring point, not a successful build."
- **Anti-recommendations and hidden-gems stay unweighted.** Weighting them by the house theme makes
  the page fight itself. The cost is stated: those pages do not reflect the house, and a reader
  comparing them with the leaderboard sees two notions of "good" on one site.

## 12.6 The compute tiers — who gets which algorithm

**The owner's requirement:** *"everyone gets best recommendations possible, but if that would take
too much resources then subscribers (or high trust level or high resonance users if I choose) get
best recommendations and everyone else no recommendations or a simpler recommendations algorithm.
So anonymous visitors get personalized recommendations (pulling towards my taste like everyone
else) if it's not too resource intensive, otherwise everyone gets my taste."*

The resolution maps exactly onto **kindred's two measured modes**, which is why the estate already
has the numbers:

| tier | who | algorithm | cost | measured |
|---|---|---|---|---|
| **T0 — house** | anonymous, and anyone before they have acted | the precomputed house-theme ranking, plus a cheap per-session adjustment (fandom affinity from what they clicked this session) | one cached list + a small in-memory blend | negligible; the list is computed once per catalogue version |
| **T1 — personal (lite)** | registered readers | the reader's own profile, blended with the house floor, using **lite mode only**: tag overlap + neighbourhood, no embeddings, no multi-strategy RRF, no co-bookmark walk, no theme-weighted consensus | the light engine | **25–26 MiB** against a 60 MiB cap; `recommend` n=5 in 125 ms |
| **T2 — full** | supporter tier, **or** trust level ≥ N, **or** resonance ≥ threshold | the full engine: CSR graph neighbourhood, spectral embeddings, co-bookmark collaborative signal, multi-strategy RRF fusion, theme-weighted consensus, arena, per-fandom diversity budgeting, the catalogue export | the heavy engine | **184–186 MiB** against a 220 MiB cap; `ingest` 181.6 MB / 1 m 31 s; `embed` 203.2 MB / 3 m 10 s |

**The rules that make this honest:**

1. **T0 is not "no recommendations".** It is the house taste, which is a real recommendation and
   is *explained*. The fallback is never an empty page.
2. **The tier a reader is on is visible to them** and states *why* — "your recommendations use the
   full engine because you are a supporter", or "…because your trust level is 4", or "…because your
   taste agrees with this instance's theme". Never a silent downgrade. **Exactly one reason is
   named, and it is the one that matched** — see rule 6.
3. **A downgrade is not a punishment and never a purchase of rank.** It buys *compute*, and it is
   priced as such (D-10). A non-supporter's T1 recommendations are not "worse", they are *cheaper*.
4. **Resonance** is how strongly a reader's learned taste agrees with the instance theme,
   discounted by the strength of their evidence — the same machinery as §12.5, read in the other
   direction. A reader with no evidence has resonance 0 regardless of agreement, because agreement
   without evidence is a coincidence.
5. **The tier boundaries are configuration**, and they are published in `/api/v1/meta` so a client
   can show the reader where they stand.
6. **The three T2 gates are evaluated in a fixed order and exactly one wins.** Three overlapping
   "or" conditions produce an ambiguous tier, and an ambiguous tier produces an ambiguous
   explanation. The order is **supporter → trust → resonance**, and the matched gate is the one
   named in `meta.tier.reason` and on the reader's page. A reader who qualifies by all three is
   told they qualified as a supporter, because that is the reason that will still be true if the
   other two change. The `meta.tier` payload carries the matched gate, the tier, and — for
   transparency — the *unmatched* gates with their current values, so the reader can see what
   they are one step away from.

```toml
[tiers]
house = "always"                       # T0 is always available
personal_min_trust = 1                # T1 needs a registered account
full_supporter = true
full_min_trust = 4                    # N
full_min_resonance = 0.55
```

**The resource frame that sets this.** The design target is a **Raspberry Pi 5B with 4 GB of
RAM**, running the archive, the taste service, Postgres and everything else. That is why the
tiers exist, why the two-engine discipline exists, and why the mood vector (§12.3) — the one
per-reader computation that does not amortise — is gated behind the tiers rather than being
universal.

## 12.7 The recommender

**A linear blend of named signals.** Each signal computes a value in [0, 1] and a reason, or
**skips**.

> **The weights below are INHERITED AND UNVALIDATED.** They come from kindred, where they were
> fitted against an AO3 mirror with no house taste, no theme channels, no mood reweight and no
> tier model. `house_taste` at 0.06 is a *copy* of kindred's `owner_taste` at 0.06, which was a
> different subject doing a different job. They are a **starting point, not a result**, and
> **M13's weight-fitting milestone** (§24.3) replaces them with fitted values. The table is
> labelled as inherited so nobody mistakes a plausible number for a measured one.

**The load-bearing convention: a signal that cannot be computed returns a skip, not a zero.** A
zero is a claim ("this is not a good match"); a skip is a gap ("I have no opinion"). Skips are named
in `meta.degraded[]` and printed on the page.

A candidate that **every** signal skipped is **dropped**, not ranked at zero. "An entity with no
signal at all is not a poor match, it is an unknown one."

| signal | weight | computes |
|---|---|---|
| `tag_overlap` | 0.22 | Jaccard against the seeds, best seed wins, normalised by the union |
| `neighbourhood` | 0.30 | PMI-weighted votes from the seeds' top-24 neighbours, divided by `sqrt(candidate tag count)` |
| `embedding` | 0.10 | cosine against the mean of the seeds' vectors; **absent in lite mode** |
| `collab` | 0.16 | item-item co-bookmark votes, normalised by the request's observed maximum; **T2 only** |
| `quality` | 0.09 | `log1p(kudos / (words/1000)) / log1p(500)` — praise per effort |
| `recency` | 0.09 | `2^(−age_days/365)` |
| `popularity` | 0.09 | `0.6·log1p(hits)/log1p(1e6) + 0.4·log1p(kudos)/log1p(1e5)` |
| `peer_rating` | 0.11 | the arena's effective rating, `(v − median)/150`, clamped; **skips when the arena has rated nothing**, but scores 0 for a work the arena has not rated |
| `house_taste` | 0.06 | the instance theme as a global prior; **skips when no theme is published** |

**The theme term is a *prior*, not a filter.** It is one weight among nine, and it decays as the
reader's own evidence grows (§12.2).

**Filters apply inside the pool query, before scoring.** "Filtering a finished result list returns
a subset of a *ranking*, which is not the same as a ranking over a smaller pool." A blocked tag
must change what is *ranked*, not just what is *displayed*.

**Diversity is MMR** (λ = 0.75, k = 500) applied after ranking, with optional per-group caps. The
group detector is **measured, not assumed** — kindred's fandom detector uses the name's *shape*,
because the corpus's `tag_type` column says "freeforms" for 3,890,504 of 3,891,300 rows.

## 12.8 The strategy registry and RRF

Multiple strategies behind one interface, blended by **Reciprocal Rank Fusion**:

```
score(w) = Σ_s  weight_s · 1/(60 + rank_s(w))
```

`k = 60` is the paper's constant. Rank-based fusion means wildly different score scales combine
cleanly.

- The registry is configuration (`strategies = "cooccur:0.4,embeddings:0.3,bandit:0.1"`). An
  unknown name is **dropped with a warning**, not fatal. An empty resolution falls back to the
  default at weight 1.0.
- **A strategy that errors is skipped and the request still succeeds.**
- **A golden guarantee:** a registry containing only the legacy strategy produces **byte-identical**
  output to the legacy engine, asserted by a DB-gated test.
- **Shadow mode** computes the pluggable result, returns the legacy result, and logs impressions.

## 12.9 Blocks

A **block** is a reader's rule: "never show me this again". It is stored **separately from an
inferred weight**, because:

- **Unblocking restores what the engine inferred.** Removing a block does not pretend the reader
  ever liked the entity.
- **A weight batch can never overwrite a block.** "Blocks are rules; weights are inferences."

Blocks are **idempotent and explicitly not a toggle**: the request carries `action: block |
unblock`. A toggle makes the outcome depend on state the request does not carry.

Every block is **visible and reversible** — the list renders first, above every candidate, with an
unblock control. The empty state is an **explanation**, not a blank page.

Candidates come from two sources, because each catches what the other cannot: the learned-negative
weights (gated on at least two comparisons — "a weight from a single comparison is a coincidence,
and offering it invites a block the reader does not want"), and the tags common across works the
reader passed over.

**No bulk block. No block reasons.**

## 12.10 Channels

A **channel** is a named, curated, **public** taste profile: a subset of the instance theme with
its own weights and reasons, followable by readers. Two kinds, and they are the same object:

- **Theme channels** — the six first-class themes of §2.2, each seeded by the owner's chosen works
  and authors and weighted by the tags that define it. **Their tag weights are community-voted**
  (§12.12) — the seed is the owner's starting point and the vote moves it.
- **Author channels** — a work author (CambrianBeckett) or a **tag-family author**
  (`waifu catalog - swiftrosenthal`, `celestial grimoire - gothicjedi666`). Their seed is their
  own works' tag profile, computed on the same principle as the house taste. **Their weights are
  derived, not voted** — an author's tag profile is evidence, not an opinion, and there is nothing
  meaningful to approve.

**Discovery.** A public, browsable directory at `/channels`: search by name, theme, tag, author and
fandom; each channel shows its owner, its seed, a sample of works, and a subscriber count. A
channel is an editorial object of the instance, and a hidden channel is just a private preference.

**Two different things are called "subscribe", and the spec refuses to confuse them:**

| | what it is | cost | what it grants |
|---|---|---|---|
| **following a channel** | a taste subscription | free | that channel's recommendation stream in the reader's feed, and a vote in the channel's diversity budget |
| **the Supporter tier** | a payment | paid | **compute only** — the T2 recommendation tier (§12.6) |

**Channel privacy.** A follower of a channel has their taste aggregated into the channel's signal
with the same **k-anonymity floor** as the snapshot exchange (§15.3). The channel owner sees
subscriber counts and aggregate engagement — never an individual's profile, never an individual
follower's history. The channel's *seed* is public by definition.

**A channel's published seed is frozen; editing it publishes a new version.** The same rule as the
instance theme (§12.1) and for the same reason: a recommendation made last month has to be
explainable against the seed that produced it. So:

- A channel has `channel_versions`, and the recommendation's reason string names the version.
- **Editing the seed creates version N+1.** It never mutates version N.
- **A follower sees which version produced a recommendation**, and a version's seed, its weights
  and its vote tally are all public.
- **Withdrawing a channel from publication is not a version** — it is a separate switch, and it
  **deletes no votes and no versions** (§12.12.8). The votes exist whether or not the channel is
  listed.

## 12.11 The taste service API

```
GET  /api/v1/recommend?seed=&seed=&kind=&n=&pool_mode=&filter…
GET  /api/v1/entities/{kind}/{id}/similar
GET  /api/v1/arena/pair?kind=      POST /api/v1/arena/compare
GET  /api/v1/arena/leaderboard?kind=
GET  /api/v1/theme                  GET  /api/v1/theme/versions
GET  /api/v1/channels               GET  /api/v1/channels/{slug}
POST /api/v1/channels/{slug}/follow
GET  /api/v1/moods                  POST /api/v1/moods
GET  /api/v1/moods/{id}             DELETE /api/v1/moods/{id}
POST /api/v1/moods/{id}/activate    GET  /api/v1/moods/{id}/training
GET  /api/v1/taste/profile          DELETE /api/v1/taste/profile
GET  /api/v1/taste/blocks           POST /api/v1/taste/blocks   (action: block|unblock)
GET  /api/v1/tiers                  GET  /api/v1/taste/degraded
GET  /api/v1/channels/{slug}/weights
GET  /api/v1/channels/{slug}/ballot  POST /api/v1/channels/{slug}/ballot
GET  /api/v1/channels/{slug}/periods
GET  /healthz                      (503 when over the RSS cap)
```

Every response carries `meta.degraded[]`, `meta.tune` (the named weights actually used) and
`meta.tier` (which tier produced it, and why).

---

## 12.12 Voted cluster weights

**The proposal.** The six themes start from the tag sets already measured in §2.2. High-resonance
users then vote on which tags inside a cluster matter, and the cluster's content mix becomes
**proportional to the community's approval percentage** for each tag.

**The verdict: worth it, and it fits the existing machinery.** The cost is four tables, six
endpoints, one catalogue field, one term in the blend, and three mutation tests. The reason it is
worth more than its cost is not novelty — it is that **a self-governed instance whose six themes
are frozen configuration is a contradiction the community will notice within a month.** The
themes are the instance's public face (D-8) and its main shared taste surface; letting the
community weight them is the first place the estate's trust ladder, quorum rules and taste graph
act on the same object.

It is not a large project. It is a small one that would be conspicuous by its absence.

### 12.12.1 What is voted, and what is not

**The boundary is the whole design, and it is three lines:**

| | decided by | mechanism | quorum? |
|---|---|---|---|
| **which tags are in a theme** | the owner (or a curator) | a taxonomy change | **yes** — §14.7's routine tag change, 2 reviewers |
| **how much each tag matters inside a theme** | **the community** | **a vote** | no — the vote *is* the quorum |
| **whether a theme is published at all** | the owner | publication | yes — §14.7's channel publication, 2 |

So a brigaded vote can shift weights inside a cluster; it **cannot** redefine the cluster, and it
**cannot** publish anything. The owner's structural power is the seed weight and the publication
switch, and the community's is the numbers. Neither can take the other's.

**The instance theme's seed stays the owner's** (D-11). The votes apply to the *theme channels*
of §12.10, not to the instance theme's seed. The instance is the owner's editorial position; the
themes are the community's working taxonomy.

**A theme's tag set is the owner's seed plus its PMI neighbours** (§2.2), and the two have
different standing:

- **Seed tags** are the owner's explicit picks. They carry the seed weight and are the theme's
  identity. A theme with no seed is not a theme; it is a cluster.
- **Expanded tags** are the co-occurrence neighbours, mechanically derived, and they carry a
  weight from their PMI rank rather than from a human choice. **The community can drive an
  expanded tag's effective weight to zero** — that is the ordinary operation of a vote, and it
  is how a cluster prunes itself. **The owner can also remove an expanded tag**, and that is a
  membership decision under §14.7.

This is why the vote is worth having: on the literal seed strings the Gamer / System cluster holds
**611 works** — too few for a weight shift to mean anything — and on the expanded set it holds
**10,567**. A vote over 611 works is a vote about six tags; a vote over 10,567 is a vote about a
genre.

### 12.12.2 The weights

Three weights per tag, and they are different things:

```
seed_weight(t)     the owner's initial weight for tag t in theme T   (from §2.2's tag sets)
pct(t)             the community's approval fraction for tag t         (this period)
effective_weight(t) what the recommender uses
```

**The community's approval fraction.** One ballot per account per period covers **every** tag in
the theme: each voter approves or disapproves each tag. Then

```
V       = distinct accounts that voted in this period
A(t)    = accounts that approved t
pct(t)  = A(t) / V
```

`pct` is a genuine percentage of the voting population, and because one ballot covers all tags,
`V` and the decay term are the same for every tag in the theme. There is no such thing as an
"unvoted tag" inside a theme that has votes.

**The blend.** The community's view starts at nothing and converges toward the owner's seed
asymptotically, never reaching it:

```
α_comm(V) = (1 − floor) · (1 − exp(−V / τ))
effective_weight(t) = α_comm(V) · pct(t) + (1 − α_comm(V)) · seed_weight(t)
```

with `floor = 0.2` and `τ = 25` by default — **the same decay shape as §12.2's house-taste
prior, with the roles swapped** so that the starting state is the owner's seed rather than the
community's. `α_comm(0) = 0` (all seed), `α_comm(∞) = 0.8` (80 % community, 20 % owner). The
owner can never be fully displaced, and the community can never take the theme over in one week.

**Why a percentage-of-voters and not a percentage-of-approvals.** `A(t)/(A(t)+D(t))` would make
a tag with one approval and no disapproval look like 100 %. The vote is a *sample of a
population*, so the denominator is the population that voted, and abstention is expressed by not
voting at all.

### 12.12.3 The content mix follows the weights — and is asserted

The theme channel's taste vector is a weighted sum:

```
vec(T) = Σ_t  effective_weight(t) · vec(t)          normalised over the theme's tags
```

Because the vector is a weighted sum, the share of the channel's recommendations attributable to
each tag follows the weights by construction.

**And that is asserted, not assumed.** The observable property is what matters, and it is
testable:

> For a theme channel T and any tag t, the share of T's recommendations whose works carry t is
> within `±0.05` of `effective_weight(t) / Σ_t' effective_weight(t')`.

This is the test that catches the failure the construction does not: a correct vector that the
ranker does not actually follow. A weighted sum is necessary and not sufficient, and the
assertion is on the *observed mix*, not on the intermediate vector.

**Blanket approval degrades gracefully.** If every voter approves every tag, `pct(t) = 1` for all
`t`, so `effective_weight(t) = 0.8 + 0.2·seed_weight(t)`. Normalised, the theme's *ordering* is
preserved but its *differences compress by 5×* — the theme flattens toward uniform without
inverting. A minority that disapproves a tag still moves it down. There is no ballot that can
invert the owner's ordering, and none that can erase the community's contribution either.

### 12.12.4 The rate limit on governance

**A weight may not move more than a bounded fraction of the remaining distance to its computed
value in one period.** With `max_shift_per_period = 0.25`, a tag moves at most a quarter of the
way toward its newly computed weight per period, so a single brigaded week cannot flip a theme
and the applied weight lags the computed weight by construction.

This is the same shape as the age gate's grace period and the trust ladder's elastic decay: **a
governance change is applied over time, not instantly.** The lag is the protection, and it is
visible — a channel page shows both the applied weight and the computed weight when they differ.

### 12.12.5 Who votes

**High-resonance users**, per the owner's instruction, with the same three-way gate as the T2
tier, but as a **separate configuration** so the two can be tuned apart:

```toml
[themes.voting]
enabled = true
min_trust_level = 3
min_resonance = 0.55
supporter_only = false
period_days = 7
max_shift_per_period = 0.25
seed_floor = 0.2          # the owner's permanent share
tau = 25                 # the community's convergence rate
```

**Votes are per account, not per pseud.** Two pseuds on one account are one vote, because
governance counts accounts (§6.1). A vote is recorded as a ballot, and the ballot's choices are
recorded as rows, so the tally is a query and the evidence is auditable.

**The owner votes like anyone else.** The owner's structural advantage is the seed floor, not an
extra ballot — a second vote would make the community's number decorative.

**A vote is a statement about the theme, not about a work.** It carries no work id, no rating, no
reading history — only the theme, the tag, and the choice. So it is publishable (the *tally* is
public), it is not a signal about a reader (§15.2's rule holds), and it can be shown to anyone
without exposing anyone's taste.

### 12.12.6 What is published

- The **tally** — each tag's approval percentage, the voter count, the period, the applied
  weight and the computed weight — is public on the theme's channel page and in
  `/api/v1/channels/{slug}`. A theme's weight profile is an editorial object; hiding it would
  make it a private preference, and §12.10 already says a channel is public.
- The **individual ballots are not public.** The aggregate is, with the same k-anonymity floor as
  the snapshot exchange (§15.3): a tag's approval percentage is not published when the number of
  voters who expressed an opinion on it is below `k`. A percentage computed over two people is
  not a percentage, it is a rumour.
- The **seed** is public. It is the owner's, and the owner published it.

### 12.12.7 The catalogue and the taste service

The applied weights are **versioned in the catalogue export** (§7.3's `themes.jsonl` gains
`effective_weight` per tag), so a recommendation made last week is explainable against the weight
profile that produced it, exactly as the instance theme is versioned (§12.1).

The taste service **does not compute the tally** — it consumes the applied weights, like every
other published artefact. The tally is a database operation; the blend is a vector operation; the
two never meet.

### 12.12.8 The failure modes this design is defending against

| attack | the defence |
|---|---|
| a small group votes a tag to 100 % and captures the theme | the owner's 20 % floor is permanent; the weight can move at most 25 % of the remaining distance per period; and the *membership* of the theme is a quorum decision, not a vote |
| blanket approval to flatten the theme | it flattens but does not invert; the ordering survives at 1/5 strength, and the compression is visible on the channel page |
| a vote becomes a trust farm (vote, then register) | votes are per account; the gate is trust/resonance/supporter; the fold grants tenure and never score (§14.2) |
| a vote becomes a *personal* signal | a ballot names a theme, a tag and a choice. It has no work id, no rating and no reading history in it, so it is not a fact about a reader and does not leave the instance (§15.2) |
| the community's weight and the house taste contradict each other | **one mechanism, one place to be wrong** — the theme channel's vector *is* the voted vector; the house taste is a separate signal that happens to point the same way. They are never summed into a third number |
| the published theme becomes a target for a hostile group | the owner retains publication and membership; a theme can be **withdrawn from publication without deleting a single vote**; the moderation of a published theme is a quorum decision like any other |

---
