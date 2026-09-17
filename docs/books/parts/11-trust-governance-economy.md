# Part 11 — Trust, governance, credits and fair queues

Checkpoint: `v0.13-community`

Moderation is a power. This part builds it with the two properties that make
power survivable: **every action is visible to the person it was taken against**,
and **every action has a way to be questioned**.

## 1. Checkpoint

```bash
git checkout v0.13-community
```

## 2. What will work by the end

```bash
curl -X POST localhost:8080/api/v1/reports -d '{"subject":"…","reason":"harassment"}'
curl localhost:8080/api/v1/reports/$REPORT             # state, quorum, votes
curl -X POST localhost:8080/api/v1/sanctions/$S/appeal -d '{"reason":"…"}'

curl localhost:8080/api/v1/me/credits                  # balance and the ledger behind it
curl -X POST localhost:8080/api/v1/bounties -d '{"work":"…","amount":500}'
```

## 3. Concepts

- **A report is a case, not a flag.** It has a state, a quorum, votes, and an
  outcome that the reporter and the reported both receive.
- **Quorum, not a single judge.** Decisions made by a small panel with a written
  threshold are reproducible and contestable; one moderator's mood is neither.
- **A sanction names what it forbids, for how long, and why.** "Banned" is not a
  sanction; "cannot post comments for 7 days, because of these two reports" is.
- **Appeals are a first-class path**, with their own state machine, and they can
  succeed.
- **Credits are a ledger, never a mutable balance.** Every number is the sum of
  entries, and every entry has a reason.
- **A fair queue spends a budget, not a position.** "Your bounties get a share of
  the instance's attention proportional to your contribution, and no single
  contributor can starve the rest."

## 4. Commands

```bash
lorehaven migrate        # applies 0016_governance and 0017_economy
cargo test -p lorehaven-app --test milestone_14
cargo test -p lorehaven-app --test milestone_15
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0016_governance.sql` | reports, cases, votes, sanctions, appeals, process feedback |
| `migrations/sqlite/0017_economy.sql` | credit ledger, bounties, fair queues, billing |
| `crates/domain/src/governance.rs` | the case state machine, quorum maths, sanction terms |
| `crates/domain/src/economy.rs` | ledger rules, bounty escrow |
| `crates/domain/src/charging.rs` | what an action costs |
| `crates/domain/src/fairqueue.rs` | the fair-share ordering |
| `crates/db/src/governance.rs`, `db/economy.rs` | storage |
| `crates/app/src/routes/governance.rs`, `routes/economy.rs` | the doors |
| `crates/app/src/routes/admin.rs` | the operator's audited tools |

## 6. The code that matters

### The case state machine

```text
reported → triaged → under_review → decided → appealed → closed
                           │                      │
                           └────── dismissed ─────┘
```

Write this as one enum with one `transition` function, and make every write go
through it. State machines implemented as ad-hoc `UPDATE … SET state=…` calls in
four handlers are how a case ends up `decided` with no votes.

Every transition writes:

```text
actor (which account, which pseud), at, from, to, and the reason for the change
```

That audit row is what makes the next section possible.

### Quorum arithmetic, in the domain crate

```rust
// crates/domain/src/governance.rs — pure, unit-tested arithmetic
pub fn decide(votes: &[Vote], threshold: Threshold) -> Outcome
```

Put the arithmetic where it can be tested without a database: how many votes
count, what happens on a tie, what happens when a voter is the reported party or
the reporter (they may not vote — enforce it in the domain, not the handler),
what happens when the review window expires with too few votes (the case closes
`dismissed`, not "open forever").

### Sanctions that expire correctly

```sql
sanctions (id, subject_pseud_id, kind, scope, reason, case_id,
           starts_at, expires_at, lifted_at, lifted_by, lifted_reason)
```

Two rules learned the hard way in every moderation system:

- **Check the expiry on read**, not only with a sweep. A sweep that has not run
  yet must not keep someone silenced.
- **A lifted sanction stays in the table.** "This was applied and removed" is
  information the sanctioned person is entitled to see, and its absence makes
  appeals unanswerable.

### The credit ledger

```text
credit_entries (id, account_id, delta, reason, subject, at, idempotency_key)
balance = SUM(delta)   -- never a stored column
```

The idempotency key is what stops a retried payment webhook from doubling a
balance. And the balance is a sum, always: a stored `balance` column is a number
that will disagree with the ledger the first time a transaction half-fails.

Bounties escrow: the amount leaves the poster's balance when the bounty is
created, and is paid to the claimer when the work is accepted — or returned if
the bounty expires. Both legs are entries, and the escrow is a distinct account,
not a special case inside the code.

### Fair queues: a share, not a rank

```rust
// crates/domain/src/fairqueue.rs
// ordering by (contribution_share, last_served_at, submitted_at)
```

The property to test for is the one that matters: **one contributor submitting a
hundred jobs does not delay another contributor's first job indefinitely.** Write
a test that submits one hundred jobs from A and one from B, and asserts B's job
is served within a bounded number of turns.

## 7. Tests

`milestone_14.rs` (governance):

- a case cannot be decided without quorum;
- the reporter and the reported may not vote;
- a tie resolves to the documented outcome (whatever you chose — assert it);
- a sanction that has expired is not enforced even before the sweep runs;
- an appeal can overturn a decision, and the audit trail shows both;
- a report from a blocked account is refused without telling the reporter why;
- every governance action is visible to its subject: `GET /sanctions/me` shows
  the reason and the evidence summary.

`milestone_15.rs` (economy):

- a credit balance always equals the sum of its entries;
- a repeated payment callback with the same idempotency key credits once;
- an expired bounty returns the escrow exactly once;
- a fair queue gives a new contributor a turn within a bounded number of jobs.

## 8. Expected UI behaviour

- Reporting asks for a reason and shows what happens next, and who will see it.
- A case has a visible state and history.
- A sanction appears in the sanctioned account's own view with its reason and
  expiry, and a button to appeal.
- Credits show a ledger, not a mystery number.
- No operator tool lets anyone move credits or lift a sanction without leaving an
  audit row.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Balance disagrees with the ledger | a stored balance column | compute from entries; store nothing |
| Cases stuck `under_review` | no expiry path | a maintenance step that closes expired windows as `dismissed` |
| A lifted sanction still blocks | the check reads a cached flag | read the row, check `expires_at` and `lifted_at` together |
| Bounty paid twice | two acceptance paths | one transition, with an idempotency key |
| A moderator's action is invisible | the audit row was written only on some paths | write it in the single transition function |

## 10. Consequences

- **Moderation data is the most legally sensitive data you hold.** It names
  people, their alleged behaviour, and who judged them. Retention, access
  control and safe deletion all need deciding here — not after the first request
  for it.
- **A sanction is a promise of proportionality.** If the instance cannot explain
  a sanction to its subject, it should not apply it.
- **Credits with real money attached become a payment system** — with the
  accounting, tax and reporting obligations that follow. Keep the ledger clean
  enough that an accountant can read it, and separate "credits" from "purchases"
  in the same way you separated account from pseud.

## 11. Checkpoint

```bash
git tag v0.15-governance
```

Verified by `milestone_14.rs` and `milestone_15.rs` on both dialects, plus a walk
through a full case by hand with three accounts: report, triage, vote, decide,
sanction, appeal, overturn.
