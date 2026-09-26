# Lorehaven Spec Amendment — Vote Decay

**Status:** Final plan
**Date:** 2026-09-26
**Milestone:** M58
**Amends:** §39.4

---

## Overview

A directory vote used to be permanent. One vote per account per entry, at the
weight it had when cast, contributing forever. That is wrong in both
directions: an entry approved in 2019 and never revisited outranks one with a
lively, current consensus, and there is no way for a reader who now knows
better to change their mind without either editing a historical row or
re-voting in a way that silently replaces their previous position.

**The rule.** A vote is a *current statement of confidence*, not a permanent
ballot. It counts fully when cast and decays toward nothing over a
configurable window. Re-voting refreshes it to full. Decay engages only on
entries that have accumulated many votes, because a small entry's score is
already small and decaying it further just makes the list worse.

### Why decay at all

A permanent vote measures *when someone first noticed an entry*, not what
anyone believes now. The honest signal in a curated directory is "how much
does the current readership agree", and that is a decaying quantity. §39.4
already required that weight come from trust and never from credits or
purchases (§0.3); decay keeps the same property for *time* — a vote's
influence is bounded by how recently the voter reaffirmed it.

### Why not all-at-once

A vote that vanishes at the cutoff is a cliff, and a cliff is gameable: an
organised group votes the day before a cut and scores the maximum. The decay
ramp is the same shape as the trust ladder — gradual, bounded, and
indistinguishable in aggregate from an honest consensus.

---

## §1.0 Vote Decay → §1.0 The Decay Function

**Modification.** Weight at read time is `base × decay(age)`, where `base` is
today's trust-and-taste weight and `decay` is a pure function of the vote's
age.

**Decay shape.** Exponential toward a configurable cutoff:

```text
decay(age_days) = (1 - age/cutoff) ^ 2
```

The square makes the early loss gentle and the late loss fast, which matches
the stated intent — *daily* voting barely matters, *weekly* matters slightly
more, *monthly* barely at all, and a two-month-old vote is worth nothing. A
plain linear ramp would make a one-day-old vote worth 98%, which overstates
it relative to a vote cast yesterday.

**The cutoff is a hard zero.** At or beyond `cutoff`, weight is exactly 0.
Not "approaching zero", not a floor of 0.01: a vote that is still nominally
present at four months is a vote the instance is still counting, and the whole
claim is that it stopped.

**Parameters.**

| Parameter | Default | Meaning |
|---|---|---|
| `decay_enabled` | `true` | Master switch. `false` restores permanent votes. |
| `decay_cutoff_days` | `60` | Age at which weight is exactly 0. |
| `decay_min_votes` | `20` | Entries with fewer *live* votes never decay. |
| `decay_exponent` | `2.0` | The curve's shape. 1.0 is linear. |

**`decay_min_votes` is the important one.** Without it, a new entry with three
votes decays toward zero and never ranks, because the ranking signal it needs
is the one thing decay takes away. The threshold makes the rule match the
request: *few* votes keep their weight indefinitely; *many* votes decay.

The count is of **live** votes for that entry (weight > 0), computed at read
time, not of rows ever inserted. An entry whose votes have all decayed is
back to "few votes" and stops decaying — so it can recover by being voted on
again, which is the only way any of this improves rather than merely
diminishes.

---

## §2.0 Vote Decay → §2.0 Storage

**Modification.** `directory_votes` gains a surrogate key and stops being
keyed on `(entry_id, account_id)`.

```text
id          TEXT PRIMARY KEY
entry_id    TEXT NOT NULL
account_id  TEXT NOT NULL
vote_value  INTEGER NOT NULL CHECK (vote_value IN (-1, 1))
base_weight REAL NOT NULL      -- trust x taste at cast time
voted_at    TEXT NOT NULL
```

The existing `weight` column is renamed to `base_weight` and its meaning is
pinned: it is the **trust-and-taste weight at the moment of voting**, not the
weight contributed to the score. This is the change that makes re-voting
meaningful — a vote cast at TL2 and refreshed at TL4 carries the TL4 weight,
which is correct, because the reader's current standing is what the vote is
expressing.

