# ADR 0013 — Gamification and quality metrics

Date: 2026-09-21
Status: accepted

## Context

§1.4 listed this ADR as `0013-no-gamification-decision.md`, but the file was never written.
Meanwhile §36 (Growth, sharing & ecosystem) already awards XP in three places — prompt
adoption, beta reading, instance migration — while §1.5 forbade introducing XP and §28.10
listed "XP and level progression" under "Deliberately not adopted". The ban had already failed
against the spec's own content: three sections used the unit, none deleted it.

The question was therefore not "should gamification exist" but "what may a gamification metric
count". The instance's premise (§0.2, priority 4) is that growth should maximize quality fiction
the instance is about, never quantity.

## Decision

Gamification is adopted with a two-tier rule, stated in §1.5 and developed in §9.7.1:

1. **Credits are a currency, not a rank.** Per-action rewards with daily and monthly caps
   (§9.7.2–§9.7.3) may reward capped *actions*. They are spendable and bounded, so they never
   compound into standing.
2. **Recognition is episodic, never cumulative.** Weekly and monthly leaderboards (§9.7.5) and
   badges (§9.7.6). A leaderboard placement expires with its window; a badge records that
   something happened once. There is no XP bar, no level, no lifetime point total and no
   cross-surface reputation score.

Every metric — a badge condition, a leaderboard category, a bounty trigger, a credit award — must
measure quality and the instance's declared themes (§0.4): completion, positive feedback,
contribution to the body of available fiction. Never word count, post count, works published, or
time online.

## Guards

- §9.7.5 forbids volume categories and all-time boards; a volume metric in a leaderboard is a
  specification error, not a configuration choice.
- No badge, placement or credit balance gates a free-core feature, rank-gates participation, or
  confers governance authority (§0.3, §19.1).
- No score computes, grants or accelerates trust: "Trust is not calculated from XP, post count,
  kudos received, credits earned, or any activity-volume metric" (§19.1).
- The one permitted per-surface signal — §35.2's forum karma, which decays monthly and confers
  nothing — stays exactly what it is: it displays and does nothing. It is the boundary case, not
  a precedent.

## Consequences

- §36's three XP references were rewritten to credits and badges. Nothing in §36's behaviour
  changed: the same actions earn the same recognition, denominated in units the spec allows.
- §18.2's "no XP" clause was deleted as obsolete; "no public ranking and no trust reward" stands.
- A reader-facing progress bar toward *influence over what others write* remains refused (§16.16.6):
  badges and leaderboard periods are progress toward recognition, not toward power.
- This ADR records explicitly what was considered and refused in the evaluation: XP (compounds and
  demands a source for every action), levels/points (a level is a rank that never expires),
  reputation (about persons rather than works; collides with §19.1 trust), and purchased standing
  (§0.3, §33.2). See the metric comparison in the working notes for the full arguments.
