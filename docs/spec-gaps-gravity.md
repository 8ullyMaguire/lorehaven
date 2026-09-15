# GRAVITY features absent from the Lorehaven plan

Status: **review draft — proposed, not adopted.** Nothing here has been added to
`docs/spec.md` or `docs/requirements.csv`. This document exists so the decision
is made explicitly rather than by omission. It follows the same pattern as
`docs/spec-gaps-ficnexus.md`.

## How this was produced

The comparison is between the plan in `docs/spec.md` and GRAVITY
(`~/code/rust/gravity` — Platform Spec v4.14; a separate codebase sharing no
code with this one). The owner requested this note specifically for the
collection-ranking gap; the observations at the end are secondary and
informational.

---

## A. Collection fit-ranking (the requested note)

**Lorehaven today (M13):** collections are curatorial groupings of works —
`collections` / `collection_roles` / `collection_submissions` /
`collection_entries`, with an open/closed submission policy. Ordering inside a
collection is curator whim or recency. There is no way to answer the reader
question *"which of these works fits this collection's theme best?"* — and no
way for a collection to rank its own entries without turning into a popularity
contest (kudos-count ordering would reward the famous, not the fitting).

**GRAVITY's answer (§5.23 directories + §11.12 entity ranking, Platform Spec
v4.14):** rank entries on **multiple named axes across different dimensions**
so popularity cannot become the axis:

- The collection defines its own **fit axes** — e.g. `theme_fit`,
  `completeness`, `tone_match` — each with a weight. Instance/collection-level,
  not platform-wide.
- Eligible members (curators, or submitters above a trust floor) rate entries
  **per axis**; a work can be brilliant on `tone_match` and mediocre on
  `theme_fit`, and the divergence is the signal — a work that scores
  identically on every axis correlates with raw rating volume and gets
  **dampened** (`adjusted = raw × (1 − w × volume_correlation_penalty)`).
- **Confidence ships with every score** (rater count × inter-rater agreement):
  3 agreeing raters display differently from 50 split ones, and nothing is
  shown below a minimum-count threshold — Lorehaven already does exactly this
  for the public rating aggregate ("states its method and count above a
  minimum"), so the pattern is native here.
- **Niche bonus:** entries covering the collection's underrepresented sub-themes
  rank up, so a 500-entry collection doesn't collapse to its five most-kudosed
  works.
- **Popularity is never an axis**, and a popularity-correlation penalty is a
  first-class, governance-settable weight.
- Optional: **frozen snapshots** (a collection can freeze its ranking at a
  point in time — useful for themed reading lists and challenge winners).

*Touch if adopted:* `docs/spec.md` §4.4 (add `collection_fit_axes`,
`collection_fit_ratings` beside the M13 collection tables), §15 discovery
(surface the ranked view), §22 (axis governance). Fits naturally alongside M13
collections or the M15 discovery milestone; nothing else depends on it.
Everything is additive.

## B. Other GRAVITY mechanisms relevant if ever adopted (informational)

- **Typed votes instead of a single kudos** (GRAVITY §3): quality-judgment
  categories with daily budgets; Lorehaven's private-rating aggregate pattern
  (method + count + threshold) would carry over unchanged.
- **Obfuscated aggregates toward others** (§3.5): tiered display kills
  leaderboard-chasing in collections and author rankings alike.
- **Multi-axis confidence display** (§11.12) is the general form of what §4's
  rating aggregate already does; adopting it once would serve ratings,
  reviews, and the fit-ranking above.
