# Design Gaps Review — Ideas Missed (2026-09-22)

Status: **review draft — proposed, not adopted.** Nothing here has been added
to `docs/spec.md`, `docs/requirements.csv`, or any ADR. This document exists
so each idea is triaged explicitly rather than adopted or rejected by
omission. It follows the same pattern as `docs/spec-gaps-ficnexus.md` and
`docs/spec-gaps-gravity.md`.

How this was produced: an external design review of the spec's discovery,
incentive, and supply mechanisms. The reviewer's framing: most of the design
so far shapes what the community sees; the bigger gaps are the operator's own
attention (the input everything calibrates against), keeping the right
authors writing, side effects of the incentives themselves, and legal
groundwork for an EU operator. Fixes to existing design come first because
they affect everything else.

---

## A. Fixes to what's already designed

### A1. Resonance rewards imitation, not scouting

Overlap with the operator's exemplars pays people for bookmarking what the
operator already liked. That is circular — gravity already surfaces those
works — and copyable if activity ever leaks.

**Proposed:** replace overlap with **scout value**: credit engagement with
works that later earn the operator's high rating or vanguard consensus,
weighted by how obscure the work was at the time. Bookmarking at 3 readers
counts far more than at 300. Copying cannot game it, and the community
becomes a search party for the operator's taste.

### A2. Incentivized engagement pollutes ranking signals

Reading-club bonuses, topic bonuses (§0.4.1), bounty-driven reads, and taste
notifications all inflate completions and bookmarks on exactly the works
gravity pushed. Credits then indirectly write to ranking, breaking the spirit
of the negative invariant (§33.2: no influence purchased).

**Proposed:** tag incentivized interactions at write time and discount them in
ranking, quality multipliers, and resonance. Cross-check credit transfers
against pins and nominations so pay-for-pin with vanguards cannot become a
market.

### A3. Incentives invite generated fic

Anything paid for posting words, finishing chapters, or winning bounties can
be farmed with an LLM.

**Proposed:**
- Vest author credits on completion by distinct trusted readers, not on
  posting.
- Require authorship attestation for bounties.
- Make generated-content posture an instance policy (`forbid | disclose |
  allow`) — some instances will want to allow it.
- Never sanction on detector output alone; false positives land on real
  authors, and §34 already keeps classifiers away from sanctions.

### A4. Meta-ranker reward is sparse and self-confirming

The operator reads a handful of fics a week, yet most of the `admin_aligned`
metric depends on them. Most impressions produce no signal, so the bandit
optimizes a proxy, and proxy users mostly engage with what gravity already
surfaced.

**Proposed:**
- A small slice of truly uniform-random slots (the only data with exactly
  known propensities).
- **Log selection probability per ranked slot** so inverse-propensity scoring
  can give unbiased estimates.
- Interleaving for head-to-head strategy comparisons.
- Delayed-reward handling (a 150k-word fic takes weeks to finish).
- Denser signal from the operator (Section B).

### A5. Works have no coordinates

The taste vector has axes like pacing and prose density, but nothing in the
plan measures them.

**Proposed:**
- **Base layer:** deterministic stylometry (sentence-length variance,
  dialogue ratio, vocabulary richness, chapter length), confirmed tags, and
  one-tap reader micro-surveys at end of work.
- **Optional layer:** local embeddings via Ollama, only where the author's
  Milestone 27 permission statement allows ML processing. In fandom this is a
  trust question as much as a technical one.

### A6. No exposure floor

Authors outside the operator's taste get no visibility and leave — and some
of them would eventually have written something the operator would love.

**Proposed:** guarantee every new work a minimum number of impressions
regardless of taste. This doubles as unbiased exploration data (ties to A4).

### A7. Tag gravity makes tag-stuffing profitable

Only reader- or wrangler-confirmed tags should count toward gravity. Cap how
many tags per work can contribute, and let readers flag inaccurate tags into
the Milestone 29 wrangling queue.

### A8. Moderation trains the taste profile

Reviewing reported works (often things the operator dislikes) logs reading
time at maximum signal weight. §16.2 already excludes moderation sessions;
verify enforcement covers reading-time signals, not just explicit likes.

### A9. Taste leaks through public incentives

