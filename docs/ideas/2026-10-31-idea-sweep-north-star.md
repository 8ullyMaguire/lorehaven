# Idea sweep — operator taste, user-driven governance, federation, preservation, power users

**Date:** 2026-10-31
**Method:** generated against the live schema (`migrations/sqlite/`, `crates/app/src/routes/`), the existing
idea dumps, and `docs/requirements.csv`. Not a wishlist from outside the repo: every idea names the
object it attaches to (pseud, work, chapter, tag, topic, standing, credits, instance peer).
**Relationship to existing dumps:** `docs/ideas/preservation-brainstorm.md` (171 items, preservation
only) and `docs/audit-100-reader-ideas-2026-10-04.md` (100 reader ideas, 81 already built) stay as they
are. This file is additive; where an idea *extends* a shipped thing it says so with `↳`.
**Not triaged.** No impact/effort scores, because none were submitted with a ranking — see the closing
section for how to read the sizes and what is deliberately left to the operator.
**Status in Concord:** these were added as `discussion` rows on project `darkhold` (id 9), which is the
ideas-board state, not the roadmap. Nothing here is a commitment.

Priority legend, from the estate north star + the operator's five stated aims:
`taste` = more quality content the operator and taste-similar curators would enjoy ·
`governance` = user-driven, minimal operator · `federation` · `preservation` = automatic ·
`power` = power-user surface.
Sizes: `S` ≤ a day, `M` a few days, `L` a week-plus, `XL` its own spec section.

---

## A. Taste, quality and discovery (operator + curators with similar taste)

