# ADR 0003 — Pseud isolation

Status: accepted (Milestone 0)
Date: 2026-09-10

## Problem

Spec §7 requires that each pseud has separate works, follows, messages,
recommendation settings, public bookmarks and notification preferences, while
the *account* holds credentials, security state, the private wallet and trust
eligibility — and that the site "does not publicly reveal shared ownership".

Two failure modes matter:

1. **Leakage:** any endpoint that answers "which account owns this pseud?" hands
   an observer the ability to link an author's separate identities — the one
   thing pseudonymity is for.
2. **Over-separation:** treating pseuds as full accounts would duplicate
   credentials, security and trust, and would let one person's pseuds vote as
   independent participants in governance (spec §18 forbids exactly that).

## Decision

- `accounts` own credentials, sessions, security state and trust; `pseuds` own
  everything public. `pseuds.account_id` is the *only* link, and it is never
  serialised into a public API response.
- **Ownership disclosure is a distinct, deliberately rare capability.** Code
  that reads `pseuds.account_id` for a response is a review trigger. The
  ordinary path is: authenticate → resolve the *active pseud* → operate.
- **Governance counts accounts, not pseuds.** Voting, reporting and quorum
  weight are computed on `account_id` (spec §18: "two pseuds from one account
  count as one participant"). The identity tables make that possible because
  the link exists server-side even though it is never published.
- **Voluntary linkage is opt-in and mutual.** `public_pseud_links` records only
  links a user chooses to publish. Its *absence* carries no meaning; the table
  must never be read as a complete map of who owns what.
- **404, not 403, for inaccessible private objects** (spec §3.3), so refusal
  does not confirm existence.

## Alternatives considered

**Pseuds as the only identity** (no account layer). Simplest public model, but
it has no place for shared credentials or a shared wallet, and it makes each
pseud an independent voter — which is precisely the Sybil problem spec §18
guards against.

**A separate "person" table between accounts and pseuds.** More faithful to the
idea that one human may hold several accounts. Rejected as speculative: it adds
a join and a concept to every authorization path to serve a case the plan does
not require. If it is ever needed, `accounts` can be split without changing the
public model, because the public model never mentions accounts at all.

**Exposing ownership behind a staff permission.** Rejected for now: spec §7
allows it ("only specifically authorized staff may retrieve private pseud
ownership"), but every such endpoint is a leak waiting for a bug. When it is
needed for a concrete legal or abuse process, it should be added with an audit
event and a reason code, not by defaulting the field into admin views.

## Consequences

- Authorization takes an `Actor { account_id, pseud_id, age_state, … }` rather
  than a user: policy has to be explicit about which of the two it is asking
  about. Existing policy functions are written this way from the start.
- Admin and moderation tooling must be built without a convenient "show me this
  pseud's account" affordance. That friction is the point.
- Account deletion must consider the pseuds it owns, and must not orphan
  published works (spec §22 covers the deletion semantics).

## Conditions that would justify revisiting

- A legal or child-safety obligation requires ownership disclosure to staff,
  at which point it is added as an audited, reason-coded operation.
- Abuse patterns show that account-level trust is being laundered through
  pseuds in a way the current accounting does not catch (for example, trust
  accruing per pseud rather than per account).
- Users ask for verifiable linkage ("these are both me") as a first-class
  feature rather than a manual profile link — that would need a signed claim,
  not a database row.
