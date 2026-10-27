# Spec amendment — the draw slot (§16.10a)

**Status:** adopted 2026-10-27. **Base:** `ultimate-fanfiction-site-spec.md` (Darkhold v2.3).
**Origin:** `ascend` (`~/code/rust/ascend`, `docs/SPEC.md` v3.0) — the "draw your powers" mechanic,
folded into discovery rather than kept as a second site.

## What a draw is

A reader asks for a hand of cards. The instance deals **k works from the reader's own ranked
candidate pool** — three by default — presented one card each. The reader takes one, or asks for
another hand.

A draw is **a presentation of the existing recommendation pipeline**, not a second recommender. The
pool is built by `routes::discovery::build_candidate_engines` and merged by
`darkhold_domain::discovery::blend` — the same candidate set `/discover` ranks, ordered by score —
and the deal only decides *which* of them the reader sees this time. Nothing here re-ranks: a work's
chance of appearing is its rank's chance, and a work outside the pool cannot be dealt.

*Not* the strategy registry, deliberately. `rec_mode = "pluggable"` runs
`rec_strategy`'s strategies, and on this tree their queries decode work ids as `TEXT` out of a
`UUID` column on PostgreSQL, so that path fails on the engine the instance actually serves through.
A draw built on it would either 500 or, worse, deal from a pool under a mode the reader's feed is not
using — two pools with two sets of bugs is exactly what this slot must not be.

## Why it is a slot and not a page

`recommendation_slot` already answers "what did this instance put in front of this reader, and
why" (§12 north-star attribution). A dealt card is a served slot, so it is recorded as one:
`mechanism = 'draw'`, `position` = the card's place in the hand, `reasons = ["drawn"]`. The take is
recorded on the same row (`chosen_at`), so the disposition is one column away from the serving
decision it is about. A hand that was dealt and not taken is then readable as exactly that — passed
slots carry no `chosen_at`.

**One take per hand, and the first one wins.** The columns are per card, so the constraint that makes
a hand a *choice* has to be in the statement: the update refuses when any card of the same request is
already taken, and the route answers with the take the hand holds rather than the one that was posted.
Without it, two taps on two cards recorded two takes — which is a second choice, not a choice, and it
would have quietly poisoned the only signal the draw collects.

## The deterministic deal

The deal is seeded by `sha256(account_id ‖ date ‖ nonce)` and dealt by partial Fisher–Yates over the
pool's indices (`darkhold-domain::draw::deal`). Consequences, all deliberate:

- asking twice with the same nonce on the same day returns **the same hand** — a refresh that
  redealt would make the reader's second look a different question;
- the nonce is how a reader asks for a *different* hand, and `?nonce=2` is a different question;
- the deal is testable without a database, because it is a pure function of (seed, pool, k).

## Non-goals (and why the mechanic stops here)

Ascend deals **rarity**, offers a **pity counter**, spends an **advantage**, and assembles a
**collection** out of what you drew. None of that is adopted, and the reason is the specification's
own decisions rather than a lack of ambition:

- **D-1** forbids a progression economy: no levels, no XP, no currency that withholds something.
  Rarity tiers plus a pity counter is a collection economy in everything but name.
- **N-1** forbids anything purchased moving rank, position or a slot.
- A card's "power" has no referent in an archive. A work is a work; its rarity would be an opinion
  about the reader dressed as a property of the text.

What *is* adopted is the part that was a discovery mechanic rather than an economy: **a hand, a
reveal, and a choice** — and the choice is the signal, which is the only part of Ascend's draw that
teaches the recommender anything (`draws.chosen_index` in Ascend's schema is the same idea).

## Surface

| method | path | who | answers |
|---|---|---|---|
| GET | `/api/v1/discovery/draw?nonce=0` | signed in | `{request_id, date, nonce, cards: [{work_id, title, summary, position, slot_id}]}` |
| POST | `/api/v1/discovery/draw/pick` | signed in, owner of `request_id` | `{work_id, chosen_at}` |

An empty pool is `200` with `cards: []` and `empty_reason`, never `Internal` — the same rule §54.6
and §100-ideas §3 state for every rail.

## Migration

`0128_draw_choice.sql` adds `recommendation_slots.chosen_at TEXT` (both dialects, additive, no
backfill: a slot nobody took has no take time). **The number skips 0127 on purpose:** 0127 is the
uncommitted M6-05 character work in the local tree, and this migration is independent of it. Apply
order does not matter — both are additive statements on different tables.
