# Browse-and-demand-weight implementation plan (M43)

Date: 2026-09-21. Read with `docs/spec.md` (§43, §16.15, §16.16) and ADRs 0021/0022.
Everything builds, everything is tested before it ships, and the default stays inert until the
guards land.

## The loop (same as every slice)

`requirements.csv` row → §4.6 tables → migration (both dialects) → domain logic →
routes/config → `milestone_43.rs`/`milestone_44.rs` tests → commit. One slice per commit, in the
order below. Migrations continue the sequence: 0050, 0051, 0052.

## Slice 1 — the `sort` vocabulary and one neutral surface (M43-01, M43-05 first half)

- Add a shared `Sort` resolver in `crates/domain`: parse, validate, name the accepted set in the
  error. List endpoints that do not take `sort` today keep behaving as before; `?sort=` on three
  neutral surfaces (works, people, tags) honours `new|updated|top|trending|best-match|az` and
  refuses anything else with the named error.
- Tests: unknown value error on each surface; exact queries bypass every profile at every weight.

## Slice 2 — `for-you` on three surfaces (M43-02, M43-04 first half)

- Reuse the existing `/discovery` blend (`blend → half-life → affinity`, then the §16.4
  reservations) behind `sort=for-you` on `/fandoms/:id`, `/tags/:tag`, `/people`.
- **Permutation test first**: fixture, `for-you` vs `new`, union equality in both directions,
  including a work `for-you` ranks last.
- Extend `route_inventory.rs` if it enumerates surfaces.

## Slice 3 — defaults, stickiness, anonymity (M43-03, §43.4)

- `[browse] default_sort/anonymous_sort/surface_defaults`; `browse_sort_preferences` per pseud.
- Anonymous on an undeclared/private-topic instance gets the neutral order even when `for-you`
  is requested; a no-profile signed-in request must equal the baseline byte-for-byte.
- Tests: journey 62 and 74 analogues.

## Slice 4 — per-surface diversity and exposure (M43-04)

- Apply the §16.4 reservations at every §43.1 surface, not only `/discover`.
- Add the §43.6 per-surface exposure counters (what each sort surfaced, overlap, first-seen
  share); operator-only, aggregate, no per-reader history.

## Slice 5 — taste sources (M43-06, §16.15; migration 0051)

- `taste_sources` / `taste_source_members` tables; `[discovery] taste_sources` config;
  `min_members` validated at startup with the named refusal.
- Opt-in and withdrawal flow; profile recomputation on withdrawal; versioning per source.
- Tests: 3-member refusal, 5-member acceptance, member-withdrawal recompute, no dial change for
  any remaining reader.

## Slice 6 — reason degradation and no-leak (M43-07)

- `reason` may name an instance term only for a `public = true` topic; otherwise one
  undifferentiated line. Apply to `/discover` output, saved responses, digests, and the
  M29 "why am I seeing this" explanations.
- Tests: assert *absence* of theme terms on all surfaces below public topics (§41.3's pattern).

## Slice 7 — the demand primitive (M43-10; migration 0052; land behind `flat`)

- Promote `domain::vote_weight` (M39) to the shared primitive with trust/taste/contribution
  terms and the `[weighting]` config (`mode`, `taste_floor/taste_ceiling`,
  `contribution_floor/contribution_ceiling`, `contribution_window_days`).
- `demand_weights` / `demand_weight_history` / `demand_signal_events` tables; domain-scoped
  keys; windowed decay over `contribution_window_days`.
- **Ship with the behaviour off**: `mode` accepted, default applied, but all call sites pinned to
  `flat`-equivalent until slice 9. Tests assert the off switch is identity.
- Floor×ceiling `≤ 1.0` validated at startup; both values published on `/api/v1/meta`.

## Slice 8 — wire the three demand call sites (M43-08, M43-09, §18.5, §20.5)

- Wishlist votes, bounty visibility/queue, prompt votes, "write next" ordering consume the weight.
- Only §16.16.1's permitted metrics feed the contribution term; everything else is ignored by
  construction, and a test feeds a *forbidden* metric (works published, hours online) and asserts
  it moves nothing.
- Bounty size is plumbed to fulfillment priority only; a funded bounty changes its funder's weight
  by zero (asserted).
- Tests: identical contribution records in disjoint fandoms produce equal weights outside them and
  different weights inside; badge/leaderboard placement for the same behaviours pays the visible
  reward only.

## Slice 9 — the guards (M43-12, M43-13, M43-14)

- `demand_diversity_percent > 0` enforced; surfaced-without-boost accounting in
  `demand_diversity_state`.
- Majority-integrity: compute both weighted and unweighted outcomes; disagreement routes to
  §19.4 quorum instead of resolving. Also apply to §39.4's directory voting in a **separate,
  bisectable commit**.
- Flip the default from `flat` only after slices 9 and 10 are green.

## Slice 10 — absence, checklists, journeys (M43-11, M43-15, M43-16)

- The absence test across every endpoint, export and log shape; journey 62–74 analogues in
  `milestone_43.rs` (slices 1–6) and `milestone_44.rs` (slices 7–9).
- §28.11 checklist rows; the metric-category refusal test (words written, hours online, all-time).
- `docs/plans/junior-implementation-plan.md` gets the M43 row if the loop still runs through it.

## Slice 11 — frontend (folded into the M32/M33 frontend pass)

One design-system change: the shared sort control on every §43.1 surface plus the standing-free
weight-blind UI. No new vocabulary in the interface — `for-you` is labelled exactly as specced,
and no surface invents a second label for the same order.

## What deliberately ships last

The default flips only after the guards, the absence tests, and the exposure report are green.
Until then `mode = "flat"` is the honest state, and every paragraph of §16.16 that says "never"
is a test before it is a claim.