- **D-A-01 · Curator cohorts from bookmark overlap** — cluster readers by bookmark/kudos co-occurrence and score a work *per cohort* rather than globally, so "quality" is measured against taste instead of averaged toward the middle. `L · taste`
- **D-A-02 · Taste neighbours on the profile** — show the 20 readers closest to you with a one-way, opt-in visibility so a curator can see whose judgement they should trust. `M · taste`
- **D-A-03 · Cohort-relative novelty** — flag a work as new *to your cohort* even when it is years old globally, so late arrivals still get discovered. `M · taste`
- **D-A-04 · Anti-hype weighting** — down-weight works whose cohort variance is near zero and whose global score is high; a work everyone likes is a weaker signal than one your cohort loves. `M · taste`
- **D-A-05 · Negative taste vectors** — "not for me, ever" on a tag subtracts from scores instead of hiding the work, so the subtraction generalises to works you have not seen. `M · taste`
- **D-A-06 · Silent-majority signal** — weight kudos-only readers separately from commenters and publish both weights, because the loud minority is currently the only voice. `S · taste`
- **D-A-07 · Re-read detection as the top signal** — treat a second read as the strongest available quality evidence and give the reader a "you keep coming back" shelf built from it. `M · taste`
- **D-A-08 · DNF as a first-class private signal** — record abandonments (private by default) and use them to learn the shape of what a reader drops, not what they like. `M · taste`
- **D-A-09 · Drop-off heatmap for authors (aggregate only)** — chapter-boundary abandonment rolled up per work, cohort-relative, with a k-anonymity floor so no individual is exposed. `L · taste`
- **D-A-10 · Chapter-level taste** — learn preference across a work's chapters, not just the work, so "great first 10 chapters, saggy middle" becomes a signal rather than a single score. `L · taste`
- **D-A-11 · Stylometry-only browse** — expose the deterministic work-coordinate vector as its own discovery axis for readers who care about prose feel. `M · taste` `↳ work coordinates`
- **D-A-12 · Pairwise preference inbox** — five A/B pairs a week from the reader's own shortlist, feeding a Glicko-style rating instead of star scores. `M · taste`
- **D-A-13 · Curator picks as an object** — a named list with a one-line rationale per work, ranked by the curator, forkable by others. `M · taste`
- **D-A-14 · Curator track record** — measure whether a curator's picks earn completion and re-reads, and show that hit-rate on the list page. `M · taste`
- **D-A-15 · First-ten-readers attribution** — credit the readers who found a work before it had any traction, which is the scout credit's missing half. `S · taste` `↳ scout value`
- **D-A-16 · Deep-cut slot per category** — one slot that may only be filled by a work under the impression floor, enforced in the query rather than promised in policy. `S · taste` `↳ exposure floor`
- **D-A-17 · Serendipity dial** — a reader-set exploration/exploitation slider with the effect stated in plain words on the dial. `S · taste`
- **D-A-18 · Series gap detector** — "you read 2 of 5 here, and part 3 is 40k shorter than the others" as one card. `S · taste`
- **D-A-19 · Abandonment-aware shelves** — exclude long-hiatus works unless the reader opts in, because a 3-year-old cliffhanger is a taste failure, not a recommendation. `S · taste`
- **D-A-20 · Length and pace as first-class dimensions** — word-count bands and derived pace (chapter length variance) as filterable, learnable dimensions. `M · taste`
- **D-A-21 · Mood taxonomy derived from existing tags** — map angst/fluff/hurt-comfort onto tags already present instead of asking authors to retag. `M · taste`
- **D-A-22 · Progressive tag reveal** — browse with fandom and warnings visible but relationship/trope tags hidden until requested, for readers who want to be surprised. `S · taste`
- **D-A-23 · Blind-date mode** — hide fandom and tags until N words are read, then show them; measure whether blind starts finish more often. `M · taste`
- **D-A-24 · "Under-read masterpiece" slot** — curator-nominated, permanent within a cycle, with the nomination rationale attached. `S · taste`
- **D-A-25 · Taste-state digest** — a monthly "what changed in your taste" note: fandoms entered, tags that stopped working, works you abandoned. `M · taste`
- **D-A-26 · Editable taste profile** — render the vector as words a person can read and edit ("less soulmate-AU, more epistolary"), then honour the edits as priors. `L · taste`
- **D-A-27 · Multi-vector identity** — separate vectors for work-level, author-level and trope-level taste, weighted differently in each surface. `L · taste`
- **D-A-28 · Fandom-hopping via tag overlap** — recommend adjacent fandoms from tag co-occurrence rather than from "users also read". `M · taste`
- **D-A-29 · Canon-recency awareness** — do not recommend pre-canon-divergence works to a reader who consistently prefers post-canon. `M · taste`
- **D-A-30 · Per-author feed quota** — cap how many works by one author can appear in a feed window, enforced in ranking. `S · taste`
- **D-A-31 · Cohort-aware exposure floor** — a floor per cohort, not per instance, so a niche cohort's new works still surface. `M · taste` `↳ exposure floor`
- **D-A-32 · Per-cohort MMR lambda** — the variety re-ranking parameter configurable per cohort, since a broad cohort needs more diversity than a narrow one. `M · taste` `↳ MMR`
- **D-A-33 · Quorum-gated front page** — promotion to the front page requires a curator quorum, not a vote count. `M · governance`
- **D-A-34 · Curation salons** — time-boxed group curation of a theme, with the resulting list as the artefact. `M · taste`
- **D-A-35 · One-tap kudos taxonomy** — "prose / plot / characters / world" as optional kudos facets, feeding finer signals without a review box. `S · taste`
- **D-A-36 · Kudos budget** — a daily kudos cap to stop reflexive clicking from flattening the signal. `S · taste`
- **D-A-37 · Retract a kudos with a reason taxonomy** — makes "changed my mind" a signal instead of a silent removal. `S · taste`
- **D-A-38 · Author audience statement** — "this is for readers who liked X" as a structured field, filterable, rather than prose in the summary. `S · taste`
- **D-A-39 · Freshness for old works** — a completed-in-2020 work re-tagged "now complete" is new to readers; treat completion as a freshness event. `S · taste`
- **D-A-40 · Mirror collapsing in feeds** — one work mirrored on three instances appears once, with mirrors listed under it. `M · federation`
- **D-A-41 · Rec explanation on the card** — the "why this" three-signal line belongs on the work card, not behind a transparency page. `S · taste` `↳ recommendation transparency`
- **D-A-42 · Mute a signal source permanently** — per-source mute ("stop using my imported history"), with the effect visible in the next feed. `S · taste`
- **D-A-43 · Trusted pre-readers** — an author can release a chapter to a cohort before public, collecting typed reactions. `L · taste`
- **D-A-44 · Author cohort view** — "who actually reads me" as aggregate cohort labels, never identities. `M · taste`
- **D-A-45 · Fest and event shelves** — seasonal fic fests with quorum-judged shortlists. `L · governance`
- **D-A-46 · Anonymous kudos** — kudos without a public trace, for readers who avoid social pressure. `S · taste`
- **D-A-47 · Content-warning caps** — reader-set maximums ("no more than one major warning"), applied before ranking. `S · taste`
- **D-A-48 · Per-language taste vectors** — separate vectors per language, so a strong English profile does not mis-rank a translated work. `M · taste` `↳ cross-language supply`
- **D-A-49 · Translation-aware discovery** — recommend the translated edition when the reader's language vector is stronger there than the original's. `M · taste`
- **D-A-50 · Import-biased cold start** — the history import seeds a *fandom-biased* prior, so a new reader's first feed is not a global average. `S · taste` `↳ history import`
- **D-A-51 · Published cohort rules** — a page stating exactly how cohorts are formed, what is shared, and what is never inferred. `S · taste`
- **D-A-52 · Visible signal half-lives** — every signal's decay stated in the UI, so "your 2019 taste still counts 3 %" is a fact the reader can act on. `S · taste`
- **D-A-53 · Public discovery overrides** — every operator/curator ranking override is listed publicly with the work it affected. `S · governance` `↳ taste leakage view`
- **D-A-54 · Per-cohort A/B assignment** — experiments assigned by cohort, never globally, so one cohort's improvement cannot degrade another's feed. `M · taste`
- **D-A-55 · Feed budget** — a hard cap on recommendations per day, with the surplus stored rather than shown. `S · taste`
- **D-A-56 · Inferred-tag display** — tags inferred from cohort behaviour shown as inferred and one-click correctable by the author. `L · taste`
- **D-A-57 · First-work curator assignment** — a new author's first work gets a named curator from the welcome rota, not just a comment. `S · governance` `↳ M45-26`
- **D-A-58 · Series-order safety** — never recommend part 4 without part 1 unless the reader opts into out-of-order. `S · taste`
- **D-A-59 · Optional completion badge** — cohort-relative finish rate shown on the card, author-toggleable. `M · taste`
- **D-A-60 · Reading-time from your own pace** — estimate from the reader's measured pace, not a global 250 wpm. `S · taste`
- **D-A-61 · Year diversity in feeds** — no feed window may be more than 60 % one publication year. `S · taste`
- **D-A-62 · Crossover competence shelf** — works requiring two canons, offered only when both are in the reader's profile. `M · taste`
- **D-A-63 · Curator disagreement surfacing** — where two trusted curators disagree on a work, show both takes rather than an average. `M · taste`
- **D-A-64 · Tag-drift alert for curators** — when a work's applied tags diverge from cohort consensus, queue it for review. `M · governance`
- **D-A-65 · Quorum-verified canon assertions** — character/relationship assertions confirmed by a quorum of readers who finished the work. `L · governance`
- **D-A-66 · "Surprise me" with a stated distribution** — a random pick whose sampling weights are published, so it cannot silently favour the promoted. `S · taste` `↳ uniform-random exploration`
- **D-A-67 · Taste-portability file** — export/import your taste vector as a signed blob, so a new instance starts warm. `M · federation`
- **D-A-68 · Cross-instance taste merge** — blend a local vector with remote follows' signals, with the blend ratio shown. `L · federation`
- **D-A-69 · Sparse-work rescue** — works with too few tags to rank get a curator pass rather than being buried. `S · taste`
- **D-A-70 · Quality signal from critique participation** — works that attract substantive critique rank higher for readers who value craft feedback. `M · taste` `↳ critique queue`
- **D-A-71 · Reader-set "effort" bands** — some readers want 300k+ epics only; make length a sticky preference, not a per-search filter. `S · taste`
- **D-A-72 · Explicit "why not" for hidden works** — when a work is filtered, offer a one-line reason rather than silence, which also debugs the filter. `S · taste`
- **D-A-73 · Cohort-level taste drift alarms** — if a cohort's centroid moves sharply in a week, surface it to curators before it reshapes the feed. `M · taste`
- **D-A-74 · Author-side cohort targeting** — an author may target a cohort for a release; the cohort is a size band, never a named list. `L · taste`
- **D-A-75 · Non-fiction and meta shelf** — essays, meta and fan-analysis get their own discovery track with their own signals. `M · taste`
- **D-A-76 · Multi-work reading sessions** — detect parallel reads (three works in one evening) and treat that as breadth appetite. `M · taste`
- **D-A-77 · Recency-weighted kudos for revived works** — a work kudosed 40 times this week outranks one kudosed 400 times in 2019, for the current feed only. `S · taste`
- **D-A-78 · Curator rotation with handover notes** — outgoing curator writes what they learned into the salons archive. `S · governance`
- **D-A-79 · Discovery regression suite** — golden feeds for fixed synthetic readers, run on every ranking change, so a tuning change cannot silently degrade a cohort. `L · taste`
- **D-A-80 · Operator-only taste lens, one click** — "show me the feed as this cohort sees it", read-only and logged. `S · taste`

