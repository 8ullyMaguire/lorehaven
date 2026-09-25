# Trust accumulation — review draft, not adopted

Status: **proposed, not adopted.** Nothing in this document has been written to
`docs/spec.md`, `docs/requirements.csv`, or any code. It exists for review only.
Per ADR 0023 the CSV is the canonical feature inventory; if this is adopted, the
CSV rows come first.

Date: 2026-09-25. Supersedes nothing. The plan of record remains
`docs/plans/remaining-work.md`.

---

## 0. Summary of the proposal

Trust becomes a quantity that moves over time rather than a label a moderator
sets by hand. Five mechanisms:

1. **A continuous score** alongside the existing TL0-TL6 label.
2. **Asymmetric gain and loss** - gaining is slow and steady, losing is fast.
3. **Multipliers** on both directions, per-account, operator-adjustable.
4. **A moderation penalty** that lowers trust and starts a recovery timer.
5. **Taste similarity to the instance theme** as a gain multiplier, and as a
   floor required to be promoted past a level.

---

## 1. The finding that has to be resolved first

**`lorehaven_db::governance::set_trust` has no production callers.**

Grepping every call site returns test harnesses only. Most of the `set_trust`
grep hits are `set_trust_proxy`, an unrelated function in `server.rs` that
configures whether to honour `X-Forwarded-For`. The only two real callers of the
trust setter are `crates/app/tests/milestone_14.rs:359` and
`crates/app/tests/milestone_22.rs:641` - both tests.

So today: trust is a number in a table, set by hand, never computed. Nothing
raises it, nothing lowers it, and the ladder in spec 19.1 is aspirational except
for the gates that read it (`M12-02` forum gates, `M45-03` voting, `private`
instance mode at TL>5).

This matters for how the rest of the document reads. Adding a score, gain rates,
loss rates, multipliers and a taste gate is not "adding a feature to a working
system" - it is building the trust system the spec has always described and
that has never been implemented. That is worth doing, but it should be a
conscious choice rather than a surprise.

**Consequence:** section 3 is not optional groundwork. The mechanisms in
section 2 are inert without something that moves trust at all.

---

## 2. The five mechanisms

### 2.1 Continuous score

Keep `trust_levels.level` as the discrete label that the existing gates read.
Add a continuous score that moves continuously, and derive the label from it.

| Column | Type | Meaning |
|---|---|---|
| `score` | `INTEGER` | basis points, -10000..+10000 |
| `peak_score` | `INTEGER` | high-water mark, for the regain rule |
| `level` | `INTEGER` | TL0-TL6, derived |
| `modifier_bp` | `INTEGER` | operator multiplier, default 0 |
| `penalty_bp` | `INTEGER` | live moderation penalty |
| `recovery_blocked_until` | `TEXT` | set while a sanction is live |
| `updated_at` | `TEXT` | |
| `last_gain_at` | `TEXT` | for the gain-rate ceiling |
| `last_loss_at` | `TEXT` | for the loss floor |

Score is stored in basis points so the arithmetic stays integer-only and a TL3 at
8000bp is a natural normal rather than a floating-point special case.

New table `trust_scores`, keyed by `account`. The `level` column duplicates
`trust_levels.level` deliberately: the existing gates read `trust_levels`, and
rewriting every gate to join a second table is churn for no benefit. One writer
per table, paired, with a test asserting the two never disagree.

### 2.2 Asymmetric gain and loss

The core ask. Two rates, deliberately unequal.

```
gain = base_gain_bp * multiplier * taste_factor     (per qualifying event)
loss = base_loss_bp * multiplier * penalty_factor    (per qualifying event)
```

| Constant | Default | Note |
|---|---|---|
| `base_gain_bp` | 40 | per qualifying event |
| `base_loss_bp` | 300 | ~7.5x the gain rate - this *is* the asymmetry |
| `loss_floor_bp` | 150 | minimum charged on any loss event |
| `loss_ceiling_bp` | 2000 | no single event zeroes an account |
| `gain_rate_ceiling_bp_per_day` | 800 | caps accumulation speed |
| `gain_dwell_days` | 30 | days at the ceiling to go TL0->TL1 |

A loss event also arms the regain rule: after a penalty, the account must return
to `peak_score * 0.7` before its label may advance again. The label can still
drop immediately; it cannot climb back to its old value until 70% of the
pre-penalty high-water mark is recovered. That is the "takes longer to earn back"
requirement, and it is why `peak_score` exists.

