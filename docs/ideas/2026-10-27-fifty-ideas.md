# Fifty ideas toward one goal

**The goal, stated once so every idea can be measured against it:** maximise the amount of *quality
system / progression-fantasy fanfiction the operator would actually enjoy reading*. Everything below is
either (a) more of those stories existing, (b) the right ones finding the right reader, or (c) the
machinery that lets readers and authors trust each other enough to keep doing it.

Three notes on reading this:

- `✓` means the mechanism already exists in the tree (or as a written plan) — the idea is then an
  extension, not a new build. `§` cross-references the spec section that owns the area.
- Ideas 24–34 are the curation engine you described in full (reader curation, quorum, trust override in
  one direction only). It is the highest-leverage cluster here, because every other filter is only as
  good as the metadata underneath it.
- Ideas 43–50 belong to Ascend rather than Darkhold. They are written here because the two products share
  the flywheel — builds are a funnel into fic, fic is a funnel into builds — and because the trust and
  quorum engine is the same engine in both.

---

## A. The funnel: daydreamers become writers (1–7)

**1. A daydream workshop that is allowed to be private.** A scratch space where a reader keeps premise
fragments, power-system rules and scene ideas, with one button: *turn this into a draft*. Most
progression-fantasy writers start as readers with a private fantasy; the site currently asks them to
become a writer before it gives them anywhere to put the fantasy. Private-by-default removes the fear that
killed the last hundred attempts.

**2. A progression-fantasy ontology that is actually specific.** `system`, `litRPG`, `cultivation`,
`dungeon core`, `regression`, `time loop`, `portal fantasy`, `training arc`, `power fantasy`,
`progression with cost` as first-class canonical kinds, each with aliases, rather than `fantasy` + free
text. You cannot filter for what you actually like until the vocabulary names it, and authors cannot tag
what the taxonomy does not offer.

**3. A taste calibration at signup, not after.** Six to ten forced choices between real works
(a `✓` seed catalogue already exists for the arena). Output is a cold-start taste vector, so the first
Discover page is not a list of everything. This is the difference between a funnel that converts on day
one and one that loses the reader at an empty shelf.

**4. A "daydream journal" on the reader surface.** While reading, a reader can note what they imagined
happened next. Export it as a prompt (idea 5) or keep it private. This is the cheapest possible on-ramp:
it converts consumption into intent without asking for a plot.

**5. A prompt board with claiming.** Prompts are posts with a `claim` action; a claim reserves the prompt
for N days and publishes a draft stub so two people do not write the same story in silence. Progress
becomes visible, which is the only thing that makes prompt boards work after the first week.

**6. A fic → build bridge, and a build → fic bridge.** A work's power system can be exported as an Ascend
build template (idea 50 reversed), and a build can be published as a "story prompt with mechanics". The
same reader is on both sides; today the two products pretend the other does not exist.

**7. A landing page that answers "what is this" in one screen.** Hero: *Read, write, and keep the
progression fantasy you keep daydreaming about.* Then three live proofs — a work, a build, a reader's
shelf — and a single primary action. The current landing is honest but abstract; a funnel needs a
concrete next step.

## B. Authors: getting them, keeping them (8–17)

**8. Import-and-claim for existing works.** Paste an AO3/FFN/SpaceBattles/QQ/Royal Road URL, get a
private import with full attribution and a canonical-original link, then a `claim` flow to prove
authorship. Every author who already writes this genre is publishing somewhere; the onboarding cost is
the whole competition.

**9. Rewards that pay for retention, not for applause.** XP from *completion* and *return* (chapter N+1
read by someone who read N) rather than raw kudos. Kudos rewards the first paragraph; retention rewards
the story. Anything that is farmable by a mailing list is not a quality signal.

**10. A serial streak with grace days.** Writing streaks are the strongest known habit loop for serial
authors, but a broken streak ends the hobby. Bank up to three grace days, earn more by finishing arcs.

**11. Reader-funded bounties on stalled works and prompts.** Escrow a small recognition stake; it releases
on publication of the promised chapter or a good-faith attempt. This is the only mechanism I know that
converts a reader's "I need more of this" into an author's "I have a reason to finish".

**12. Co-writer matching for system design.** Progression fantasy dies in bookkeeping: tables, costs,
levels. Match a prose writer with a systems designer (some people genuinely prefer building the rules).
Half the abandoned serials in this genre are abandoned by people who loved the idea and hated the ledger.

**13. Reciprocal critique matching.** Opt in, get matched with one other author, each reviews the other's
opening three chapters within a week. Progression authors quit for lack of feedback far more than for
lack of readers, and a structured obligation beats a vague invitation to comment.

**14. Badges that unlock work, not decoration.** Milestone rewards should be *useful*: a curated
first-arc review, a featured slot, a cover commission pool, a slot in a themed collection. Cosmetic-only
gamification is a thing people stop noticing in a month.