---

## B. User-driven governance with minimal operator oversight

The test every idea here is held to: **could this run for a year with the operator absent?** Anything that
needs the operator to keep it honest is either redesigned or explicitly listed as a reserved power.

- **D-B-01 · Publish the trust-level thresholds** — every promotion criterion in one page, computed from live queries rather than written by hand. `S · governance` `↳ trust levels`
- **D-B-02 · Per-category quorum thresholds** — a critique-heavy category can require a larger quorum than a fan-art one, configured per category. `M · governance`
- **D-B-03 · Jury duty from the eligible pool** — disputes draw jurors at random from users above a trust level, with accept/decline and a decline budget. `L · governance`
- **D-B-04 · Cohort-proportional juries** — sample jurors by cohort so one clique cannot pack a verdict. `M · governance`
- **D-B-05 · One appeal, differently sampled** — every quorum verdict is appealable once, to a fresh jury with the first one's reasoning withheld. `M · governance`
- **D-B-06 · Decision receipts** — every moderation action emits a public, pseudonymised receipt citing a rule version. `M · governance`
- **D-B-07 · Rules as versioned objects** — each rule is a row with a version; actions cite the version in force at the time. `L · governance`
- **D-B-08 · Meta-moderation** — a sample of verdicts is re-judged blind; overturned verdicts cost the original jurors standing. `L · governance`
- **D-B-09 · Standing decay for inactive moderators** — trust decays with inactivity so authority follows participation rather than history. `S · governance`
- **D-B-10 · Community-proposed categories** — new categories require a proposal plus quorum, not an operator decision. `M · governance`
- **D-B-11 · Tag synonyms by quorum** — synonym merges are voted, applied with a reversible mapping, and logged. `M · governance`
- **D-B-12 · Public sanctions ladder** — warn → throttle → topic ban → category ban → instance ban, each with a stated duration and auto-expiry. `S · governance` `↳ forum_sanctions`
- **D-B-13 · No permanent bans except two enumerated cases** — spam and illegal content; every other sanction expires without intervention. `S · governance`
- **D-B-14 · Moderator recall petition** — a threshold of trust-level users forces a re-vote on a moderator's mandate. `M · governance`
- **D-B-15 · Term limits for category curators** — rotations are automatic, with handover notes as the artefact. `S · governance`
- **D-B-16 · Automatic recusal** — a juror who bookmarked, kudosed or flags-conflicts the work recuses by query, not by self-report. `S · governance`
- **D-B-17 · Blind exhibits** — disputes strip pseuds and author names from the evidence bundle before jurors see it. `M · governance`
- **D-B-18 · Evidence packs** — a dispute bundles the contested passage, revision history and prior verdicts as one immutable object. `M · governance`
- **D-B-19 · Public case log with statistics** — cases, outcomes and overturn rates per rule, queryable, DSA-shaped. `L · governance` `↳ GDPR/DSA tooling`
- **D-B-20 · Ombuds with no powers** — an elected role whose only output is a periodic report on moderation behaviour. `S · governance`
- **D-B-21 · Enumerated reserved powers** — the operator's powers listed explicitly (funding, legal, secrets, instance survival) and nothing else; the list is public and a diff is dated. `S · governance`
- **D-B-22 · Escalation only after quorum failure** — a case reaches the operator only after N failed juries, and the failure count is public. `M · governance`
- **D-B-23 · Juror vote budgets** — a cap per juror per week, to prevent enthusiast capture of the queue. `S · governance`
- **D-B-24 · Sybil resistance from reading depth** — weight votes by demonstrated reading history, not account age alone. `M · governance`
- **D-B-25 · Federated disputes judged at origin** — for mirrored content, the origin instance judges and peers observe; the verdict carries provenance. `L · federation`
- **D-B-26 · Signed instance policy documents** — an instance's rules are a signed doc; users see a dated diff when it changes. `M · federation`
- **D-B-27 · Machine-readable constitution** — the rules the platform actually enforces exist as data with tests that assert each one is enforced. `L · governance`
- **D-B-28 · Consent-scoped proposal volume** — per-user caps on open proposals, so governance does not become a queue of spam. `S · governance`
- **D-B-29 · Proposal pipeline as objects** — draft → discussion → quorum → implementation task, each state visible and timed. `L · governance`
- **D-B-30 · Claimable implementation tasks** — approved proposals become tasks any qualified user can claim, with credit on merge. `M · governance` `↳ kudos`
- **D-B-31 · Duplicate-work merge by quorum** — the same work posted twice is merged by jury, with both URLs preserved as aliases. `M · governance`
- **D-B-32 · Series split/merge by quorum** — same mechanism, different object, with an undo window. `M · governance`
- **D-B-33 · Tag quarantine instead of deletion** — a disputed tag is hidden pending verdict, never removed from the record. `S · governance`
- **D-B-34 · Jury rubrics authored by users** — the critique queue's rubric becomes a template any cohort can author and reuse. `M · governance` `↳ critique queue`
- **D-B-35 · Ranking changelog** — every ranking/tuning change published as a dated entry with the cohort effect measured. `M · governance`
- **D-B-36 · Rule-health review from overturn rate** — rules above a threshold overturn rate are automatically queued for revision. `M · governance`
- **D-B-37 · Moderator action budgets** — an unusual burst of actions triggers a peer review rather than taking effect silently. `M · governance`
- **D-B-38 · Reporter anonymity to moderators** — juries see the report, never the reporter. `S · governance`
- **D-B-39 · Report-deletion requires appeal** — a moderator cannot erase a report; closing it is logged with a reason code. `S · governance`
- **D-B-40 · Public moderation-load metric** — how much volunteer time the community is spending, so the cost is visible before it burns out. `S · governance`
- **D-B-41 · Category curators elected by cohort** — the cohort that reads the category elects its curator. `M · governance`
- **D-B-42 · Published curator criteria** — what makes a good curator is written down and used in elections, not implied. `S · governance`
- **D-B-43 · Standing as collateral** — high-trust users can stake standing on a contested verdict; wrong stakes cost, right stakes earn. `L · governance`
- **D-B-44 · Status-quo default for unresolved disputes** — time-boxed decisions that expire to "no change", which removes the incentive to stall. `S · governance`
- **D-B-45 · Votes carry rationale** — quorum votes require a one-line reason, published pseudonymised; empty votes are rejected. `M · governance`
- **D-B-46 · Generated jury instructions** — instructions assembled deterministically from the rule library, so no one hand-writes the framing. `M · governance`
- **D-B-47 · Dispute-volume throttle per work** — a work cannot attract an unlimited number of simultaneous cases. `S · governance`
- **D-B-48 · Audit of every automated action** — no bot moderation without a public row naming the rule and the input. `M · governance`
- **D-B-49 · Everything reversible by quorum** — any action a moderator can take, a fresh quorum can undo. `L · governance`
- **D-B-50 · Continuity document** — what breaks if the operator disappears, which quorum powers take over, and the interval before nominal authority returns. `M · governance`
- **D-B-51 · Rules that travel with a mirror** — a mirrored work carries its origin instance's policy and the local instance's additions, both visible. `M · federation`
- **D-B-52 · Instance ban transparency** — a ban lists its rule and is appealable to a cross-instance panel. `M · federation`
- **D-B-53 · Spam handling without the operator** — rate limits plus jury removal, with a documented worst case. `M · governance`
- **D-B-54 · Quorum-verified bounty completion** — the fic-finder bounty pays when a jury confirms the found work is the requested one. `L · governance` `↳ fic-finder bounties`
- **D-B-55 · Reading-group self-moderation with appeal** — group founders moderate their group; quorum hears appeals, not first-instance disputes. `M · governance`
- **D-B-56 · Immediate consent withdrawal** — withdrawal propagates platform-wide in one transaction, logged, no queue. `M · preservation` `↳ author consent`
- **D-B-57 · Charter amendment thresholds** — amendments need a supermajority and a cooling-off period, both published. `S · governance`
- **D-B-58 · Visible default thresholds** — a config page showing every threshold in force, so an instance's differences are legible. `S · governance`
- **D-B-59 · Two-vouch promotion** — a trust-level promotion requires two existing high-trust vouches, recorded. `S · governance`
- **D-B-60 · Auto-generated governance report** — an annual "state of governance" document assembled from the case log, no author required. `M · governance`
- **D-B-61 · Juror fatigue detection** — a juror whose verdicts correlate with speed is quietly rested, not accused. `M · governance`
- **D-B-62 · Conflict-of-interest registry** — curators declare interests once, and the registry is queryable at decision time. `S · governance`
- **D-B-63 · Cross-instance standing import with a cap** — standing imported from a peer is bounded and decays faster than local standing. `L · federation`
- **D-B-64 · Public queue depth per category** — readers see which moderation queues are starved, which is the hiring signal. `S · governance`
- **D-B-65 · Deliberation windows** — a verdict cannot be reached faster than N hours, which suppresses pile-ons. `S · governance`
- **D-B-66 · Outcome sampling for quality** — random closed cases are re-opened for review, like a tax audit but public. `M · governance`
- **D-B-67 · Withdrawal audit trail** — when an author withdraws, the record of what was mirrored where survives as a tombstone. `M · preservation`
- **D-B-68 · Standing for accurate flagging** — flag precision feeds standing, so flagging is a skill with a score. `M · governance`
- **D-B-69 · No operator-only tools on the hot path** — every review action the operator performs must have a quorum equivalent, or the gap is listed as reserved. `M · governance`
- **D-B-70 · Governance dry-run simulator** — feed the engine a synthetic dispute and see what the current rules would do, before changing them. `L · governance`