`loss_ceiling_bp` matters as much as the rate. Without it, one bad report
resolving badly zeroes a four-year account, and the asymmetry stops being
"gained slowly, lost fast" and becomes "one mistake erases everything".

### 2.3 Multipliers

Per-account, set by the operator, additive across named reasons so an account can
carry "long-standing member" and "new but demonstrably good" at once.

| Reason | Default | Kind |
|---|---|---|
| `long_standing_member` | +500 | operator-set, permanent |
| `demonstrably_good` | 0 | operator-set, permanent |
| `post_sanction_cooldown` | -2000 | set on sanction, expires with it |
| `taste_alignment` | -1500..+1500 | computed, see 2.5 |

`taste_alignment` is the only computed one. The other three are operator
discretion and may be negative with the audit log as the only explanation.

### 2.4 Moderation penalty and recovery

On `issue_sanction` at `ReadOnly` or above:

- `penalty_bp` set to the sanction's configured value (default 1500)
- `recovery_blocked_until` set to the sanction's end, or now + 30 days if
  open-ended
- `last_loss_at` stamped

`VerbalWarning` and `PostThrottle` do **not** move trust. Spec 19.1 requires
*reviewed conduct*; a throttle is a tool, not a verdict. A warning is not
evidence that someone's trust should fall.

During an open sanction the account still accumulates score - it just cannot
advance a label. Still accumulates, so returning from a sanction does not start
from zero.

**Appeal interaction (spec 19.10).** If a sanction is reversed on appeal, the
trust penalty is refunded in full and `recovery_blocked_until` cleared, because
the conduct was never adjudicated. If reduced rather than reversed, the penalty
scales with the reduction. If the sanction ends by expiry instead, the penalty
*decays* rather than being refunded - expiry is an administrative event, not a
vindication, so the hit is treated as partly deserved.

This has to be implemented in `decide_appeal`, not left as a follow-up. A
reversed sanction that leaves a permanent invisible trust hit is the kind of bug
that is invisible until someone appeals successfully and is still treated as
untrustworthy.

### 2.5 Taste similarity: multiplier and promotion floor

Instance theme is `instance_themes.theme_vector` (migration 0043,
`crates/db/src/instance_theme.rs`). Per-user taste is
`lorehaven_db::taste_vectors::get_taste_vector`, which returns
`Option<(Vec<f64>, f64, String)>`.

Similarity is a cosine over the two vectors, **quantised to integer basis
points before it reaches any trust arithmetic**. This is not cosmetic. Two
accounts whose similarity differs below the quantum must produce an identical
trust outcome, or the same conduct yields different trust depending on floating
point noise. Quantise first, then multiply.

```
taste_factor = clamp(1.0 + alignment * 0.5, 0.5, 1.5)
```

A user aligned with the instance theme gains at up to 1.5x, an opposed one at
0.5x. Both are asymptotic, since perfect alignment is unreachable in practice.

Promotion floors, configurable per level, `-1` meaning "no floor":

| Transition | Default floor |
|---|---|
| TL0->TL1 | none |
| TL1->TL2 | 0.00 (off) |
| TL2->TL3 | 0.10 |
| TL3->TL4 | 0.20 |
| TL4->TL5 | 0.30 |
| TL5->TL6 | 0.40 |

The floor is a promotion gate only. It never blocks reading, posting, or
anything else - a member with opposed tastes still participates, they just do
not advance into the levels that grant stewardship. The reason for that split:
tying a governance privilege to taste similarity is precisely what spec 16.16.2
forbids, and the floor has to be an *entry* threshold rather than a standing
condition so that it gates promotion without making governance contingent on
taste.

---

## 3. What actually moves trust (prerequisite)

Section 2 needs events. Proposed, smallest useful set:

| Event | Direction | Qualifies |
|---|---|---|
| Reviewer quorum approval of a proposal | gain | the proposal passed, not the proposer voting |
| A published comment surviving review | gain | first-time pass only, not every comment |
| A held destructive comment upheld | loss | upheld, not merely held |
| A sanction issued at ReadOnly+ | loss | see 2.4 |
| Appeal reversed | refund | see 2.4 |
| Time in good standing with no events | gain | slow drip, see `gain_dwell_days` |

