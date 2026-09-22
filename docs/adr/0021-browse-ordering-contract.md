# ADR 0021 — One ordering contract for every browse surface

Date: 2026-09-21
Status: accepted

## Context

§16 specified the recommendation engines and §16.5 promised the reader's influence dial applies to
"all surfaces", but no section named the surfaces or defined how a browse page chooses an order.
Each route invented its own ordering: `/discovery` blended candidates, `/fandoms/:id` listed
"recent, popular, picks", and `/works`/`/library/items` took a free-form `sort` string. The result
was that "for you" existed in one place while the rest of the site ordered by whatever a route
author had written.

## Decision

§43 names the surfaces, defines one `sort` vocabulary
(`for-you | new | updated | top | trending | best-match | az`), and states the load-bearing rule:
**`for-you` is a permutation, never a filter.** The reachable set for a filter is identical under
every `sort` value. A taste-skewed default that removed results would be a shadowban applied by
ranking instead of by moderation — invisible by construction (§0.3 forbids explaining the order) —
so set equality is asserted by test rather than trusted by review.

Exact and literal queries are exempt (§15.10, unchanged): `for-you` orders eligible results, and a
literal query is not a recommendation request. Anonymous traffic gets a neutral default while the
instance's topics are empty or all private, because the *ordering itself* would otherwise publish a
private theme on the front page (§0.4.3).

## Rejected alternative

Per-surface bespoke ordering (the status quo ante): thirteen surfaces each defining "sorted by"
locally. It makes the dial's promise untestable, fragments the frontend's sort control, and leaves
the permutation guarantee unenforceable. A surface added later inherits the contract or it is a defect.

## Consequences

- `/admin/discovery` reports per-surface exposure (§43.6): without it, an operator cannot see
  `for-you` collapse into the same works on every page.
- §16's acceptance list gains the §43 cross-references; §16.5's "all surfaces" prose is now a
  pointer to §43.1 rather than a promise with no list.