---

## C. Federation

Framing measured today: `crates/db/src/federation.rs` signs with `format!("sig_{}", instance_host)` — a
placeholder, not an HTTP Signature — and `federation_queue` has no delivery worker. So "federated" is
currently peer *discovery* by theme similarity, not interop. These ideas are ordered so the cheap,
honest subset works before any protocol work: **C-01..C-14 are the minimum real federation**, the rest
assume it exists.

- **D-C-01 · Real HTTP Signatures** — replace the `sig_{host}` placeholder with RFC 9421 signing and verification on both inbox and outbox, with a test that rejects a forged signature. `M · federation`
- **D-C-02 · WebFinger and actor discovery** — `/.well-known/webfinger` plus actor documents so a peer can find a pseud without prior arrangement. `M · federation`
- **D-C-03 · Outbox-only announce first** — ship one direction (announce a new chapter) before attempting inbox, so interop is proven with a real peer at the smallest possible surface. `M · federation`
- **D-C-04 · Inbox idempotency and replay defence** — activity ids are dedupe keys; a replayed activity must be a no-op with a log line. `M · federation`
- **D-C-05 · Delivery worker with backoff and dead-letter** — retries with jitter, a dead-letter queue, and a per-peer failure counter visible to admins. `L · federation` `↳ federation_queue`
- **D-C-06 · Capability advertisement** — each instance declares which features it speaks, so a peer never sends an activity the other cannot parse. `M · federation`
- **D-C-07 · Graceful local-only degradation** — every federated surface has a local-only path, so a dead peer cannot break a page. `M · federation`
- **D-C-08 · Cross-instance work identity** — canonical origin URL plus content hash, so the same work on three instances is one object with three locations. `L · federation`
- **D-C-09 · Mirror list on the work page** — where else this work lives, with freshness stamps and a "verified today" marker. `M · preservation`
- **D-C-10 · Signed "work moved" notices** — origin redirects are signed, so a mirror can follow without guessing. `M · federation`
- **D-C-11 · Tombstones for deletions** — a deleted work leaves a signed tombstone so peers delete rather than re-mirror forever. `M · preservation`
- **D-C-12 · Peer health beacons** — uptime, queue depth and last successful delivery per peer, published per instance. `S · federation`
- **D-C-13 · Federation diagnostics page** — "why did my chapter not federate" answered from the queue and the peer's capability set. `S · federation`
- **D-C-14 · Content-addressed bodies** — store bodies by hash so a mirror of a popular work costs one copy, not N. `L · preservation`
- **D-C-15 · Federated comment threads** — a topic keeps its origin instance and every reply carries its own; the thread renders as one tree with provenance. `L · federation`
- **D-C-16 · Per-reply federation scope** — a reply can be local-only while its topic is federated, which is what a shy user actually wants. `M · federation` `↳ federation-scope`
- **D-C-17 · Cross-instance kudos aggregation** — show federated kudos separately from local ones instead of summing them into one inflated number. `M · taste`
- **D-C-18 · Signed theme fingerprints** — the existing instance-theme vector, actually signed and exchanged, so peer discovery is not self-reported. `M · federation` `↳ instance_fingerprints`
- **D-C-19 · Reciprocal peer handshake** — both sides record the other's key before either delivers anything. `M · federation`
- **D-C-20 · Peer gossip of link-dead events** — one instance's 404 becomes every instance's probe result. `M · preservation`
- **D-C-21 · Federated search across opted-in peers** — per-instance indexes, merged with provenance, respecting each peer's robots and rate limits. `XL · federation`
- **D-C-22 · Cross-instance reading progress (opt-in)** — continue a work on another instance, end-to-end encrypted, local key. `XL · federation`
- **D-C-23 · Local-only bookmarks** — federated identity must not make bookmarks a shared signal; default is local. `S · federation`
- **D-C-24 · Series stitching across instances** — part 2 on another instance is still part 2. `M · federation`
- **D-C-25 · Preservation pacts between instances** — a signed mutual-mirroring agreement with coverage guarantees and a cost cap. `L · preservation`
- **D-C-26 · Federation cost accounting** — bandwidth and storage per peer, shown to admins, so peering is a decision with a number. `M · federation`
- **D-C-27 · Quota negotiation** — peers declare how much they will mirror; the sender respects it instead of discovering it by failure. `M · federation`
- **D-C-28 · Author claim via rel=me** — an author proves ownership on their own domain and the instance trusts that, not their word. `M · federation`
- **D-C-29 · ActivityPub bridge to PieFed, announce-only** — publish chapter announcements to the existing PieFed instance for reach, never reading back; the cheapest real fediverse presence. `M · federation`
- **D-C-30 · RSS and JSON feeds per category, author and cohort** — the reach that costs nothing and needs no protocol. `S · power`
- **D-C-31 · Webmention support** — blogs and other instances can point at a work and the link appears. `M · federation`
- **D-C-32 · IndieAuth login for authors** — own your identity outside the instance. `M · power`
- **D-C-33 · Federated tag vocabulary alignment** — a mapping table between instances' tag vocabularies, applied at ingest with the mapping visible. `L · federation`
- **D-C-34 · Canon assertions merged by hash** — two instances asserting the same character relationship agree without coordination. `L · federation`
- **D-C-35 · Remote moderation with provenance** — a peer's mod action is an object with its origin rule, never silently applied as local policy. `M · federation`
- **D-C-36 · Cross-instance jury for federated content** — jurors drawn from both origins for mirrored disputes. `L · governance`
- **D-C-37 · Bounded credit reconciliation** — cross-instance credits settle in bands, opt-in, never cashable. `XL · federation`
- **D-C-38 · Federated bounty board** — fic-finder requests visible across peers with quorum-verified payouts. `L · governance` `↳ fic-finder bounties`
- **D-C-39 · Cross-instance critique exchange** — a work can request critique from a peer community with a named rubric. `L · taste`
- **D-C-40 · Federated reading groups** — a group spans instances; its moderation stays local to each side. `L · federation`
- **D-C-41 · Language-scoped federation** — an instance accepts only content in its languages unless it opts in, which keeps discovery useful. `M · federation`
- **D-C-42 · Category-scoped federation** — adult content federates only to peers that have opted in explicitly, at category granularity. `M · federation`
- **D-C-43 · Peer key rotation and revocation** — a rotated key is announced, an old one refused, both logged. `M · federation`
- **D-C-44 · Signed instance manifests for clients** — apps read one document per instance instead of scraping. `S · power`
- **D-C-45 · One-way Lemmy community mirror** — darkhold posts appear in a Lemmy community for reach, clearly labelled as mirrored. `M · federation`
- **D-C-46 · AP actor per pseud with field-level privacy** — public identity optional per pseud, never inferred from activity. `L · federation`
- **D-C-47 · Federated reply notifications** — a remote reply notifies the local author, deduplicated by activity id. `M · federation`
- **D-C-48 · Cross-instance spam hashes** — known spam bodies shared as hashes, not content. `S · federation`
- **D-C-49 · Mirror-first publish** — publishing seeds mirrors automatically rather than waiting for a curator to request them. `L · preservation`
- **D-C-50 · Air-gapped instance sync via signed bundles** — sneakernet federation for an instance with no network, applied idempotently. `L · preservation`
- **D-C-51 · Read fallback when origin is down** — serve the mirror with a banner naming the origin and the fetch time. `M · federation`
- **D-C-52 · Uptime-based read routing** — the client prefers a live peer when the origin times out. `M · federation`
- **D-C-53 · Peer delivery honesty score** — peers that ack and never apply are visible, which is the fediverse's real failure mode. `M · federation`
- **D-C-54 · Deterministic merged-edit conflict resolution** — last-writer-wins by revision timestamp plus a visible conflict list, never a silent overwrite. `L · federation`
- **D-C-55 · Reader-identity-safe analytics** — cross-instance aggregates with a k-anonymity floor and no per-reader identifiers. `M · federation`
- **D-C-56 · Federated instance directory with themes** — opt-in discovery of instances by taste theme, replacing self-reported lists. `M · federation` `↳ list_public_themes`
- **D-C-57 · Machine-readable export of an instance's catalogue** — so a dying instance's catalogue can be adopted by a peer in one action. `L · preservation`
- **D-C-58 · Cross-instance canonical-comment dedupe** — the same comment mirrored twice appears once. `S · federation`
- **D-C-59 · Peer-scoped rate limits** — per-peer fetch and delivery budgets, so one bad peer cannot consume the instance. `S · federation`
- **D-C-60 · Mastodon/Nostr relay for new chapters** — announce to the wider fediverse without hosting an AP server. `M · federation`

