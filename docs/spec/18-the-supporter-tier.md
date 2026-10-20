# 18. The Supporter tier — compute, not rank

**Status:** reserved
**Milestone:** M24
**Source:** the corresponding section of the Darkhold specification

Deliberately small, and deliberately about cost recovery.

- **What it grants:** the **T2 recommendation tier** (§12.6) and nothing else. If the operator
  also enables the "Moods" gate, it grants mood creation as well — but that is a *trust* gate
  (§12.3), not a payment gate, and the two are configured separately.
- **What it never grants:** rank, position in any list, a better score, moderation authority, trust,
  influence, or a larger library. **Paying buys compute, never rank** (D-10, N-1).
- **The model:** an **entitlement**, recorded in `supporter_subscriptions` and granted through
  `entitlements`. The grant's *source* is the operator's choice, and the entitlement is the
  mechanism, not the payment.

**The entitlement has more than one source, and it must.** D-9 makes the instance an adult-content
host by default, and **no mainstream payment processor accepts that content** — so a
payment-only tier is *unbuildable on the default instance*, which is the configuration the spec
otherwise assumes. A spec that quietly assumes a payment processor it cannot use is a spec that
quietly does not ship.

So the entitlement is granted by a **grant source**, and the operator may run any number of them:

| grant source | how it works | when it is the right one |
|---|---|---|
| **self-hosted** (the default) | the operator issues signed entitlement codes, or grants by account, or by invite code, or by a manual approval queue | the default, and the only one that works with D-9's content policy |
| **payment processor** | a monthly or annual subscription through an operator-chosen processor, with a webhook that grants and revokes the entitlement | only where the operator's content policy permits it, or where the payment is for something *other* than the tier (a tip, a donation, a merchandise item) |
| **federated grant** | a peer instance vouches for a reader's entitlement | an instance that federates and trusts its peers |
| **self-service purchase of a code** | the operator sells codes by hand, by transfer, by crypto, or through a page of their own | the practical middle ground when the operator wants money but not a processor relationship |

**The rules, whichever source:**

1. **The entitlement grants compute and nothing else** — the T2 recommendation tier, and the mood
   gate if the operator enables it as a tier entitlement. Never rank, never trust, never
   moderation, never a position in any list (D-10, N-1).
2. **A lapsed entitlement drops the reader to T1 at the end of the paid period and never below.**
   It never removes a bookmark, a rating, a review, a mood, a channel subscription, or a work. **The
   reader's data is not held hostage to the payment.**
3. **The grant is revocable and the revocation is audited.** A refund, a chargeback, or an
   operator decision ends the entitlement on a date, not by a background job that can fail
   silently.
4. **The core reading, writing, importing and community features work with no entitlement
   source configured at all.** The tier is an accelerator, never a prerequisite. That is the
   property that makes the self-hosted path the default rather than a fallback.

---
