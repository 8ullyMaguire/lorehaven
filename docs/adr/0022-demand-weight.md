# ADR 0022 — Taste-weighted demand: trust × taste × contribution, weight as computation

Date: 2026-09-21
Status: accepted

## Context

The instance wants aligned readers to have more pull on what gets written next — wishlist votes,
bounty visibility, prompt votes, "write next" opportunities — without letting alignment become a
purchased or inferred editorial authority. §39.4 already shipped the pattern for one surface
(`trust_multiplier × taste_multiplier`, M39 green): the question was whether to promote it, what
to add as the quality term, and which guards keep it honest.

## Decision

One primitive in §16.3, three applications (§39.4, §18.5, §20.5), specified in §16.16:

```text
demand_weight = trust_multiplier(trust_level)           // safety ramp, §19.1 ladder
              × taste_multiplier(theme_affinity)        // silent; floor … ceiling
              × contribution_multiplier(domain_record)  // earned; windowed, decaying
```

Everything that makes this defensible, in order of importance:

1. **No new unit.** No XP, levels, points, reputation, karma or "standing". The weight is computed
   when used from signals the instance already keeps, and is never accumulated into a number a user
   owns (§9.7.1, §28.10; see the metric comparison in the working notes for why each named unit
   was refused). Visible recognition stays episodic: weekly/monthly leaderboards and badges.
2. **The floor and ceiling are configuration** (`[weighting] taste_floor/taste_ceiling`,
   `contribution_floor/contribution_ceiling`), but the relationship is not: `floor × ceiling ≤ 1.0`
   is refused at startup, and both values are published on `/api/v1/meta`.
3. **Majority integrity.** A weighted bloc may reorder demand; it may never reverse a plain
   majority. Disagreement routes to §19.4 quorum review. This also closes a gap in §39.4 itself.
4. **A demand diversity budget** (`demand_diversity_percent`, default 20, must be `> 0`): the
   §16.4 budget applied to supply, so the queue cannot become monothematic even when the multiplier
   is working exactly as intended.
5. **Bounty size buys fulfillment priority, never weight** (§0.3, §33.2). A funded bounty raises
   its own request and changes its funder's weight by zero.
6. **The contribution term is domain-scoped, windowed, and lists its permitted signals
   exhaustively** (§16.16.1). A metric outside the list is a defect, not a tuning choice.
7. **Visibility follows topic visibility** (§16.16.2): the mechanism is always documented, its
   components are silent unless a topic is public, and alignment never adds to anything a user can
   see. A test asserts the absence across every surface (§41.3's pattern).

## Rejected alternatives

XP as the unit (compounds; demands a source for every action); a public "influence score" (a
taste-derived public standing broadcasts the theme); influence purchasable with credits or bounty
size (§0.3, §33.2); trust as the whole value (conflates governance with taste; fails on
single-admin instances); weight touching published-work ranking (§15.10), trust, or moderation
(§34.3's "never decided here"); an anonymous `for-you` on private-theme instances (§0.4.3); a
reader-facing progress bar toward influence (streak-shaped, §0.2).

## Consequences

- `domain::vote_weight` (M39) is promoted to the shared primitive with a third term, rather than
  forked into a second weight implementation.
- §33.2's "from trust alone…" precedent now explicitly extends to any future gamification unit:
  money-derived signals feed neither rating weights nor demand weights.
- §34.3's "never decided here" list keeps its boundary: a weight orders the unwritten and can
  never decide a sanction, shadowban, DMCA case, payout, trust level, or AI-training verdict.