---

## D. Automatic preservation

`docs/ideas/preservation-brainstorm.md` holds the strategy layer (171 items, many already ticked as
built). These are the *mechanisms* that make it automatic: what probes, what verifies, what repairs,
and what happens when nobody is watching.

- **D-D-01 · Scheduled link-dead probes with backoff** — every mirror is probed on a schedule that backs off for stable targets and shortens on recent failures; a dead mirror is an event, not a footnote. `M · preservation`
- **D-D-02 · Per-chapter content hashing** — sha256 per chapter stored at fetch, verified on read, so silent edits and truncation are detectable. `M · preservation`
- **D-D-03 · Mirror health score on the work page** — a computed score from probe recency, hash agreement and independent hosts, not a static badge. `S · preservation`
- **D-D-04 · Automatic re-mirror request on degradation** — when coverage drops below the pact threshold, a mirror request is queued without a human noticing. `M · preservation`
- **D-D-05 · Integrity audit sweep** — a rotating full-content verification pass with a bounded daily budget, so bit-rot is found by the system rather than by a reader. `L · preservation`
- **D-D-06 · Cold-storage tiering** — works untouched for N days move to a cheaper tier as a RO-Crate or BagIt bundle, still restorable in one command. `L · preservation`
- **D-D-07 · Restore drill that actually runs** — a scheduled restore of a random cold bundle into a scratch database, asserting the body hash, so "backed up" means "restores". `M · preservation`
- **D-D-08 · Deletion grace archive** — a deleted work keeps a consented shadow copy for a stated window, with the author's consent recorded per work. `L · preservation` `↳ author consent`
- **D-D-09 · Metadata-only shadow when the body is refused** — if an author refuses mirroring, the instance still keeps provenance (where it was, when, what it was) for citation. `M · preservation`
- **D-D-10 · Signed provenance chain** — every mirror action is an append-only row: who mirrored what, from where, when, under which consent record. `L · preservation`
- **D-D-11 · RFC 3161 timestamping on snapshots** — an external, verifiable time for each preserved snapshot. `M · preservation`
- **D-D-12 · Monthly Merkle root of the preservation ledger** — one number per month that proves nothing was quietly rewritten. `M · preservation`
- **D-D-13 · Reader-verifiable downloads** — an EPUB export carries its hash in the filename and a manifest inside. `S · preservation`
- **D-D-14 · Format normalisation on ingest** — parse to a canonical internal form and keep the original bytes, so a reader in 2040 is not dependent on one renderer. `L · preservation`
- **D-D-15 · MIME sniffing instead of trusting extensions** — a `.html` that is actually a PDF is stored correctly and rendered as what it is. `S · preservation`
- **D-D-16 · Duplicate detection by hash across archives** — the same work at three URLs is one object with three sources. `M · preservation`
- **D-D-17 · Earliest-fetch preference** — when duplicates differ, keep the earliest capture and record the later ones as revisions. `M · preservation`
- **D-D-18 · Rate-limited polite fetcher with per-domain budgets** — crawling is quota'd per domain, published in the diagnostics page. `M · preservation` `↳ shared safe fetcher`
- **D-D-19 · Loop-shaped pagination detection** — a "next chapter" that points backwards is caught rather than crawled forever. `S · preservation`
- **D-D-20 · Adapter failure dashboard** — per-archive error rates, so a broken adapter is visible within a day. `S · preservation` `↳ source adapters`
- **D-D-21 · Deletion-watch on the source archive** — a scheduled re-check of works whose origin is not this instance, with a "gone" state distinct from "unreachable". `M · preservation`
- **D-D-22 · WARC capture for evidence** — the source page as served, for disputes and takedown receipts. `L · preservation`
- **D-D-23 · Legal-hold mode** — a disputed work is frozen (no edits, no deletion) rather than deleted, with the hold reason and expiry. `M · preservation`
- **D-D-24 · Takedown receipts that outlive the work** — what was removed, under which rule, when, and who can appeal. `M · governance`
- **D-D-25 · Preservation debt metric** — "works with a single copy" as a first-class number on the admin page and in the governance report. `S · preservation`
- **D-D-26 · Auto-escalation below two copies** — coverage below two triggers the mirror queue at higher priority, not a notification nobody reads. `M · preservation`
- **D-D-27 · Series-complete preservation** — mirroring a series means all of it, or the gap is explicitly listed. `M · preservation`
- **D-D-28 · Fandom snapshot** — a quorum-approved "save this fandom" job with a stated scope and cost, run as one bounded task. `L · preservation`
- **D-D-29 · Preservation cost per work, shown** — bytes, copies, egress; transparency is what keeps the pacts honest. `S · preservation`
- **D-D-30 · Published preservation SLOs** — e.g. "three independent copies within 30 days of publication", measured and reported monthly. `M · preservation`
- **D-D-31 · Tombstone ledger** — what was lost, when and why, so loss is counted rather than forgotten. `M · preservation`
- **D-D-32 · Zombie detection** — a work that 404s at origin but exists on a mirror is marked "preserved, origin dead" instead of vanishing. `M · preservation`
- **D-D-33 · Orphan adoption with consent** — an abandoned work can be adopted by a curator for maintenance only when the author's consent record allows it. `L · preservation`
- **D-D-34 · Media dedupe** — images and audio stored once by hash and referenced, so preserving a fandom does not multiply by embedding. `M · preservation`
- **D-D-35 · Derivative registry** — narration, translations and EPUBs point at the work and the source hash they were made from. `M · preservation` `↳ narration`
- **D-D-36 · Hash mismatch quarantine** — a body that fails verification is quarantined and re-fetched, never served silently. `S · preservation`
- **D-D-37 · Distribution of copies by host** — the coverage score must count independent hosts, so three copies on one box count as one. `S · preservation`
- **D-D-38 · Mirror reciprocity score** — instances that mirror others are prioritised for mirroring, which keeps pacts from being one-way. `M · governance`
- **D-D-39 · Preservation pact templates** — a small instance can adopt a standard pact (coverage floor, cost cap, exit terms) in one action. `M · preservation`
- **D-D-40 · Instance shutdown bundle** — one command produces a signed, restorable bundle of everything, with a manifest a peer can verify. `L · preservation`
- **D-D-41 · Adoption path for a dead instance's bundle** — a peer can ingest another instance's shutdown bundle and serve the catalogue with provenance intact. `XL · preservation`
- **D-D-42 · Institutional handoff format** — the same bundle shaped for a library or archive that is not an instance. `L · preservation`
- **D-D-43 · Readable-plaintext fallback** — every work stored with a plaintext rendering so no future dependency on the current renderer. `M · preservation`
- **D-D-44 · Robots and consent recorded per fetch** — the robots.txt (or its absence) captured with the fetch, so the ethics are auditable years later. `S · preservation`
- **D-D-45 · Consent-withdrawal propagation** — withdrawal deletes mirrors at peers within one delivery cycle, with confirmations tracked. `L · preservation`
- **D-D-46 · Source-archive newsletter/watch feed** — detect new works at a source without crawling the whole archive. `M · preservation`
- **D-D-47 · Store-and-forward fetch** — a fetch that must be retried hours later survives a restart, because it is a queue row and not a stack frame. `M · preservation`
- **D-D-48 · Content-addressed garbage collection with retention** — GC respects holds, pacts and consent windows, and refuses to collect a last copy. `L · preservation`
- **D-D-49 · Preservation metrics on the public dashboard** — copies, coverage, losses this month; a preservation programme nobody can see does not get funded. `S · preservation`
- **D-D-50 · Backup target health for the pool** — the NFS/disk targets used for storage are themselves checked, since the last disaster story is always "the backup disk". `M · preservation`
- **D-D-51 · Reader-side save-to-device with hash** — an offline copy a reader can verify later. `S · power`
- **D-D-52 · Archive.org handoff job** — a one-click submission job for works with a permissive licence. `M · preservation`
- **D-D-53 · Licence metadata parse and display** — CC terms parsed from the work, displayed, and used to gate mirroring. `M · preservation`
- **D-D-54 · Self-hosted mirror in a box** — a compose file a reader can run to hold their own copies of what they love. `L · power`
- **D-D-55 · Preservation-first ingest for fragile sources** — sources known to delete aggressively get priority in the fetch queue. `M · preservation`
- **D-D-56 · Coverage ratio per fandom** — which fandoms are least preserved, so curation effort goes where loss is likeliest. `M · preservation`
- **D-D-57 · Silenced-mirror detection** — a mirror that exists but stopped updating is a distinct state from dead. `S · preservation`
- **D-D-58 · Integrity alerts to authors** — an author is told their work's mirror degraded, with the option to re-seed. `S · preservation`
- **D-D-59 · Cost-bounded preservation budget** — a monthly byte budget with a queue priority, so preservation cannot starve the instance. `M · preservation`
- **D-D-60 · Disaster rehearsal calendar** — an annual announced drill: restore from bundles only, publish the time to full recovery. `M · preservation`