Standing-bounty criteria, public topics, vanguard selection, and
"your work matched a bounty after admin rating" notifications each reveal
part of the lens. Deliberate disclosure is fine; accidental disclosure is
not.

**Proposed:**
- A leakage view on `/admin/discovery` showing what an observant user could
  infer from public artifacts.
- Attribute payouts to "instance" and batch them rather than tying them to a
  rating event.
- Update the owner-visible resonance label only from a weekly batch, in
  coarse buckets — near-real-time feedback turns it into an oracle for
  probing the operator's taste.

### A10. "Stuck without explanation" breaks honest verification

Trust-coupling UX and Milestone 29's "why am I seeing this" pull in opposite
directions. Disclose *that* curation exists, never the lens itself — like a
bookstore's staff picks. Explanations can be truthful at coarse grain
("editorial pick," "trying something new," "readers with similar bookmarks")
without revealing a single dimension.

### A11. The economy will inflate

Faucets have multiplied: curation, referrals, topic bonuses, standing
bounties, lifecycle multipliers.

**Proposed:** faucet/sink dashboard; rule of thumb — faucets pay for signal or
supply, sinks convert credits into supply (bounties, translation jobs,
podfic/TTS, mirroring). Credits stay closed-loop and never cashable.

### A12. Cross-instance credits don't exist

Each instance has its own ledger, so "100 credits per instance where your
algorithm is promoted" cannot be paid, and reporting performance upstream is
telemetry. Marketplace reputation should come from install counts and opt-in
aggregate reports instead.

---

## B. The operator's attention is the scarcest input

Every mechanism calibrates against the operator, and the operator can only
read so much.

### B1. Tasting menu