Deliberately **not** qualifying: post count, kudos, credits, ratings, views,
reading streaks, marketplace purchases, subscription tier, XP. The spec already
forbids most of these (lines 1277, 1532, 3188, 4784, 4898) and the user's
proposal does not ask for them.

The "surviving review" gain is the one worth arguing about. It rewards good
conduct, which is what trust is for, but it is an activity signal, and the spec
is wary of activity-volume metrics. The mitigation is that it counts *distinct
pieces passing review*, not comments posted, and that the daily ceiling caps it
regardless.

---

## 4. Config surface

New `[trust]` section. Every rate is operator-adjustable, because the defaults
below encode one taste of community and another instance will want different
numbers.

| Key | Default | Meaning |
|---|---|---|
| `enabled` | `false` | off by default; see section 6 |
| `base_gain_bp` | 40 | |
| `base_loss_bp` | 300 | |
| `loss_floor_bp` | 150 | |
| `loss_ceiling_bp` | 2000 | |
| `gain_rate_ceiling_bp_per_day` | 800 | |
| `regain_threshold_bp` | 7000 | percentage of peak, as bp |
| `taste_multiplier_strength` | 50 | the 0.5 in 2.5, as a percentage |
| `taste_floor_tl2` .. `taste_floor_tl6` | see 2.5 | |
| `sanction_penalty_bp` | 1500 | |
| `penalty_decay_bp_per_day` | 50 | |
| `open_sanction_recovery_days` | 30 | |

Startup validation, as with the instance mode: a misspelled key or a
`base_loss_bp` below `base_gain_bp` stops startup rather than silently
defaulting. A trust system that silently runs at the wrong rate is worse than
one that refuses to start.

---

## 5. Where it goes in the code

| Piece | Location |
|---|---|
| score arithmetic | new `crates/domain/src/trust.rs` - pure, no IO, fully unit-testable |
| persistence | extend `crates/db/src/governance.rs` |
| schema | new migration, both dialects, matching the existing pair |
| event hooks | `issue_sanction`, `decide_appeal`, plus a new `record_trust_event` |
| gates | unchanged - they keep reading `trust_levels.level` |
| admin surface | new route group under the existing operator-gated area |
| reader surface | new "why am I at TL2" panel on the account settings page |

The domain module is the important structural decision. All the rate maths,
clamping, quantisation and floor logic belongs in pure functions with no
database access, because that is the only way to test the asymmetry property
directly - "one loss event costs more than seven gain events" is a property of
a function, and it should be asserted as one.

## 6. Migration and rollout

Four milestones, each independently shippable:

- **M57-01** - `crates/domain/src/trust.rs` plus its unit tests. No schema, no
  wiring. Proves the arithmetic.
- **M57-02** - migration and `governance.rs` persistence, both dialects.
- **M57-03** - event wiring in sanctions and appeals.
- **M57-04** - taste factor and promotion floors, admin surface, reader surface,
  E2E.

`enabled = false` by default, with the computed score maintained in shadow mode
so an operator can watch what it *would* do before it does anything. The
existing `trust_levels` remains authoritative until the flag is on. This mirrors
how the recommendation strategies shipped and is the least surprising rollout.

---

## 7. Open questions for you

1. **Does taste similarity belong in trust at all?** Spec 16.16.2 says
   "taste alignment never affects governance or trust", stated twice, and
   19.1 requires trust earned through *reviewed conduct*. Taste alignment is not
   conduct. My recommendation: keep the taste *multiplier* and drop the taste
   *floor* - a faster earn rate for aligned users is a nudge, a promotion gate
   is a governance decision. But this is your call and the spec would need
   amending either way.
2. **Asymmetric by default, or only when configured?** A 7.5x ratio is a strong
   default for a community that has not asked for it.
3. **Should a gain event be possible during an active sanction?** Proposed no,
   but you may want yes for long-running sanctions.
4. **What is the ceiling?** Proposed TL6 is unchanged, but the regain rule makes
   TL6 sticky in a way it has never been before.
5. **Do you want per-level gain rates**, or one rate with per-level floors?
   The latter is simpler and probably right.
6. **Does the regain rule apply to the score or only the label?** Proposed only
   the label - a score is not a promise.

## 8. What I did not do

No code, no spec edit, no CSV rows, no migration. This document is the whole
deliverable, per your instruction to review before anything is modified.
