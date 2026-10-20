# 26. Open questions

**Status:** reserved
**Milestone:** —
**Source:** the corresponding section of the Darkhold specification

**Closed by the owner's rulings and by the 2026-10-19 measurements.** Recorded here so the
resolution is visible rather than silently dropped.

| # | question | resolution | date |
|---|---|---|---|
| 1 | Which six themes, and which tags in each? | **DONE.** The six of §2.2, with each theme's measured support. The **overlap between *Dominion / Ownership* and *Corruption / Mindbreak* is settled by the rule in §2.2**: a tag's primary theme is the theme whose seed claims it, and the secondary is derived. `master/slave`, `master/pet` and `sexual slavery` are primary in Dominion; `self-insert` and `multiverse` are primary in Isekai | 2026-10-19 |
| 2 | The Supporter price | **The mechanism is decided, the number is the operator's.** §18 decides the *entitlement* and its four grant sources; the price is a config value and the spec's recommended default is **5 EUR/month**, on the reasoning that the tier exists to recover compute on a Pi-scale deployment, not to run a business. Nothing in the design depends on the number | 2026-10-19 |
| 3 | `full_min_trust` and `full_min_resonance` | **DECIDED.** §12.6 rule 6: the three gates are evaluated in a fixed order — **supporter, then trust, then resonance** — and exactly one wins and is named. Defaults: `full_min_trust = 4`, `full_min_resonance = 0.55`, `full_supporter = true`. The supporter path is the primary one, because it is the only one that pays for the compute the tier costs | 2026-10-19 |
| 4 | The mood gate | **DECIDED.** `trust >= 3` **or** supporter, `supporter_only = false`, `max_moods_per_account = 12`. The union of the two conditions, because a reader's trust is earned and a supporter's payment is a cost recovery, and either alone would gate a feature on the wrong axis | 2026-10-19 |
| 5 | Is a channel's seed editable after publication? | **DECIDED: frozen at publication; editing publishes a new version.** §12.10. The same rule as the instance theme's, for the same reason | 2026-10-19 |
| 6 | Pi or workstation first? | **DECIDED: workstation first, and the Pi is the gate that proves the tier model.** §24.3 adds **M32 — the Pi deployment**, which runs the whole instance on a Pi 5B and measures the total footprint against §22.3's 2.5 GiB. The Pi is last because it *validates* the tiers, and you cannot validate a tier model on hardware you are not deploying to | 2026-10-19 |
| 7 | The 1,000-work selection's filters | **RESOLVED.** Replaced by importing **every work with metadata** (111,686 measured, §9.8.1), with the 41 bookmarked works as the held-out evaluation set (§9.8.2). D-14 | 2026-10-19 |
| 8 | The theme boundary between Dominion and Corruption | **RESOLVED** with question 1 | 2026-10-19 |
| 9 | The inferred-fandom threshold | **RESOLVED by measurement.** §11.8: the shape heuristic plus the author-qualified form, minus a closed list of AO3 relationship/category **bases** (not qualifiers). Measured: 3,258 → **18,654** candidate fandom tags, 3,044 → **16,597** canonical groups, 52,033 → **82,608** works covered, and **`harry potter` goes from 9 groups to 1 group of 19,880** | 2026-10-19 |
| 10 | What a reader sees on the 111,000 works with metadata but no body | **RESOLVED.** §9.8.3: a first-class **provenance badge** — `hosted` / `referenced` / `known` — visible in the list, not only on the work page, and named in the recommendation's reason string | 2026-10-19 |

| 15 | Should the same author across different sources — and across their different pseuds — be clustered, so one author profile finds all their works? | **YES, and the author may opt out.** D-17 and §6.3.1: an **author identity** groups author names across sources and pseuds; the **source's own** account→pseud link is trusted, an inferred name match is only a **proposal**; a pending opt-out claim **blocks** the merge until it is decided; the raw names are never overwritten; the two largest "authors" in the mirror — `orphan_account` (1,889) and `Anonymous` (1,650) — are **reserved authors**, not people | 2026-10-19 |
| 16 | A list of tags the instance should show less of | **A published, versioned, instance-level de-prioritisation list.** D-18 and §6.10: the default is a **weight** of 0, which demotes the tag and removes nothing; a **block** is an explicit operator opt-in that excludes the works from browse and recommendation only, and the blocked set is **published in `/api/v1/meta`** so the sort contract exempts exactly that set. The default list is the owner's nineteen. It is the operator's and is **not** voted, and it never touches a reader's own library | 2026-10-19 |
| 17 | The corpus should grow by BFS over an author's works and bookmarks, as a deterministic, generally useful workflow | **D-19 and §9.10.** The machinery already exists in `~/code-local/ao3/baseline` (BFS, collectors, crawl progress with cursors, the parse cache, and an **offline `plan`** that prints exact request counts), so the work is a **contract**, not a rewrite: deterministic order, configurable sources and budget, resumable, honest about its own cost, posture-respecting. The expansion lands through the **metadata exchange**, in ≤ 200-signal batches, idempotent by content hash. **The 24-hour run is an operational instance of the workflow, not a mode of it** | 2026-10-19 |
| 18 | The site should be simpler, in the shape of novellunar.com | **§21.2 rewritten around it**, with the reference measured from the site: one flat seven-item top nav, one sidebar of 74 genres, a numbered "Weekly Trending" list, an `aspect-[3/4]` cover-card grid with a gradient scrim, **a layered dark scale — `#1a1a1a` / `#1e1e1e` / `#252525` — rather than black**, one sans family. Simplicity is a **testable** property: each of the six identity-bearing pages is measured against the reference | 2026-10-19 |

**Still open, and these are the owner's alone.** Each is a number or a preference, not a design:

11. **The Supporter price itself.** The mechanism is decided (§18) and the default is 5 EUR/month;
    the operator sets it, and it may be zero.
12. **`full_min_resonance`'s number.** 0.55 is a default derived from the agreement curve
    (§12.5); the operator may set it against the observed resonance distribution once there are
    readers.
13. **The Pi's memory allocation between Postgres, the archive and the taste service.** §22.3's
    2.5 GiB total is a frame, not a plan; the split is measured in M32 and decided there.
14. **Whether the instance theme's seed stays the owner's alone, permanently.** D-8 and D-11 make
    it so, and §12.12's votes apply to the *theme channels* rather than the theme's seed. If the
    owner ever wants the community to weight the instance theme itself, that is a different
    decision and it changes what "the instance's voice" means.

---
