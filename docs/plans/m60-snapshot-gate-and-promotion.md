# Plan — M60: close the snapshot-policy gate's hole, then promote the 7 `built` rows

**Status:** DONE. All seven rows promoted to `implemented-locally-tested`, each with
the run that settles it. Two defects were found and fixed on the way: the PII gate
was silent on the most sensitive column in the schema, and — the significant one —
`take-snapshot.sh` published the **entire unmasked instance** beside the masked
copy, with every gate in the repository green on it.

**The lesson this plan was written to avoid, and did not avoid well enough.** The
plan said "Step 2 before step 1 is wrong and would hide the point", and it was
right about the gate — but it assumed the seven rows were seven verification tasks.
Running M60-09's verifier instead found a critical disclosure defect in the
pipeline that produces the artefact. A `built` row is not bookkeeping; it is
untested machinery, and the one that publishes data to other people is the one
that was broken.

See `docs/verification.md` (top entry) for the full account.

**Goal:** M60's seven `built` rows are code that has never been verified against
the spec's own standard. Promoting them is the cheapest remaining work, and doing
it honestly has already produced a gate failure.

---

## What "built" means here, and why it is not "done"

`docs/goal.md:180` is the rule this plan is written against:

> A requirement is done when it has evidence, not when a status flips. A row whose
> machinery is unreachable — nothing writes the table, nothing enqueues the job —
> is not tested by unit tests on the dead functions.

So `built` means the machinery exists and the row has never been through that
standard. Four of the seven can be promoted by *running the verifier the row itself
names*. The other three cannot, and one of those is a real defect.

## Step 1 — the PII gate is RED, and three of the four columns are new

```
$ python3 scripts/check-snapshot-pii.py
FAIL: 4 column(s) with no snapshot policy decision.
  undecided: reader_body_copies.account_id
  undecided: retention_body_requests.account_id
  undecided: retention_proposal_votes.account_id
  undecided: works.redistribution

$ python3 scripts/check-snapshot-pii.py --self-test
SELF-TEST FAILED -- the gate cannot be trusted:
  - the working tree does not pass its own gate
```

This is the gate working. It is `M60-01`'s own enforcement, and it caught
`reader_body_copies.account_id` and `retention_body_requests.account_id` — the
two tables **M59-10 added three hours ago**, in the previous session. That is the
gate doing precisely the job `M60-01`'s note claims for it.

`retention_proposal_votes.account_id` and `works.redistribution` are older and were
missed because the gate only required a decision for columns in `COVERED_TABLES`
or columns whose *name* says identity — and `redistribution` says neither.

### 1a. The treatments, decided rather than defaulted

| Column | Treatment | Why |
|---|---|---|
| `reader_body_copies.account_id` | `rekey_account` | A UUID FK to `accounts.id`. The copy is a reader's *own* artefact, so re-keying on the account salt preserves the join and destroys the link to the real account. Same treatment and the same `why` as the other 63 `account_id` columns, which is what makes it auditable. |
| `retention_body_requests.account_id` | `rekey_account` | Same, and the point of the row is the reader who asked — so the column must survive as a join while the identity behind it does not. |
| `retention_proposal_votes.account_id` | `rekey_account` | A ballot is already unstackable per account; re-keying keeps it so without naming the voter. |
| `works.redistribution` | `drop_column` | See below. |

**`works.redistribution` is the one that needs a decision, and `keep` is wrong.**
It is `M45-14`'s "redistribution assertion" — a per-work flag recording that an
author has asserted their work may be redistributed. A snapshot carries it
verbatim under `keep`, and a recipient reading it learns which works are
asserted-redistributable and which are not, which is a fact about an author's
legal position that the author asserted for *this* instance's audience.

`drop_column`, not `keep_gated`, and the reason is the same one `works.body_audience`
already carries in the policy: **the absence is the only honest value.** A
`keep_gated` copy still tells a recipient this work is asserted-restricted; a
redacted constant tells them the same thing less precisely. Dropping it removes
the fact entirely, which is what §11.16 wants — the snapshot is a research corpus,
not a record of who asserted what.

**Recorded as a decision, and notified rather than assumed:** I am dropping
`works.redistribution` from published snapshots. It is reversible (revert the
policy entry), it affects only snapshot output, and the alternative — publishing
an author's redistribution assertion — is the kind of quiet disclosure the whole
subsystem exists to prevent.

