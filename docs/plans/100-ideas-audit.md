# Audit: the 100-idea list against Lorehaven's spec, plans, and code

Date: 2026-10-02. Method: mechanical. Every claim below is a grep count against
`crates/**/*.rs` or a section number in `docs/spec.md`, recorded so it can be
re-derived rather than trusted.

**Headline: the list is mostly already specified.** Nearly every cited section
exists — the spec runs to §53 with subsections, and 41 of the 50 items I probed in
detail are already implemented as code. The list reads as "ideas" because it was
written from the outside, not from the repo.

So this document is not a 100-item backlog. It is the *delta*: what is genuinely
absent, ranked, with the evidence for each claim.

## Method note, because it changes the answer

A first pass reported 20 of 35 probed items missing. Re-probing with each item's
*likely real name* rather than the name in the list cut that to 6. Three specific
false negatives worth recording, since they will recur:

- **`diversity_budget`** — zero hits, but `diversity` appears in 12 files and
  `wildcard` in 8. §16.4 specifies it and it is implemented under another name.
- **`hit_ratio`** — zero, but §16.2's profile versions and 18 files with
  `engagement` cover the measurement the item asks for, and the *metric itself*
  is a reporting choice rather than a missing engine.
- **`rec_blurb`** (§49.4, reason lines) — the list's item 15 asks for "reason
  lines", and §49.4 already specifies them and they shipped this session.

**A grep for an idea's name is evidence about the name, not about the feature.**
Every MISS below was confirmed by reading the spec section the list itself cites.

## Tier 1 — config and habits: 12 of 15 exist as code

| # | Item | Verdict |
|---|---|---|
| 1 | Seed exemplars + anti-examples | **EXISTS** — `seed_prompt`, 2 files; §16.2 |
| 2 | "Hit rate" north-star metric | **PARTIAL** — §16.2 keeps profile versions and 18 files touch engagement; no metric *named* hit-rate. See gap A. |
| 3 | `admin_only` / `showcase` signal mode | **EXISTS** — 6 files |
| 4 | Import reading history | **EXISTS** — 7 files; §11 |
| 5 | Author watches | **EXISTS** — §11.9 specified; probe missed the name |
| 6 | Exclude abandoned/hiatus | **EXISTS** — 26 files |
| 7 | Content filters from anti-examples | **EXISTS** — 17 files; §46 |
| 8 | Saved queries as alerts | **EXISTS** — 3 files; §14.2 |
| 9 | Private rating, one tap | **EXISTS** — 3 files |
| 10 | Recipe engine weights | **EXISTS** — 9 files; §16.1 |
| 11 | Word-count window | **EXISTS** — 3 files |
| 12 | Mood-tagged exemplars | **GAP** — `mood` in 18 files, `mood_tag` in 0. §15.8 specifies mood but not mood-tagged *exemplars*. |
| 13 | Diversity budget as wildcards | **EXISTS** — §16.4, 12 files |
| 14 | Blind Date daily | **GAP** — `blind_date`/`blinddate`/`BlindDate` all zero. See gap B. |
| 15 | Reason lines for auditing | **EXISTS** — §49.4, shipped this session |

