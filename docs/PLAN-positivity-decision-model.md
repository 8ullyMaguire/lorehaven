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
by `effective_...` at line 86) means the downstream consequence of a class is already policy-gated.
`delivery_outcome` (lines 130-142) is the whole policy in one match:

```rust
FeedbackClass::Positive      => Delivered,
FeedbackClass::Constructive  if prefs.accept_constructive => Delivered,
FeedbackClass::Constructive  => Held,
FeedbackClass::Ambiguous     if prefs.ambiguous_auto      => Delivered,
FeedbackClass::Ambiguous     => Held,
FeedbackClass::Negative      => Held,
```

### `ambiguous_auto` — the question this plan opened with, now answered

**It is exactly the abstention path, and it was designed as one.** Line 139 is the whole mechanism:
`FeedbackClass::Ambiguous if prefs.ambiguous_auto => DeliveryOutcome::Delivered`. With the flag off
(the `Default`, line 71) `Ambiguous` is **Held** for moderator review; with it on, `Ambiguous`
delivers.

`describe_policy` (line 160) renders it to the user in plain words —
*"ambiguous feedback delivered"* vs *"held for review"* — so it is a visible, per-work,
per-account preference rather than an internal constant.

**Why this materially strengthens the case for a decision model here.** The policy is built around
one class being *the uncertain one*, and it is the only class whose handling is a user-facing
toggle. Today `Ambiguous` is produced by a keyword rule that returns confidence `3000` (0.30) for
empty input and `4500` for "nothing matched" — that is, **the current classifier reports "ambiguous"
mostly as a failure to decide anything.** A decision model is distinguished precisely by honest
abstention with a calibrated probability, which is what this branch wants. The model would not be
adding a capability the code lacks; it would be producing a *real* `Ambiguous` instead of a
placeholder one.

Also worth noting: `sender_receipt` (line 146) returns only `"Comment posted."` or
`"Comment held for moderator review."` — spec 12.4's *"never the class or reason"*. So a wrong
classification leaks nothing to the sender; the blast radius of an error is a held comment, which
is the recoverable failure.

## 4. Suggested order, if this is ever picked up

1. ~~Read what `ambiguous_auto` was meant to do.~~ **Done — see §3.** It is the delivery toggle
   for `Ambiguous`, it is user-facing via `describe_policy`, and the default is `false` (held).
2. **Find the missing call site.** The module is complete and unwired; the actual question is why.
   Nothing was ever classified, so there is no class distribution and no way to know whether the
   markers are adequate. Establishing that requires real review text, not more code reading.
3. Wire the existing `classify` first, behind whatever gate the spec intended. Measure the class
   distribution. A large `Ambiguous` share would confirm the "placeholder abstention" reading in §3
   and is itself the finding that justifies step 4.
4. Only then evaluate a decision model as a *drop-in producer* for `Classification` — behind a
   config flag, dual-run against the markers, with the `confidence_bp` mapping pinned by a fixture
   test.
5. Keep `signals`. It is the one part of the current output that is real evidence rather than a
   constant, and it is what makes a dual-run disagreement diagnosable.

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