### 1b. Fix the gate's own hole, which is the larger finding

`reader_body_copies` has `plain_text` and `sanitized_html` — a reader's copy of an
external body, the highest-sensitivity text in the schema — and **the gate did not
ask about either.** It asked about `account_id` on the same table and stayed silent
about the body.

The reason is structural, and `M60-05`'s own policy note already names it:

> the M60-01 gate only required a decision for columns in TABLES it knew carried
> PII, and chapter_revisions was not one of them. A gate that cannot fail on the
> most sensitive column in the schema is not a gate.

`chapter_revisions.plain_text` is gated today only because a human hand-added an
entry. **Add a new table with body text and no hand-added entry and the gate is
silently silent on it.** That is the same failure the note describes, recurring
one table over, and it is the reason `M60-05` is marked `built` rather than
promoted: the fix was applied once, to one table, and the class was not fixed.

**The fix: a `CONTENT_NAME_RE` fan-out beside the existing `IDENTITY_NAME_RE`.**
Any column named `plain_text`, `sanitized_html`, `document_json`, `body_text`,
`raw_text`, `excerpt` or `content` is gated on **any** table, for the same reason
the identity regex is unanchored: real names are qualified, and a gate that
recognises only the bare form recognises the form nobody writes.

Unanchored, so it has the same false-positive cost the identity regex already
carries and accepts — a question costs a line in a JSON file, a missed column
costs a disclosure. That trade is already made once in this file and it is the
right way round.

**Then: prove the new fan-out fires**, by adding a fourth self-test probe — a
`plain_text` column on a table that is neither covered nor identity-bearing — and
requiring `check` to flag it. The self-test currently asserts three properties;
this adds the fourth, and it is the one that would have caught the hole.

### Verify

```sh
python3 scripts/check-snapshot-pii.py            # expect exit 0, no undecided
python3 scripts/check-snapshot-pii.py --self-test # expect exit 0, 4 properties
```

## Step 2 — promote the rows the verifiers can settle

Each promotion runs the verifier **the row itself names**, and the evidence entry
records the run, not the intention.

| Row | Verifier | Expected |
|---|---|---|
| `M60-10` | `python3 scripts/check-snapshot-channel.py --self-test` | 11 refusals, 4 controls, exit 0 |
| `M60-09` | `scripts/take-snapshot.sh` end to end on Postgres | restores into a throwaway DB, `doctor --strict` clean |
| `M60-08` | two consecutive `build-snapshot-sql.py --timestamp-offset` runs | different offsets, destinations, passphrases |
| `M60-02`, `M60-05`, `M60-06`, `M60-11` | `cargo test -p lorehaven-app --test snapshot_anonymisation` on **both** engines | 10 passed each |

`M60-02`'s row is worth a note: it reads the dump as **bytes**, not as a restored
database, because a canary in a `pg_dump` comment is invisible to a query and is
exactly the leak the requirement is about. That test already exists and has passed
on both engines — it is in the 3348. Promoting it is recording a fact, not
manufacturing one.

## Step 3 — the two rows that cannot be promoted by running something

`M60-09` is only promoted if the end-to-end run is done **here**, not inferred from
the script existing. If Postgres is unavailable, the row stays `built` and the
reason is recorded — a row promoted on the strength of a script's existence is the
`be68f32` defect in a different costume.

`M60-08` needs **two** consecutive snapshots to mean anything, and each takes a
restore. If only one can be taken, the row stays `built`.

## Step 4 — the ledger and the log

- `docs/requirements.csv`: the 7 rows, each with the run that settles it.
- `docs/verification.md`: a newest-first section, both engines, and the gate's
  own failure recorded as the headline — because **a gate that found four
  undecided columns three hours after the code that created them is the best
  evidence in this file that it works.**
- `docs/known-gaps.md`: nothing to add. A gate that fails loudly is not a gap.

## Order

1. The four policy entries (1a) — the tree goes green.
2. The gate's content fan-out + fourth self-test probe (1b) — the tree stays green
   *and* the hole closes.
3. Run the four verifiers (step 2), recording each.
4. Promote what passed. Leave the rest `built` with a reason.

**Step 2 before step 1 is wrong** and would hide the point: promoting rows while
the gate that governs them is red would record seven successes on a tree with a
known disclosure path. The gate goes green first, or not at all.