**Uniqueness moves to a partial index**, so a voter has at most one *live*
row per entry while history remains queryable:

```sql
CREATE UNIQUE INDEX idx_directory_votes_one_live
  ON directory_votes (entry_id, account_id)
  WHERE weight > 0;
```

This is SQLite 3.8+ and PostgreSQL 3.0+, both satisfied. It is the only way to
express "at most one current vote" once rows are allowed to go stale, and it
makes the invariant a database guarantee rather than a transaction's
discipline.

**Migration** (`0048`): add `id`, add `base_weight`, backfill `id` with a
generated value per existing row, drop the old primary key, add the partial
index. Dual-dialect, per `migration-dialect-convention`.

---

## §3.0 Vote Decay → §3.0 Behaviour

**Modification.** Three cases, and the third is the one that was ambiguous.

### 3.1 Cast or refresh

Voting always sets the voter's row to the current `base_weight` and
`voted_at = now`. There is no accumulation: a user who votes fifty times has
one row worth one fresh vote, not fifty stacked. This is stated explicitly
because "voting every day" could otherwise be read as a way to farm influence,
and it is not one.

### 3.2 Flip

A vote that changes direction is **replaced**, not added. The old row is
removed and the new one starts fresh at full weight. Rationale: a reader who
changed their mind has not made two statements, they have made one.

### 3.3 Decay is computed, never stored

The score is recomputed from base weights and ages on read. Storing a decayed
weight would make the stored score wrong the moment the clock moves, and §39.4
already requires that a list never show a stale score — a stored decayed
weight satisfies that only until the next second.

**Consequence, and it is a real cost:** the score can no longer be a
denormalised column updated in the vote transaction, because decay moves it
without any vote happening. The score becomes a computed value from a
correlated subquery over the vote rows:

```sql
SELECT COALESCE(SUM(vote_value * base_weight * decay(age)), 0) ...
```

The `directory_entries.score` column stays for the *undecayed* case (an
operator with `decay_enabled = false`) and for entries below the threshold,
and the read path picks the cheap path when it can. Every entry in the Top
sort is recomputed, which on a directory of a few hundred entries is
microseconds and on a directory of a few hundred thousand is a sequential
scan — which is a reason for §4.1's recompute job.

### 3.4 The score still never exposes a weight

Unchanged and re-asserted: the response carries the score and the viewer's
own direction, never a weight. Decay makes this more important, not less — a
per-vote weight would now also be a timestamp-derived number.

---

## §4.0 Vote Decay → §4.0 Freshness

**Modification.** With decay on, a stored score is stale by definition. So:

- The score is recomputed on read for entries above `decay_min_votes`.
- A background job recomputes denormalised scores hourly for the same
  entries, so the Top sort has a cheap path and a cold list is not a stale
  one.
- Responses carry `score_computed_at` when decay is on, so a client can say
  "as of 4 minutes ago" rather than presenting a moving number as fixed.

**Why this matters for correctness, not just polish:** with decay on, two
reads a minute apart legitimately differ. A client that treats the score as
stable will show a number that was true when the page loaded and is not now,
which is a small dishonesty a reader can detect by refreshing.

---

## §5.0 Acceptance

- A vote cast today contributes its full trust-and-taste weight.
- The same vote at 14 days contributes more than at 30, which contributes
  more than at 59, which contributes exactly nothing.
- At and beyond `cutoff_days` the contribution is exactly 0, not a small
  number.
- An entry with fewer than `decay_min_votes` live votes does not decay at
  all, at any age.
- Re-voting restores full weight and does not stack.
- Flipping direction replaces the previous row.
- A voter's trust change takes effect on their next vote, and the stored
  base weight is whatever was true at cast time.
- With `decay_enabled = false`, behaviour is identical to today.
- The score is never returned together with a weight.
- `score_computed_at` is present whenever decay is enabled.

**Non-goals.** Per-vote weights in any response. Vote history UI. A decay
curve that depends on the voter's trust (base weight already does that).
Decay as a *disincentive* — a fresh vote is always worth more than a stale
one, so voting again is always rational, and nobody is locked out.