---

## E. Power-user surface

- **D-E-01 · Command palette** — every action reachable by keystroke with fuzzy search over objects, not just menu names. `M · power`
- **D-E-02 · Keyboard-first reading** — j/k, space, chapter nav, and a discoverable help overlay. `S · power`
- **D-E-03 · Saved query shelves** — a shelf defined by a query string, shareable, re-evaluated on each visit. `M · power`
- **D-E-04 · Tag algebra** — AND/OR/NOT with per-term weights, stored as a query object. `M · power`
- **D-E-05 · Regex tag filters** — for the reader who thinks in patterns, with a timeout so a bad regex cannot hang a feed. `S · power`
- **D-E-06 · Bulk actions** — tag, bookmark, export and mirror-request across a selection, with a dry-run preview. `M · power`
- **D-E-07 · Scoped API tokens** — tokens with per-scope permissions and their own rate budgets. `M · power`
- **D-E-08 · Webhooks for author events** — new chapter, kudos threshold, mirror change, with a delivery log. `M · power`
- **D-E-09 · CLI client with offline cache** — read, search and export from a terminal, syncing when online. `L · power`
- **D-E-10 · Stable DOM contract for userscripts** — documented selectors and data attributes, versioned, so power users' scripts do not break monthly. `S · power`
- **D-E-11 · Guaranteed permalinks** — ids that never change, with redirects for renamed slugs. `S · power`
- **D-E-12 · Chapter and paragraph anchors** — deep links into a specific paragraph that survive minor edits or state why they cannot. `M · power`
- **D-E-13 · Revision diff view** — what changed between revisions, chapter by chapter. `M · power`
- **D-E-14 · Search operators and alerts** — saved searches with notification, operator syntax documented inline. `M · power`
- **D-E-15 · Nested collections** — collections containing collections, with explicit ordering. `M · power`
- **D-E-16 · Reading statistics export** — JSON/CSV of everything the reader did, in a documented schema. `S · power`
- **D-E-17 · Private-first annotations** — highlights with notes, exportable, shareable per annotation. `L · power`
- **D-E-18 · Footnote and glossary rendering** — author-defined terms rendered with hover definitions. `M · power`
- **D-E-19 · Print and PDF stylesheet** — a reading mode that prints without chrome or broken page breaks. `S · power`
- **D-E-20 · User CSS themes** — per-reader stylesheet, scoped and sanitised. `M · power`
- **D-E-21 · Multi-column reader mode** — for wide screens, with the column break respecting paragraphs. `S · power`
- **D-E-22 · TTS integration hooks** — the narration surface addressable per paragraph, so external TTS can drive it. `M · power` `↳ narration`
- **D-E-23 · Offline PWA with sync** — read on a train, reconcile progress on reconnect. `L · power`
- **D-E-24 · Calibre interop contract** — a documented metadata mapping so `lorebook` and Calibre both handle a work identically. `M · power` `↳ lorebook`
- **D-E-25 · Bulk import from a library** — CSV or Calibre import with a preview and per-row error reporting. `M · power`
- **D-E-26 · Rate-limit visibility** — the client can see its remaining budget before it hits the wall. `S · power`
- **D-E-27 · Custom feed builder** — compose a feed from signals with explicit weights, then publish it as a shareable feed. `L · power`
- **D-E-28 · Personal instance mode** — run your own instance seeded from a main one, with a documented sync story. `XL · power`
- **D-E-29 · Reading journal** — a private log of what you read and why, exportable and searchable. `M · power`
- **D-E-30 · Author funnel metrics** — per-chapter views, completion, and where readers stopped. `M · power`
- **D-E-31 · Cohort-scoped preview release** — publish to a cohort before public, with a toggle and a size band. `M · power`
- **D-E-32 · Snippet and quote cards** — generate a shareable quote image with attribution and a return link. `S · power`
- **D-E-33 · Fast jury triage UI** — keyboard-driven, one verdict per keypress, with an undo window. `M · power`
- **D-E-34 · Saved jury rubrics** — reusable decision templates per rule family. `S · power`
- **D-E-35 · Batch tag application from CSV** — with a diff preview and a rollback. `S · power`
- **D-E-36 · Instance config bundle export/import** — an admin can move their instance's configuration between hosts as one file. `M · power`
- **D-E-37 · Queryable audit log for one's own actions** — "what did I do here" answered from the log. `S · power`
- **D-E-38 · Data portability one-click** — everything a user authored, as a documented archive. `M · power`
- **D-E-39 · Public dataset export for researchers** — hashed, rate-limited, consent-respecting. `L · power` `↳ GDPR/DSA tooling`
- **D-E-40 · Scriptable moderation webhooks** — a trusted power user can automate their own queue with a token, logged and rate-limited. `M · power`