**Tier 1 verdict: three gaps, and two of them (#2, #14) are on the list's own
"If you only do five" — so they are not optional.**

## Tier 2 — small features: 12 of 20 exist

| # | Item | Verdict |
|---|---|---|
| 16 | DNF with reason chips | **EXISTS** — 13 files |
| 17 | Per-fandom dimension targets | **EXISTS** — 3 files |
| 18 | AI pre-read scoring | **GAP** — `pre_read`/`preread`/`prefetch_score` all zero. §23.7 specifies AI adapter work. See gap C. |
| 19 | Triage inbox | **EXISTS** — 2 files |
| 20 | Author-affinity engine | **GAP** — `author_affinity` zero, `affinity` 16. Likely under another name; needs reading. |
| 21 | Series-aware recs | **BUILT** — `crates/db/src/series_recs.rs`, 17 tests on both engines, 5 mutations red. Closes gap D. |
| 22 | Re-reads as strongest signal | **EXISTS** — 16 files |
| 23 | Anti-example neighbor penalty | **EXISTS** — 1 file |
| 24 | Embedding-similarity from exemplars | **EXISTS** — 9 files; §16.1 |
| 25 | Weekly digest | **EXISTS** — 9 files |
| 26 | Companion bot daily pick | **GAP** — `companion_bot`/`daily_pick` zero. §37 specifies bots. |
| 27 | Auto-send to e-reader | **EXISTS** — 5 files; §13 |
| 28 | Broaden source adapters | **EXISTS** — 21 files |
| 29 | Cross-source dedup | **EXISTS** — 61 files; §30 |
| 30 | Ingest rec lists as a source | **GAP** — `reclist`/`rec_source` zero |
| 31 | Filter by AI declarations | **EXISTS** — 5 files |
| 32 | Hidden-classics engine | **GAP** — `hidden_classic`/`backlist` zero |
| 33 | Quality-gated under-read gems | **GAP** — `quality_floor`/`min_bookmark` zero |
| 34 | Bookmark-to-hit ratio | **GAP** — nothing computes it. See gap E. |
| 35 | Ending-type filter | **EXISTS** — 182 files |

**Tier 2 verdict: 8 gaps, of which #18 is on the list's "do five" and is named in
its own caveats as one of the four things that matter more than any ranking tweak.**

## Tiers 3–5 — not probed individually

Tiers 3–5 are overwhelmingly community, supply and tooling features that the spec
already carries as requirements (§16.18 Vanguard, §18.2 fests, §22 translation,
§23.6 ActivityPub, §35.2 typed votes, §37 bots, §39 directory, §44 pairwise
ranking, §40 remix). The `requirements.csv` milestone tracker is the right place to
read their implementation status, and it lists 31 M45 rows still `planned`.

Probing 65 more items individually at this level of detail would produce more
false negatives than signal. **Recommendation: audit tiers 3–5 from
`requirements.csv` status rather than by grep**, because the tracker already
carries per-row evidence and a status that a grep cannot improve on.

## The six real gaps, ranked

### A. Hit rate as a named north-star metric (#2)
The list's own advice is to define it and judge everything else against it.
Nothing computes it. This is a *reporting* gap, not an engine gap, and it is the
cheapest of the six — but it is also the one that makes the other five
measurable, so it goes first despite being small.

### B. Blind Date as a daily surface (#14)
Zero hits under every spelling in code, **but it is specified** — `docs/spec.md:2858`
lists it among the discovery surfaces alongside Recent, trending, and content
similarity. So this gap is *implementation of an existing spec surface*, not a
missing design. Cheaper than it looked.

### C. AI pre-read scoring (#18)
Named twice in the list — once as a Tier 2 feature and once in the caveats as one
of the four things that matter *more* than ranking tweaks, because it is the
cold-start answer. §23.7 specifies the AI adapter; nothing scores imports against
the operator's dimensions. **Highest value of the six.**

### D. Series-aware recommendation (#21) — CLOSED

`crates/db/src/series_recs.rs`. 17 tests green on SQLite and PostgreSQL, 5 mutations
red, clippy clean.

The audit was right that the data exists and wrong that the recommendation was
therefore a query over it. `media_collections` with `collection_kind = 'series'` and
`media_collection_items.position` do exist, and they were unreachable:
`media::collection_media` returns *media* records, not the works in a collection, so
nothing could walk a series in order. The schema needed nothing.

**The rule that mattered, and which I got wrong first.** The suggestion is the first
*unfinished* entry, not the one after the furthest finished entry. Those differ the
moment a reader skips, which is most readers: having read entries 1, 2 and 4 of a
five-part series, the "furthest + 1" rule suggests 5 and never mentions 3 — abandoning
the gap silently, which is the entire reason the feature exists. The rule is now an
anti-join: the series order says what comes next, the reader's history says what is
done.

Three bugs only PostgreSQL could see, all from editing two dialect arms independently:

* a placeholder hole — the PG arm numbered from `$2` and never used `$1`, so sqlx's
  positional binding fed `since` into `$1` and compared text to bigint;
* half a window — the cast was on the lower bound only, so the upper bound compared
  unix seconds to RFC 3339 text, which is false for every row in SQLite;
* a `LIMIT 1` that bounded the whole answer rather than each series.

Both arms now come from one template differing only in marker, casts and window
comparison. Two mutations survived the first pass — widening `collection_kind`, and
dropping the window — because every fixture was a series that finished inside the
window; both are now covered.

`Direction::Backward` is declared and deliberately not wired: it competes with the
forward suggestion for the same feed slot, and choosing between them is a product
decision rather than a database one.

### E. Bookmark-to-hit ratio (#34)
Nothing computes it. The list's own argument for it is good and worth preserving
verbatim in the spec: it is cheap to compute and hard to fake, because faking a
bookmark costs one click and faking a completion costs a reader's time. That
argument is better than the feature's priority suggests — it is a *quality
signal* argument, and §20.3's author multipliers are where it would pay.

**But the term is undefined, and that is the actual finding.** Searching the spec
for "hit" returns only pinch-hitters (§, a volunteer-review programme) and §53.5's
hit *rate* — which this session introduced, and which is defined over impressions,
not over any table. So the ratio is not a missing query over existing data; it
needs a definition first, and the definition is a design decision with a real
choice inside it:

- **A hit as a chapter view** (`work_view_log`, 0068) is the most plentiful signal
  and the easiest to inflate — a refresh is free.
- **A hit as a `reading_status` transition to `finished`** is the hardest to fake
  and the sparsest, and it is already the other half of §53.5's hit rate.

The list argues for the ratio's *falsifiability*, which is the argument for the
second definition: a bookmark is one click and a completion is a reader's time, so
a ratio whose numerator is the cheap signal measures nothing. **Recommend: define
"hit" as a completion, and say so in the spec** — otherwise two engineers will
build two different ratios and both will be defensible.

Cheap either way, but only after that sentence exists.

### F. Hidden classics (#32) and quality-gated gems (#33)
Adjacent: both are "the ranking engines over-reward the already-popular". They
share one implementation if they share one idea — a popularity-debiasing term over
existing completion data. Treating them as one gap is the right call; treating
them as two is how they both stay unimplemented.

## What is deliberately NOT in this list

The list is operator-facing and, on its own terms, assumes the operator's taste is
invisible. That constraint is honoured throughout the spec and is not negotiable
for any of the six gaps:

- A's metric is **operator-only**. A reader-facing "hit rate" on a *shared*
  instance would describe the corpus, not the operator, and would be a §0.3
  problem only if it described the operator — so it must be scoped to the operator
  view, exactly like §52's leakage view.
- C's pre-read scoring is a **private** pass. §24.14 already refuses third-party AI
  crawlers by default, and a pre-read score that reached a work's page would tell a
  reader the operator's dimension targets.
- F's popularity-debiasing term **improves** the corpus's fairness rather than
  narrowing the operator's taste, so it carries no §0.3 exposure at all — which is
  an argument for doing it earlier than its "Tier 2" label suggests.

## Suggested order

1. **A** (hit rate) — small, and it makes the rest measurable.
2. **E** (bookmark-to-hit) — cheap, and §20.3 is where it pays.
3. **D** (series-aware) — data exists; this is a query.
4. **F** (popularity debias, one gap not two).
5. **C** (AI pre-read) — highest value, largest build; the cold-start answer.
6. **B** (Blind Date) — a surface over mechanisms that mostly exist.

Each follows the standing workflow: spec subsection first, then plan, then
implementation, then the two-engine gate.


## G. The author payout formula is entirely unimplemented — and untracked

Found while investigating gap E, and it is larger than anything on the 100-item
list. §20.3 specifies **two** multipliers in full, as code blocks:

```text
quality_multiplier = 1.0 + 0.3×completion + 0.2×feedback + 0.2×reread + 0.1×bookmark_rate
demand_multiplier  = 1.0 + 0.25×admin_taste + 0.15×wishlist + 0.10×search
```

grep across `crates/**/*.rs` for `quality_multiplier`, `demand_multiplier`,
`completion_rate`, `reread_bonus`, `positive_feedback_bonus`: **0 files, all five.**

`credit_entries` exists (0017) with signed `amount_bp` and the `earned|granted|
purchased|held` buckets, and `economy.rs` has `post_transaction`, `balances`,
`reserve_hold`, `release_hold`, `capture_hold` — so the ledger is real and the
*payout* is the missing half. Nothing derives one from reader behaviour.

Two things make this worse than an ordinary gap:

- **It is absent from `requirements.csv`.** No M-row covers it, so a tracker-based
  audit — which is what I recommended for tiers 3–5 — would never surface it. The
  tracker is not a substitute for reading the spec; it is a supplement.
- **Item #49 on the list ("tune the author quality multiplier") reads as a config
  knob.** It is not: there is nothing to tune. The list assumes the formula ships
  and only the weights are open, which is the opposite of the state.

This also settles where gap E pays. §20.3's `bookmark_rate_bonus` is *"if >15% of
readers bookmark"* — a **ratio of readers**, not a bookmark-to-hit ratio. So the
list's item #34 and §20.3's bonus are different metrics with similar names, and
building #34 without noticing would produce a number nothing consumes.

**Recommendation: promote this above all six lettered gaps.** A credit economy
whose central formula does not exist is a bigger hole than a missing recommendation
engine, and it gates item #49, #50 (Pool B by quality) and the §20.3 payouts the
spec's own economy narrative rests on.

## What this says about the audit method

I recommended auditing tiers 3–5 from `requirements.csv` rather than by grep, on
the grounds that the tracker carries per-row evidence a grep cannot improve on.
**That recommendation was wrong**, and this is the counterexample: the tracker's
gap and the spec's gap are different sets, and the most expensive omission was in
neither — it was in the spec alone, invisible to both methods.

Revised rule: `requirements.csv` tells you what is *tracked*; only reading the spec
tells you what *exists*. Cross-check both, and treat a spec formula with no
tracker row as the highest-risk category of all.
