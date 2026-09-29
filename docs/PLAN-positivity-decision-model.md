# PLAN (optional) — wire `positivity::classify`, or replace it with a decision model

Status: **optional, not adopted, not implemented.** No dependency added, no code changed, no model
downloaded. Records an opportunity found during the Laya decision-model survey of 2026-09-29.

Origin: `~/secondbrain/30-Resources/laya-decision-model-use-cases.md` and its per-repo ledger.
Companion: `~/code/rust/ficnexus/docs/PLAN-laya-decision-backend.md` (the same idea applied to a
classifier that *is* wired).

---

## 0. The finding

This module is complete, database-backed, and **not called by anything.**

| Symbol | Location | What it is |
|---|---|---|
| `FeedbackClass` | `crates/domain/src/positivity.rs:21` | `Positive` / `Constructive` / `Ambiguous` / `Negative` |
| `Classification` | `crates/domain/src/positivity.rs:53` | `{ class, confidence_bp: i64, signals: Vec<String> }` |
| `classify(text) -> Classification` | `crates/domain/src/positivity.rs:254` | deterministic keyword-marker rule engine |
| `record_classification` | `crates/db/src/positivity.rs:316` | upsert into `review_classifications` |
| `FeedbackPreferences` | `crates/domain/src/positivity.rs:61` | `accept_constructive`, `ambiguous_auto`, `comments_enabled` |

The schema backs it on both dialects:
`migrations/sqlite/0010_positivity.sql:57` and `migrations/postgres/0010_positivity.sql:40` both carry
`class TEXT NOT NULL CHECK (class IN ('positive','constructive','ambiguous','negative'))`.

**`record_classification` has zero callers.** A repo-wide search returns only its definition, the
`Classification` row-mapping beside it, and one reference in
`crates/app/tests/milestone_9.rs`. Nothing in the app calls it.

## 1. What `classify` actually does today

```rust
/// Classify one text. Deterministic: same input always yields same class.
pub fn classify(text: &str) -> Classification {
    let folded = text.to_lowercase();
    let trimmed = folded.trim();
    if trimmed.is_empty() {
        return Classification { class: FeedbackClass::Ambiguous, confidence_bp: 3000,
                               signals: vec!["no-signal".to_owned()] };
    }
    let hostile = contains_any(trimmed, HOSTILE_MARKERS);
    if !hostile.is_empty() { /* Negative, confidence_bp: 8500, signals "hostility-pattern" + hits */ }
    /* ... otherwise Positive/Constructive, confidence_bp: 4500 */
}
```

Three points worth being blunt about:

1. **The confidence values are constants, not measurements.** `3000` for empty input, `8500` for a
   marker hit, `4500` otherwise. `confidence_bp` is a column in a table and it carries no
   information about the text beyond which branch was taken.
2. **`signals` records which markers fired**, which is genuinely useful and worth keeping in any
   replacement.
3. **It has never been evaluated**, because it has never run. There is no accuracy figure to beat
   and no confusion matrix to reason about. Whatever replaces it, the first real measurement is the
   first measurement of anything.

## 2. The two options, and why the choice is not obviously "add a model"

### Option A — wire the existing classifier (no model at all)

Find the call site the module was written for, call `classify`, store the result. This is the
cheapest possible change and it is the honest baseline: **an unwired classifier is a bug-shaped
gap, and the fix might not be ML-shaped at all.** Anyone reaching for a model here should first
establish that the marker engine is insufficient, which cannot currently be done because it has
never seen production text.

### Option B — replace `classify` with a decision model

`Classification { class, confidence_bp, signals }` is already a decision model's output with a
different unit: `{ choice, probabilities, evidence }`. A `choice` question with `criteria` keyed to
the four classes maps onto it without changing the struct — only the producer of the confidence
changes, and `confidence_bp` stops being a guess.

The specific hazard, which is the same one flagged in the ficnexus plan: **Laya, Jev, and Kev each
compute `confidence` differently**, and the Unsloth docs say twice to threshold on
`probabilities`, not `confidence`. There is no tuned threshold here to preserve — `8500` was never
tuned — so this is the *easy* case for that hazard, but the mapping into `confidence_bp` still needs
a unit test with a fixture so a model swap cannot silently move it.

## 3. What is already safety-bounded

`FeedbackPreferences` (line 61) with per-work overrides (`WorkFeedbackOverride`, line 79, resolved
by `effective_...` at line 86) means the downstream consequence of a class is already policy-gated:
`accept_constructive` and `ambiguous_auto` decide whether a given class is acted on at all, and
`comments_enabled` can be false outright. A wrong classification is therefore bounded by existing
policy rather than reaching users directly.

**`ambiguous_auto` is the interesting one.** A decision model's most valuable property here is
honest abstention — `Ambiguous` is exactly the label a probabilistic model can return with high
honesty, and the policy flag already exists to decide what happens then. Whether that was the
intended design of `ambiguous_auto` is **not yet checked** and is the first thing to read.

## 4. Suggested order, if this is ever picked up

1. Read what `ambiguous_auto` was meant to do (spec 8.4 / 12.3 are cited in the code comments).
2. Wire the existing `classify` first. Measure the class distribution over real review text.
   A large `Ambiguous` or `Positive` share is the actual finding, and it may end the discussion.
3. Only if marker matching is demonstrably inadequate, evaluate a decision model as a *drop-in
   producer* for `Classification` — behind a config flag, dual-run against the markers.
4. Keep `signals`. It is the one part of the current output that is real evidence rather than a
   constant.

## 5. Verification

```bash
# the zero-caller claim, re-runnable
rg -n "record_classification" --type rust -g '!target/**' .
# expect: the definition, the row mapper, and one test reference — no call from app code
```

---

## Related

* `~/secondbrain/30-Resources/laya-decision-model-use-cases.md` — the cross-project survey
* `~/secondbrain/30-Resources/laya-decision-model-repo-ledger.csv` — per-repo ledger, 74/74 inspected
* `docs/PLAN-laya-decision-backend.md` — the same pattern in ficnexus, where the classifier *is* wired
* [[Kev-4B — Use Cases Across the Local Projects]] — the predecessor note this supersedes on feasibility
