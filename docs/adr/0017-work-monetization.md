# ADR 0017: Work monetization

- Status: accepted
- Date: 2026-09-14

## Context

The platform's revenue is credits, subscriptions and marketplace fees, but
nothing pays authors. A positivity-first archive that funds infrastructure
but not writers asks writers to subsidize everyone else's hobby.

## Decision

Authors may be paid through four models — **tips, early access, purchase,
patronage** — subject to an instance eligibility setting: `original` (default),
`any-with-assertion`, or `disabled`.

- Money and credits stay in separate ledgers. The platform never converts
  credits to money; credit tips transfer credits between wallets and stop there.
- Early-access chapters unlock permanently at `public_at`; nothing is ever
  locked retroactively.
- Entitlements bind to the account, survive pseud switching and pricing
  changes, and are checked server-side like every other policy.
- Paid works gain no ranking, recommendation or moderation advantage; a
  paying reader's comments pass the positivity filter unchanged.
- Self-dealing is refused: purchases and tips between pseuds of one account
  are rejected and earnings never accrue from the author's own reading.

## Consequences

- `work_pricing`, `work_entitlements`, `author_earnings_ledger`, `payouts`,
  `monetization_assertions` are new tables (migration 0022).
- The eligibility setting is an instance setting; `any-with-instance` widening
  re-asserts rights at price changes.
- Payouts are the operator's compliance surface, as with marketplace payouts.
- Free core (§0.3) is not diminished: reading, commenting and participation
  never require payment.
