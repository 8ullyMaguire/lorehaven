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

**Decay shape.** A power curve toward a configurable cutoff:

```text
t            = 1 - min(1, max(0, age / cutoff))
decay(age)   = t ^ exponent
```

`exponent` is an **integer**, defaulting to 2. That is not an aesthetic
choice and §2.1 records why it cannot be anything else.

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
| `decay_min_votes` | `20` | Entries with fewer *votes* never decay. |
| `decay_exponent` | `2` | The curve's shape, an integer. 1 is linear. |

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

## §1.5 Vote Decay → §1.5 The Exponent Is an Integer

**Why.** The curve is computed in SQL as well as in Rust (§3.3), and
**sqlx's bundled SQLite has no math functions at all**. `POWER`, `exp`, `ln`
and `sqrt` all return `no such function: POWER` and friends — `SQLITE_ENABLE_
MATH_FUNCTIONS` is a compile-time flag the bundled build does not set. Only
integer powers are expressible in portable SQL, because an integer power *is*
repeated multiplication.

A fractional exponent would therefore be computable in Rust and unreachable in
the database, and the two would disagree on every score with nothing to report
it. The Rust side therefore multiplies in a loop rather than calling `powf`, so
that both dialects perform the same operation on the same numbers, and
`vote_decay_parity.rs` asserts the two agree to 1e-3 across a table of ages and
parameter sets on both backends.

The 1e-3 tolerance is a bound on the *engines'* arithmetic, not on the curve.
If it ever needs loosening, the cause is a dialect implementation of a
primitive, not the design.

**Corollary.** `1.0 - age/cutoff` must be clamped into `[0, 1]` *before* the
power. Without the clamp, a vote older than the cutoff gives a **negative**
base raised to an even power, which is positive on both engines: a 400-day-old
vote would score `(1 - 400/60)² = 28.4`, i.e. one stale vote outweighing 28
fresh ones, and both backends would agree, so no dialect test would catch it.
The clamp is asserted structurally in the parity test for exactly that reason.

---

## §2.0 Vote Decay → §2.0 Storage

**Modification.** None required. This is the part of the design that turned
out not to need doing, which is worth recording so nobody "fixes" it later.

The existing `directory_votes` is keyed `(entry_id, account_id)` with
`weight` and `voted_at`. Under §3.1 a voter has exactly one row per entry at
any time, so that key is already the right constraint, and re-voting is an
UPDATE — which resets `weight` to the current trust-and-taste value and
`voted_at` to now. Refresh comes for free.

**What changes is the meaning of the `weight` column.** It becomes explicitly
*the trust-and-taste weight at the moment of voting*, never the weight
contributed to the score. Today the comment says "recomputed on trust/taste
change", which is a promise the code does not keep and decay would make
actively misleading: a weight that was recomputed on trust change could not
also be a function of age, and the distinction is the whole mechanism.

Rename to `base_weight` for the same reason the analytics milestone renamed
`weight` → `base_weight` elsewhere: a column whose name does not say *which*
of three meanings it carries is a bug waiting for a reader who assumed the
other one. This is a rename with no behaviour change, so it is safe.

**No partial index.** An append-only design would need one
(`WHERE weight > 0`), but with refresh semantics there is never a stale row to
be confused with a live one — every row is current by definition. The index
that exists (`idx_directory_votes_entry`) is still what the score query needs.

**Migration** (`0079`): `ALTER TABLE directory_votes RENAME COLUMN weight TO
base_weight`, in both dialects, with no data movement. SQLite 3.25+ and
PostgreSQL both support the rename, and it is a schema-only change, so
`cargo test --test migrate` covers it.

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
without any vote happening. `cast_vote` today recomputes and stores
`directory_entries.score` in the same transaction as the vote, which §39.4
requires and which is the right design for permanent votes. With decay it
would be wrong the moment the transaction committed, so the score becomes a
computed value from a correlated subquery over the vote rows:

```sql
SELECT COALESCE(SUM(vote_value * base_weight * decay(age)), 0) ...
```

The `directory_entries.score` column stays and is still maintained — it is
the cheap path for an instance with `decay_enabled = false` and for entries
under `decay_min_votes`, and the read path picks the cheap path when it can.
**`decay(age)` has to be computed in SQL, not in Rust**, which means the
curve exists in two places, and the SQL cannot be a literal transcription of
the Rust: three dialect facts force it to be *constructed* per backend rather
than written once.

1. **No math functions on SQLite** (§1.5) — the power is a product, not a
   `POWER` call.
2. **PostgreSQL has no scalar two-argument `min`/`max`.** Those are aggregate
   functions; `max(double precision, double precision)` does not exist and the
   query fails at plan time with **42883**. The scalar forms are
   `LEAST`/`GREATEST`.
3. **PostgreSQL's bare decimal literals are `NUMERIC`**, so an uncast `0.0`
   makes `GREATEST` fail to resolve against a `double precision` operand —
   42883 again, from the same class of mistake.

The age is fractional in both dialects (`julianday` on SQLite,
`EXTRACT(EPOCH …)` cast to `double precision` on PostgreSQL); an integer day
count would make the curve a staircase, and "full weight for 24 hours then a
step down" is not the rule anyone asked for.

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
- An entry with fewer than `decay_min_votes` votes does not decay at
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
