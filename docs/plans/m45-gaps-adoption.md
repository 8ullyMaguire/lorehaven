# M45 gaps adoption — specifying and building the 40 remaining `planned` rows

Status: **the live plan for the last 40 rows of `docs/requirements.csv`.** Every
one of them is `planned`, and all 40 are M45. Written 2026-10-01 by auditing
`docs/requirements.csv` against `docs/spec.md` and the tree.

Supersedes nothing. `m45-ranking-substrate.md` covers the five rows already
built; this covers the other forty. `docs/spec-gaps-design-review-2026-09-22.md`
is the *source* of these rows and is explicitly **"review draft — proposed, not
adopted"**; this plan is what adopts them.

## The finding that shapes this plan

**36 of the 40 planned rows have no spec section at all.** `concierge`,
`north-star`, `keystone`, `canon-agnostic`, `sensitivity reader`, `Fic Finder`,
`dead-man's switch`, `curator lens`, `sharer code`, `governance mode`,
`view-as-persona`, `minimum activity`, `degradation tier`, `scheduled gravity`,
`notice-and-action`, `Statement of Reasons`, `DSAR`, `GDPR`, `DSA` — all zero
hits in `docs/spec.md`. Only A1 (scout value), A2, A4, A6 and F5 have become
spec text, and those are exactly the five already built.

They exist as one-line CSV rows whose `notes` column says `Gaps review A3` /
`B1` / `G2` and nothing more. **So the first task is not code — it is
specification.** `goal.md`'s standing rule is spec + plan before any code, and
here the spec genuinely does not exist yet.

A 0 grep is not evidence of absence for *code*, but it is decisive evidence of
absence for *spec*: `docs/spec.md` is the authority for what the platform must
do, and these 40 rows have no clause an implementer could work from.

The precedent is exact. `8c668e3 spec(47)` wrote §47 from gaps items A1/A2/A4/A6/F5
and *then* `56586fc` added migration 0098, `f4cbbda` implemented `rank_works`,
`5f4a7d0` wrote the suite. Spec first, then migration, then code, then tests.
Every phase below follows that order.

## What already exists to extend (do not reinvent)

| Machinery | Lives in | Serves rows |
|---|---|---|
| `TagWeights` + `rank_works` | `crates/db/src/ranking.rs` | 14, 16, 19, 30, 31, 45 |
| Taste settings, history, rollback | `crates/db/src/instance_taste_settings.rs` | 45, 46, 50 |
| Credit ledger, bounties, escrow | `crates/db/src/economy.rs` | 18, 33, 34, 41 |
| DMCA workflow, modlog, shadow sanctions | `governance.rs`, `§19.11` | 51, 52, 55 |
| `recommendation_slots` + reasons | `crates/db/src/recommendation_slots.rs` | 23, 24, 35 |
| ActivityPub / federation | `crates/app/src/routes/federation.rs` | 39, 44 |
| Source credentials (M53) | `crates/db/src/` | 20, 42 |

Kudos already has 19 files of machinery; M45-24 is the reason column, not the
feature.

## Grouping, by dependency — why this order and not by ID

The rows sort into five groups. The order is dependency order, and breaking it
costs real work, so the reasoning is given rather than asserted.

### Group 1 — Taste signal (M45-14, -16, -19, -20, -24, -31, -35)

**First, because every later group calibrates against the reader's taste
profile, and a denser profile is what makes the rest of the machine work.** Group
2's keystone detection and Group 4's north-star attribution both consume the
weights these rows write.

Breaking the order: M45-19 (tasting menu) needs tag weights to exist as a target
for its samples, and M45-20 (history import) is the highest-yield source of
labeled examples that M45-19's active learning consumes. M45-16 (only confirmed
tags count) gates all of them — an unfiltered tag-gravity signal teaches the
ranker from tag-stuffed works, and every weight written after that is worse than
the one before. M45-24 (reason-tagged kudos) is here rather than in Group 3
because its reason tags *are* the signal, not a social feature.

M45-14 (stylometry) and M45-35 (rec blurbs) are here because both produce
dimensions the ranker scores against.

### Group 2 — Supply (M45-25, -26, -28, -29, -33, -34, -36, -37, -42)

**Second, because supply mechanisms need a calibrated taste profile to decide
who gets routed to what.** A welcome rota that routes readers to
under-commented works needs to know which works the reader would want; so does
beta matching and so does cross-language selection.

Breaking the order: M45-25 (keystone authors) is defined as "the handful of
authors whose work the operator consistently loves" — which is a taste query, so
it cannot be answered before Group 1. M45-26 (feedback-drought) is C2's
operational form of C1 and shares its detection pass.

