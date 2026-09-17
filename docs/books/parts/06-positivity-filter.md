# Part 6 — The positivity filter and feedback delivery

Checkpoint: `v0.06-jobs`

This is the part that makes the site different from every other comment section,
and it is the part most likely to be built wrong. Read the rule twice before you
write any code:

> The filter exists to protect the writer's experience of their own work. It does
> not exist to punish the reader, and it must never lie to either of them.

## 1. Checkpoint

```bash
git checkout v0.06-jobs
```

## 2. What will work by the end

```bash
curl -X PUT localhost:8080/api/v1/works/$WORK/feedback-preferences \
  -d '{"mode":"positive_only","allow_list":["trusted-reader"]}'

curl -X POST localhost:8080/api/v1/works/$WORK/comments \
  -d '{"body":"the pacing in chapter four is astonishing"}'
# 201 { "comment": { ..., "receipt": "Comment posted." } }

curl -X POST localhost:8080/api/v1/works/$WORK/comments \
  -d '{"body":"this is garbage, stop writing"}'
# 201 { "comment": { ..., "receipt": "Comment held for moderator review." } }
```

The author sees the first, and does not see the second. The second commenter is
told their comment was held for review — which is true, and is all they are
entitled to know. Their comment is stored, not destroyed.

## 3. Concepts

- **Two outcomes, and only two.** `delivered` or `held`. A third state ("deleted
  because the filter disliked it") loses the text and with it the ability to
  review, appeal or learn.
- **The sender learns their own outcome, never the author's settings.** The
  receipt says "posted" or "held for review". It never says "this author only
  accepts positive comments" — that would let anyone probe a writer's settings by
  posting and reading the response.
- **Stored outcomes are never rewritten.** Classification rules improve; a
  comment classified last month keeps the outcome it was given, unless a human
  reviews it. Rewriting history makes the numbers unexplainable.
- **Rules first, models later.** A deterministic rule pass (insult patterns,
  hostility markers, spam shapes) runs first. An optional model pass is an
  additional input, not the definition of the feature, so an instance with no
  model still has a working filter.
- **The author's view is framed as what arrived.** No "held by the filter"
  counter, no scoreboard of how many people were silenced — the author's view
  counts what reached them.

## 4. Commands

```bash
lorehaven migrate        # applies 0010_positivity and 0015_comment_positivity
cargo test -p lorehaven-app --test milestone_7
cargo test -p lorehaven-domain positivity
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0010_positivity.sql` | classifications, author preferences, allow/deny lists |
| `migrations/sqlite/0015_comment_positivity.sql` | comment outcomes |
| `crates/domain/src/positivity.rs` | the rule pass, the outcomes, the receipts |
| `crates/db/src/positivity.rs` | classification storage and lookups |
| `crates/app/src/routes/feedback.rs` | the author's inbox, allow and deny |
| `crates/app/src/routes/comments.rs` | the comment door, with classification |
| `crates/app/src/routes/works.rs` | feedback preferences on the work |
| `crates/app/tests/milestone_7.rs` | acceptance tests |

## 6. The code that matters

### The outcome vocabulary

```rust
// crates/domain/src/positivity.rs
pub enum DeliveryOutcome { Delivered, Held }          // "delivered" | "held"

pub const fn sender_receipt(outcome: DeliveryOutcome) -> &'static str {
    match outcome {
        DeliveryOutcome::Delivered => "Comment posted.",
        DeliveryOutcome::Held      => "Comment held for moderator review.",
    }
}
```

The commenter's receipt is deliberately the same sentence for every reason a
comment was held: a hostile comment, a comment from someone on the author's deny
list, a comment on a work whose author chose "positive only". If the receipts
differed, they would be an oracle for the author's settings.

### Author preferences, and how they apply

```text
mode = open            → everything is delivered
mode = positive_only   → the rule pass decides; held if it flags the text
allow_list             → these pseuds bypass the filter
deny_list              → these pseuds are always held
```

The author's settings are read at classification time and stored on the
classification row, because the settings can change later and you must be able to
explain an old decision.

### Where classification runs

Classification runs **in the same transaction** as the comment insert. If it ran
asynchronously, there would be a window in which a hostile comment is publicly
visible — which is the entire thing the filter exists to prevent. That means the
rule pass must be fast and free of I/O. Put the model call behind the queue
(Part 5) only if you can hold the comment until the answer arrives; otherwise run
rules inline and treat the model as a re-classification of *already delivered*
text, with the outcome never rewritten except by a human.

### The author's inbox

`GET /api/v1/feedback/inbox` returns what arrived, per work: delivered comments,
and reviews. Two rules for this endpoint:

- it is the author's own pseud, so the door is `RequirePseud` and every row is
  filtered to works owned by that pseud;
- the payload contains no count of held items, and no per-reader identity beyond
  what the comment itself carries.

### What a moderator sees, and what the author does not

Held comments are visible to moderators (Part 11), with the classification reason
and the text intact. That is the mechanism that makes the filter reviewable: a
false negative is visible to a human, and the comment can be delivered manually —
which is a state change a human makes, with an audit row, never something the
rules do retroactively.

## 7. Tests

`milestone_7.rs` asserts:

- a hostile comment is held, and the author's inbox does not contain it;
- the held comment still exists, and is visible to a moderator;
- the receipt for a held comment is identical whether the cause was the text, the
  author's mode, or the deny list;
- a pseud on the allow list bypasses the rule pass;
- an author's dashboard counts delivered feedback only, and contains no string
  resembling a held-item count;
- the same comment body posted twice produces two classifications, and the stored
  outcome of the first is not rewritten by the second;
- an instance with no model configured still holds every text the rules flag.

## 8. Expected UI behaviour

- A held comment appears to its author exactly once, in their own view, labelled
  "held for review".
- The author's feedback inbox shows what arrived, framed as feedback.
- No screen anywhere shows the author a count of comments that were withheld.
- No screen shows a commenter anything about the author's filter settings.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Hostile comment briefly public | classification ran after the insert committed | classify inside the transaction |
| A commenter can detect the author's mode by probing | the receipt varies by cause | one receipt for every held case |
| The author's counts include held items | the query joins classifications without filtering `outcome` | filter on `outcome = 'delivered'` |
| Re-classifying changes old rows | a backfill rewrote outcomes | store the outcome once; changes are human actions with an audit row |
| The filter holds everything | the rule pass is too broad and nobody noticed | assert a corpus of ordinary praise still delivers |

## 10. Consequences

- **You are keeping a record of what people said that the author did not see.**
  That record is legitimate — it is the review queue — but it is also sensitive:
  it holds hostile text, and its existence must be documented in your privacy
  page, with a retention period.
- **The author's protection must not become a moderation blind spot.** Held text
  has to reach a human who can act, or the filter simply hides abuse from the
  people who could stop it.
- **A held comment is not a banned reader.** Nothing in this part sanctions
  anyone; escalation belongs to Part 11, where it is auditable.

## 11. Checkpoint

```bash
git tag v0.08-positivity
```

Verified by `milestone_7.rs` plus the domain rule tests, and by hand: post a
praise comment and a hostile one from two accounts, and confirm what each party
sees.