A swipe-style calibration queue: summary plus a random 300-word passage,
rated in seconds, with reason tags ("prose," "characters," "pacing," "trope
execution," "not for me: ___"). Pick items by uncertainty (active learning)
so each minute teaches the model most. Prose style shows in an excerpt, so
80k words need not be read to label it.

### B2. Bring your history

Import AO3/FFN/Goodreads bookmarks, kudos, and history through existing
source credentials — hundreds of labeled examples on day one. Offer the same
to every user. Beats a quiz for cold start; each imported bookmark can pull
in the work (subject to the import caveat in G).

### B3. Structured DNFs

When a work is abandoned, one tap records why: pacing, OOC, tense-hopping,
abandoned WIP, wrong mood. DNF reasons are the most informative negative
signal available. Private by default; shared with authors only in aggregate
and only where they opted into constructive feedback (§8.4).

### B4. Personal concierge

A private queue ranked by predicted enjoyment, with a session selector
("comfort read," "wreck me," "I have 20 minutes"). Uses moods from the
taxonomy and reading speed from progress sync. Add "tell me when this WIP
completes" for fics being saved.

### B5. Define the objective, then attribute it

North star: works the operator rates per month, plus time-to-find. Attribute
each loved work to the mechanism that surfaced it (scout, bounty, import,
translation, vanguard pin, probe, keystone author). This tells the operator
which features deserve effort, and is the correct reward signal for the
meta-ranker (A4).

### B6. Reason-tagged kudos and line highlights, for everyone

Optional reason on kudos ("loved the prose") and highlight-to-react on a
sentence — "THIS LINE" is already fandom's most common comment. Authors get
specific praise (a positivity-first win); the operator gets dense signal
about prose taste.

---

## C. Supply: more of the right fics written, finished, and found

### C1. Keystone authors

Identify the handful of authors whose work the operator consistently loves.
Route feedback their way, catch their droughts early, offer beta matching and
opt-in credit patronage. One prolific keystone author can matter more to the
goal than hundreds of users, and none of it touches ranking.

### C2. Feedback-drought intervention

Authors quit from silence far more than from criticism.

- Detect works with strong completion but few comments and route readers to
  them.
- A welcome rota guarantees every first work a genuine comment within 72
  hours.
- Small bonus for commenting on under-commented works.

This is the spec's founding mission ("keeps authors writing") made
operational.

### C3. Exchanges, prompt memes, big bangs

Fandom's highest-yield production engines: Yuletide-style gift exchanges with
a matching engine, anonymous prompt/fill boards, writer-plus-artist big
bangs, drabble challenges. The operator participates as prompter or recipient
like anyone else. Bounties are transactional; exchanges are social, and
fandom strongly prefers the latter.

### C4. WIP adoption and closure notes

Let authors mark abandoned WIPs "up for adoption" (existing fandom practice)
or publish their planned ending so readers get closure. Credit closure notes,
with lineage from Milestone 27. Goes straight at the abandoned-at-chapter-4
problem.

### C5. Finisher signals

Show update cadence ("updates about every two weeks") and completion history
("finished 9 of 10 works"), with a Finisher badge. Positive framing only;
never label non-finishers. Authors can optionally enable completion pledges
through existing reserve/capture escrow. Off by default — "update pls"
pressure is considered rude.

### C6. Cross-language supply

The fics the operator would love most may be in languages they don't read;
pixiv, LOFTER, Ficbook, and the Spanish- and Portuguese-language communities
are enormous. Taste-score foreign works (tags map across languages; stylometry
partially transfers), then fund translation bounties with author permission.
Priority 7 becomes a supply engine for Priority 2.

### C7. Fandom-blind discovery

Works readers finish with no other activity in that fandom are accessible
without canon knowledge. Surface them to widen reading into unknown fandoms;
AUs are the usual suspects.

### C8. Trend radar

Spot emerging fandoms early (new canon releases, search spikes, first
imports) and time events and bounties to catch the wave. Multi-dimensional
profiles transfer across fandoms, so taste applies before anything has been
rated there.

### C9. Beta and specialist matchmaking

Betas, Britpickers, canon consultants, and sensitivity readers — all
credit-rewarded and opt-in. Fits positivity-first perfectly: critique goes
only where requested.

### C10. Fic Finder bounties

"Looking for a fic where…" is fandom's most common post. Pay whoever
identifies a half-remembered fic, using full-text search, optional semantic
search, and reverse media search.

### C11. Rec blurbs as second summaries

Plenty of great fics hide behind "I suck at summaries." Show the best rec
note and a short excerpt beside the author's summary in results, so prose
quality is visible in five seconds.

---

## D. Extending the link-rot idea: identity over location

A URL records *where* something is, and locations rot. Also record *what* it
is.

### D1. Descriptions as last fallback

Curate one description per MediaReference ("faceclaim: silver-haired woman
in a red coat, 1940s film still"). If every link dies, readers still get the
description. Screen readers and TTS editions can narrate it; it is written
once per fic using that image. Pay for it with a curator bounty.

### D2. Stable identifiers

- **Songs:** MusicBrainz IDs/ISRCs, resolved to whatever platform the reader
  uses (song.link-style).
- **Playlists:** stored as tracklists, so a deleted Spotify playlist loses
  nothing.
- **Faceclaim actors:** Wikidata IDs, so replacement images can be found.
- **Canon references:** ISBN/TMDB IDs.

### D3. Links in text rot too

Author's notes pointing to Tumblr meta, soundtracks, and reference posts
should go through the same snapshot-and-monitor pipeline.

### D4. The instance itself is a link

Self-hosted instances die when operators burn out.

- Mutual backup agreements between instances.
- A dead man's switch that exports public works to a successor instance or
  archive.
- Persistent work IDs with a redirect registry (plus ActivityPub Move for
  accounts).
- Author-notified bulk export.

---

## E. Growth and influencers

### E1. Launch narrow

Gravity and scouting need density. Start in one or two fandoms where the
operator and founding cohort overlap, then widen.

### E2. Curator lenses

Invited rec-bloggers and influencers publish an opt-in lens: the archive
re-ranked by *their* taste profile, with attribution. "Browse the archive
through X's eyes" is a stronger pitch than referral credits — their curation
gets a permanent home and their audience follows. The operator chooses whom
to invite, so their engagement enriches the model too. The operator's own
lens stays invisible.

### E3. Rec-post attribution

Share links carry the sharer's code. When a visitor arrives and finishes a
work or registers, both sharer and author earn credits. Rec posts are already
how fandom promotes things; this rewards existing behavior.

### E4. Claim your works

Where an author's works already exist here as metadata with readers and
bookmarks, invite them: "Claim your 23 works — 400 readers are here." Verify
with a code on their AO3 profile; pair with an exclusion registry for
opt-outs.

### E5. Meet fandom where it lives

A Discord bot (rec commands, per-fandom new-work feeds, account linking that
carries referral attribution) and share templates for Tumblr and Bluesky.

### E6. Sister instances

Let instances subscribe to each other's public picks over ActivityPub, with a
directory searchable by public topics (§0.4).

---

## F. Flexibility for operators

### F1. Taste as a versioned, blendable object

Version history with rollback, export/import, per-fandom profiles, and blends
across co-admins or a collective. Add a governance mode (operator / council /
community vote) for instances that want the community to own the lens. This
also serves as a succession plan.

### F2. Preview before apply

Let operators "view as" a user or persona and preview discover and search
under a proposed config before saving, with config history and rollback. With
this many knobs, preview is what makes flexibility safe.

### F3. Scale-aware activation

Many instances will be 20 friends. Quorum needs quorum, the bandit won't
converge, and a "top 10%" vanguard is two people. Mechanisms should declare
the minimum activity they need and stay dormant (or fall back to simpler
behavior) until the instance reaches it — giving small instances a sense of
unlocking features as they grow.

### F4. Hardware-aware presets

Plenty of self-hosters run a $5 VPS or a Raspberry Pi. Perceptual hashing,
link checking, WASM, embeddings, and resonance recomputation all cost CPU and
bandwidth. Presets need resource tiers that degrade gracefully — SQLite with
weekly batches and no embeddings at one end, Postgres plus Redis with
incremental updates at the other.

### F5. Variety within the aligned set

The diversity valve handles taste-distant content, but ten near-identical
slow burns in a row is its own failure. Re-rank with maximal marginal
relevance and model satiation, so three angst fics in a row nudges the next
pick toward fluff.

### F6. Scheduled gravity

Time-boxed configs for events and seasons ("October: horror").

---

## G. Legal groundwork for a Spain-based operator (verify with counsel)

### G1. GDPR

- The hidden resonance score is inferred personal data. It must be disclosed
  on access requests, and the profiling purpose belongs in the privacy
  notice. If legitimate interest is the legal basis, users can object.
  Invisible in the UI is fine; invisible to an access request is not.
- Erasure must cascade through resonance, caches, media references, and
  derivatives.
- A stated policy on whether private-library copies survive an author
  deleting a work.

### G2. DSA

- Notice-and-action (Art. 16) and statements of reasons (Art. 17) apply to
  hosting services of any size.
- Art. 17 explicitly covers demotion and visibility restrictions imposed for
  ToS reasons. That collides with the spec's "shadow" sanction tier;
  deceptive high-volume commercial spam is carved out.
- The media section's DMCA endpoint should become a DSA notice flow, keeping
  DMCA intake for US claimants.
- Taste ranking is not a ToS-based restriction, so core gravity is likely
  outside Art. 17.

### G3. Imports

If cached third-party fic bodies are publicly readable without the author's
consent, that is the single biggest reputational risk in fandom — reposting
without permission is a cardinal sin — and it is copyright exposure. Safer
default: the public sees metadata and a link; the cached body stays in the
importer's private library until the author claims the work or grants
permission.

### G4. Monetization

EU copyright has no US-style fair use, so paying for fic itself is riskier
than AO3's non-commercial footing. Charge for infrastructure (compute, TTS,
translation), not fic.

### G5. RPF

Add an instance policy knob. Spain's honor and image rights (LO 1/1982) make
explicit RPF, and real-person faceclaims on explicit works, riskier than in
the US.

### G6. Built-in tooling

Every operator is legally responsible for their own instance, so shipping
compliance tooling (notice forms, statement-of-reasons templates, DSAR
export) is a feature.

---

## H. If you only take five

1. **Scout value** instead of overlap-based resonance (A1).
2. **Tasting menu + history import** to densify the operator's signal (B1,
   B2).
3. **Feedback-drought intervention + keystone authors** to keep the right
   people writing (C1, C2).
4. **Generated-content policy + credits that vest on reader engagement**
   (A3).
5. **North-star metric with per-mechanism attribution** (B5).

Two things are cheap now and impossible to retrofit: **logging selection
probabilities per ranked slot** (A4) and **tagging incentivized interactions
at write time** (A2). Without them, none of the historical data can be used
for unbiased evaluation later. Before launch, also settle the GDPR/DSA
pieces (G1, G2) and the public-visibility default for imports (G3).