### Group 3 — Community growth (M45-22, -27, -32, -41, -43, -44)

**Third, because these are distribution: they move readers in from outside and
are measured by the attribution in Group 4.** They need Group 1 to know what to
surface and Group 2 to have supply worth surfacing.

Breaking the order: M45-41 (rec-post attribution) must be instrumented *before*
it ships or its own credits are unattributable — the same retrofit problem §47.3
records for selection propensities ("cheap now and impossible to retrofit").

### Group 4 — Measurement and governance (M45-17, -18, -23, -45, -46, -47, -48, -50)

**Fourth, because these are operator-facing controls over the machine the
earlier groups build.** M45-47 (scale-aware activation) gates the whole feature
set on instance size; M45-46 (preview) and M45-45 (versioned taste) make the
many knobs safe; M45-48 (hardware tiers) and M45-50 (scheduled gravity) are
instance config.

Breaking the order: M45-47 declares minimum activity per mechanism, so it needs
the mechanisms to exist before it can declare their thresholds. M45-23 (north
star) needs Groups 1–3 to have run, or it attributes to nothing.

### Group 5 — Legal (M45-12, -51, -52, -53, -54, -55)

**Fifth, and deliberately last in code.** This is not deprioritisation — the
gaps review itself puts GDPR/DSA (G1, G2) and the import-visibility default (G3)
in the "before launch" set. The reason to sequence them here rather than skip
them is dependency, not importance: **M45-53 (import visibility) and M45-52
(notice-and-action) change behaviour the earlier groups build**, so landing them
late means one correction pass rather than rework of the import pipeline.

M45-51's disclosure requirement has a hard constraint that makes it first in its
group: §47's `TagWeights` is inferred personal data under GDPR, so the access
request must be able to produce it. That is a read-only addition to an existing
table and can land any time; the erasure cascade cannot be built until Groups
1–3 have written every table it has to cascade through.

## Phase order, with exit conditions

Each phase is spec → migration (both dialects) → code → tests → gate. No phase
starts before the previous one is committed with a green two-backend gate.

| Phase | Rows | Gate |
|---|---|---|
| 1 | M45-16, -19, -20 | ranking taste signal is filterable and self-improving |
| 2 | M45-24, -14, -35, -31 | reasons and dimensions reach the served row |
| 3 | M45-25, -26, -28, -29 | supply routing works from real taste |
| 4 | M45-33, -34, -36, -37, -42, -30, -27, -32 | the remaining supply rows |
| 5 | M45-22, -41, -43, -44 | distribution, attributed from ship |
| 6 | M45-17, -18, -23, -45, -46, -47, -48, -50 | operator controls |
| 7 | M45-12, -53, -54 | policy knobs and the import default |
| 8 | M45-51, -52, -55 | DSAR export, notice-and-action, SOR |

**Phase 1 is next.** Its spec section is §49, and `docs/plans/m45-taste-signal.md`
carries the per-step build.

## Standing decisions made here

Recorded because `goal.md` asks for genuinely open decisions to be written down
rather than left implicit.

1. **Spec before code, for all 40.** Not because it is the house style but
   because there is nothing to implement from. A row whose only text is
   `Gaps review B1` cannot be built without deciding what it means, and
   deciding that in the code rather than the spec makes the decision invisible.
2. **Groups over ID order.** ID order interleaves a taste change with a legal
   change and a Discord bot, which is exactly the shape that produces a
   half-integrated instance.
3. **Legal last, not skipped.** See Group 5.
4. **M45-12 (generated-content posture) sits in Phase 7 with the policy knobs**,
   not with Group 2 where its incentive effect lives: it is an instance policy
   (`forbid|disclose|allow`), and §47.4's "never sanction on detector output
   alone" means the value must be an operator setting, not a computed one.
5. **No new top-level spec section per row.** One section per phase (§49 … §56),
   with the rows inside it. Fifty-eight one-line sections would be unreadable and
   fifty-eight cross-references to maintain.

## What this plan does not decide

- **Counsel.** G1–G6 are marked "verify with counsel" in the source review. The
  build implements what the spec says; it does not give legal advice, and Phase 8
  rows carry that caveat in the spec text itself.
- **Cross-instance credits (A12).** Not in the CSV as a row, and it needs two
  instances to be meaningful. Left out deliberately — the goal's "40 planned"
  figure is the authority, and A12 is not one of the 40.
- **"Launch narrow" (E1).** An operational decision, not a build item.