**15. Invite seats that decay.** `✓` invites exist; make each invite create a seat that yields reward only
while the invitee is active (publishes or reviews within 90 days). Rewards quality recruitment and
punishes the "invite 200 strangers for points" failure mode by construction.

**16. An author dashboard that shows where readers leave.** Per-chapter retention, drop-off point, and
the single most common exit chapter. Authors cannot fix what they cannot see, and this is the most
actionable number the site can hand them.

**17. A beta-reader roster with declared specialties.** Progression mechanics, pacing, prose, dialogue,
tagging. Opt-in, with a request queue. Reviewers get reputation for the reviews that authors mark as
having changed the draft.

## C. Invitations and the social loop (18–23)

**18. Guest-first identities.** `✓` GS-1: a guest can read, comment and rate, then claim everything on
signup. Every account created before the decision to commit is a reader the funnel keeps instead of loses.

**19. An invite tree with capped, decaying upstream rewards.** Every account records its inviter; rewards
flow up two levels at most and halve each hop. Rewards the person who found a good author, not the person
who built a pyramid.

**20. Two-vouch invitations.** A new author needs two established accounts to vouch. Halves the spam
surface, doubles the social buy-in, and gives the invitee two people who will actually read chapter one.

**21. "Bring your fandom" campaigns.** Seed a fandom with twelve half-decent works and invite its authors
as a cohort, not one at a time. Authors move in groups or not at all.

**22. Referral rewards paid in visibility.** A featured slot, a collection front page, a review — not
points. Visibility is what a writer actually wants and what the site can grant without inventing currency.

**23. Anti-farming by definition.** Invite credit only counts once the invitee publishes a work or writes
a review another user marks helpful. Any reward keyed to *signups* will be gamed within a week.

## D. Curation: reader tags, quorum, and one-directional trust (24–34)

This is the cluster you asked for. The rule that makes it work: **every curation decision is a row that
records who decided, at what trust level, and why — and a higher level may supersede a lower one, never
the reverse.** Nothing is deleted; the newest decision at the highest level wins, and the history stays
readable.

**24. Tag provenance on every tag.** `author` / `community-confirmed` / `community-unconfirmed` /
`disputed` / `inferred`, rendered as a small glyph and filterable. A reader deciding whether to trust a
`harem` tag needs to know whether the author said it or twelve readers did.

**25. Quorum scaled by blast radius.** `✓` §14.7 already sets per-action quorums; extend the same table
to tags: adding a harmless tag 1, removing an author tag 2, changing a genre-defining tag 3, tagging
`underage` or `non-con` 2 *and* never auto-applied. The blast radius is the number of readers whose
filter would silently change.

**26. The override ladder, enforced at write time.** A decision carries the tier that made it. Writing a
decision at tier T requires T ≥ the tier of the decision it supersedes; the reverse is refused with a
typed error, not a warning. Lower-tier disagreement is expressed as a *contest* (idea 28), never as an
edit. This is the one place to be strict in the schema, because a soft rule here becomes an edit war.

**27. An override is a courtesy event.** Overriding a decision notifies the person who made it, requires a
one-line reason, and records both in the modlog. `✓` §14.6 modlog exists. Without the notice, the
volunteer whose tag was replaced learns that their work is invisible and stops; the reason is what makes
curation a collaboration instead of a hierarchy.

**28. Contests instead of reverts.** A tag can be contested, which opens a quorum vote with a deadline
instead of flipping the tag back and forth. Services rot on edit wars; a deadline and a tally end them.

**29. An author veto with a budget.** Authors can reject community tags on their own work — but a veto
that is overridden by quorum three times costs the veto for 90 days. Authors get the benefit of the doubt
and keep the ability to correct, without being able to hold metadata hostage.

**30. Confidence-ordered display.** Tags sort by `(agreement − disputes)`, low-confidence ones collapse
behind a "show community tags" control. Readers see the consensus, curators can still see the frontier.

**31. Per-facet curation reputation.** Someone excellent at mood tags is not automatically good at content
warnings. Reputation accrues per facet (mood, trope, content warning, character, relationship) and quorum
counts facet-specific standing. One global number makes every specialist look average.

**32. A golden set to measure curation against.** 200 works tagged by people who know the fandom, used to
score new decisions and any model assistance. Without it, "curation is working" is an opinion, and the
first bad week is invisible.

**33. Taxonomy changes are governed acts.** A new canonical kind (idea 2) is a level-3 action with quorum
2, and it lands with aliases and a migration note. Taxonomies that grow by accretion become unusable
exactly when they become valuable.

**34. Keyboard-driven bulk review.** A review queue that takes `j/k/a/r/u` and applies one decision to a
selection, with undo. Quorum requirements are only affordable if the act of curating costs seconds, not
minutes — otherwise the queue becomes the reason curation stops.

## E. Filtering and discovery (35–42)

**35. Facet algebra, not a form.** AND across facets, OR within a facet, negation, and `must not` — the
same grammar as `✓` §11's query language, extended to curated facets. "Cultivation, not harem, finished,
80k+, female lead" is one line, not five dropdowns.