---

## How to read this sweep

**310 ideas, by section:** 80 taste (A), 70 governance (B), 60 federation (C), 60 preservation (D),
40 power-user (E). The per-idea tag differs from the section where an idea serves two priorities —
measured: 74 governance, 72 taste, 66 preservation, 53 federation, 45 power. 31 carry `↳` because they
extend something that already ships — those are the cheapest wins, since the object and the tests
already exist.

**What this list deliberately does not contain.** No mechanic that pays anyone for an outcome about a
person's body, no publishing of private reading activity without consent, no forced-deadline pressure,
and nothing that needs the operator awake to stay honest — the governance section is held to the
"sit absent for a year" test, and every entry that fails it is written as a *reserved power* to be
enumerated (D-B-21), not as a feature.

**Order I would actually take it, if asked.** The federation block C-01..C-14 is the only place where
current behaviour is dishonest (a fake signature implies interop that does not exist) — that is a
correctness fix, not a feature. After that, the cheap `↳` items, then the governance jury core
(D-B-03..D-B-08), because everything else in governance assumes a jury that can be sampled.

**Concord.** The 310 rows were added to project `darkhold` as `discussion` — the ideas-board state, not
the roadmap — with the same ids, sizes and rationale as this file. Nothing was marked `draft`: an
untriaged brainstorm is not a commitment, and the owner's triage is what moves a row.

