# ADR 0020: Redistribution floor

- Status: accepted
- Date: 2026-09-21

## Context

Work monetization (ADR 0017, §20.9) pays authors through tips, early access,
purchase and patronage, but places no ceiling on direct-attribution income and
redistributes nothing. Every predecessor platform that paid by volume
(Wattpad Paid Stories, Webnovel contracts, Kindle Vella) collapsed into
winner-take-all dynamics or AI-generated floods. Separately, §2.1 named
Stripe as the billing adapter — a processor whose terms prohibit the adult
content a default Lorehaven instance hosts.

## Decision

1. **§2.1 names no processor.** The billing adapter is operator-chosen; the
   operator owns content-policy compatibility. The spec stays vendor-agnostic.
2. **§20.10 splits author-bound revenue into two pools.** Pool A (direct
   attribution: tips, reading-time-attributed subscriptions, bounties,
   marketplace) and Pool B (solidarity: cap overflow plus per-flow
   contributions). Splits are configurable, published on `/api/v1/meta`, and
   change on 90 days' notice.
3. **A soft graduated cap** (100% to 5× median, 50% to 10×, 0% above) on a
   trailing 3-month active-earner median spills overflow into Pool B without
   excluding the capped author from solidarity.
4. **Pool B distributes by a quality composite** — completion, reread,
   positive-feedback density, bookmark rate, long tail, trust-weighted reader
   diversity — never by word count or work count.
5. **§20.9.4 requires an AI content declaration** per monetized work
   (`none | assisted | co-written | generated`). `generated` is Pool
   B-ineligible, `co-written` contributes at 0.3×. The defense against AI
   flood is economic structure plus declaration honesty (quorum-decided), not
   a detection arms race.
6. **§20.9.5 records processor fees** per transaction; pool math runs
   post-fee, and the trailing fee range is published.

## Consequences

- New tables (§20.10.9): `pool_b_distributions`,
  `monetization_period_summaries`, `work_ai_declarations`; `payment_events`
  gains `processor_fee_minor`. Migration 0049, both dialects.
- §9.7.4/§9.8 quality signals now feed monetary distribution, not only
  credits; their anti-gaming rules (§9.7.8) carry over unchanged.
- §34.5's signals-never-verdicts contract governs AI-declaration flagging:
  classifiers suggest, quorum decides.
- The operator's compliance surface grows: processor selection, quarterly
  financials, 90-day parameter notices.
- Free core (§0.3) untouched: reading, commenting, participation never
  require payment; a billing-disabled instance has none of this.
- Calibrating the multiplier and weights against real distribution data is
  deferred by design — parameters are configurable precisely so the first
  year of data can revise them.