**36. "More like this, but …".** Similarity over curated facets with overrides: nearest neighbours of a
work, then subtract `slow burn`, add `completed`. This is the single most-used discovery gesture on sites
that have it, and it needs the curation engine of section D to be worth anything.

**37. One filter grammar across fics, cards and decks.** The same facet machinery on Ascend builds
(idea 45): `power:regression`, `companion:healer`, `world:dungeon`, `cards<=40`. Shared grammar means one
query UI, one index, and one mental model for the reader who uses both products.

**38. Saved searches with alerts.** "Tell me when a completed 100k+ system fic with a female OC appears"
is a subscription to the future of the catalogue. This is how a small site beats a large one on
freshness: the reader does not need to poll.

**39. Bayesian scores everywhere.** Rank by the lower bound of a rating's credible interval, not the mean.
One five-star vote cannot outrank forty four-and-a-half votes, and a new work with three enthusiastic
readers is not buried under it.

**40. Private reader facets.** Let a reader tag works for themselves ("read on the train", "comfort",
"abandoned at ch. 12") and filter by them. Free metadata, zero moderation cost, and it makes the shelf
feel like theirs.

**41. Slop filters that are on by default.** Detect machine-generated prose, machine-translated dumps and
stub works that stop mid-sentence, and let readers exclude them. Volume is not the scarce resource in
2026; a reader's attention is.

**42. A facet coverage meter.** Per work and per fandom, show how complete the metadata is, and put the
worst-covered works in the curation queue first (idea 34). Curation effort should follow reader traffic,
and this is what makes that visible.

## F. Ascend: builds as a showcase, not a Discord message (43–50)

**43. Import a build from Waifu Catalog or any CYOA export.** Three entry points, one canonical result:
paste the text export, upload the image export, or give the link. A parser extracts the structured parts
(perks, drawbacks, companions, worlds, points, card lists) where it can, stores the original artifact
verbatim where it cannot, and marks the parse confidence. The image is the source of truth for a build
nobody can re-derive; the structured form is what makes it searchable.

**44. A build gets a page, a URL and an OG card.** Today a build lives in a Discord channel and dies with
it. A canonical URL with a generated card (the `✓` OG worker exists) makes it linkable, shareable and
indexable — which is the entire difference between a showcase and a chat log.

**45. Granular curated build search.** Facets: skill, perk, companion, starting world, power archetype,
drawback, deck size, card rarity. Curated by users with the same trust/quorum engine (section D) —
`✓` §36 and §37 plans already sketch the substrate; this is the search surface over it. "Builds with
these five skills and a healer companion" is the query that makes a build catalogue useful instead of a
gallery.

**46. Scoring with more than one axis.** `would play` (intent), `consistency` (does the build actually
hold together under its own rules — reviewers vote), and comment sentiment as a tiebreaker, all Bayesian
(idea 39). A single like button measures popularity; a build catalogue needs *quality of the build*.

**47. Threaded comments with structured critique.** Comments can carry a facet (`synergy`, `balance`,
`lore fit`, `rules`), and the facets aggregate into the build's page. Feedback becomes comparable across
builds instead of a pile of "cool build".

**48. Trending builds on the landing page.** Two rails and one rule: *new and rising* (score per hour with
a 48-hour half-life) and *all time* (Bayesian score, minimum sample), with a diversity cap so one prolific
author cannot own the front page. The landing page is the only place a visitor sees what the community
is proud of this week.

**49. Build forks with lineage.** A build can be forked ("inspired by"), inheriting facets and crediting
the parent. Progression-build culture is already remix culture; making the lineage visible rewards the
original builder and gives the fork a starting point.

**50. Attribution, claims, and a build → fic conversion.** Every imported build carries its source, a
license field, an author claim path, and a removal-on-request path. Then the reverse of idea 6: publish a
build as a story prompt with mechanics attached, and let a fic inherit the build as its canon. This is
the loop that makes both products worth more than they were apart.

---

## If only five things got built

Ordered by how much of the goal each unlocks per unit of work:

1. **26 + 25 — the override ladder and per-field quorum.** Everything in discovery and filtering is
   downstream of trustworthy metadata, and this is the part that is a schema decision, cheap now and
   expensive later.
2. **35 + 36 — facet algebra and "more like this, but".** The reader-facing payoff of the same work.
3. **8 — import-and-claim.** The only idea that adds supply of exactly the stories the goal is about,
   using work that already exists elsewhere.
4. **43 + 44 — build import with a real page.** Turns an existing Discord community's output into
   indexable, linkable content, and is the Ascend funnel's entry point.
5. **48 — trending on the landing page.** Cheap, and it is the difference between a site that looks alive
   and one that looks abandoned.

The rest are worth having, in the order the graph suggests: curation tooling (34, 42) before more filters;
retention rewards (9, 10) before more badges; contests (28) before any appeal process.
