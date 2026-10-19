# ADR 0028: The requirement ledger is re-triaged in place, not restarted

- Status: accepted
- Date: 2026-10-19

## Context

`docs/requirements.csv` is the row-by-row authority for what is built. It carries **703 rows**
in 48 areas, and it is the estate's clearest working practice: evidence, not status; a status
document carries the command that re-derives it; a requirement is done when it has code that
runs, tests that drive the real path, and evidence recorded.

The Darkhold conversion changes what the project *is*, and it therefore changes what most of
the rows *mean*. The vocabulary moves (§5 of the specification: `work`, `source`, `source
record`, `work identity`, `unit`, `revision`, `creator`, `canonical entity`, `tag assertion`,
`theme`, `channel`, `instance theme`, `mood`, `signal`, `taste signal`, `block`, `trust level`,
`guest`, `supporter tier`, `catalogue`). A row that says "lorehaven's `chapters` table" is a
row about a *unit* now, and a row that says "the operator's private taste" is a row about a
*published instance theme* now — which is the opposite of private.

There are two obvious ways to do this. Restart the ledger from the Darkhold specification, or
re-triage the existing one.

## Decision

1. **The ledger is re-triaged in place.** Every one of the 703 rows is mapped into the
   Darkhold vocabulary, the duplicates are merged, and the statuses are re-derived against
   Darkhold's definitions. The result is the roadmap, and **it is measured, not estimated**.
2. **The rows that are decisions rather than requirements are carried across as `unsupported`.**
   The specification's sixteen non-goals are refusals, not backlog, and `unsupported` is the
   status that means *"consciously declined — a standing exclusion, never counted as remaining
   work"*. This is the mechanism by which the refusals are **enforced** rather than merely
   stated: a row cannot be silently reclassified as `planned` without the diff being visible,
   and the diff is the audit trail.
3. **The two columns the Darkhold status vocabulary adds — `verified_at` and `gates` — are
   specified and deferred.** `verified_at` is the only authority on whether a row's evidence is
   current, and `gates` names the gate commands that were run for the row. A status document
   that cannot be re-derived rots faster than the code it describes.

   **They arrive with a deliberate migration that populates them**, and not before, because
   populating 703 inherited rows means knowing each one's gate command — and **guessing would
   be fabricating the evidence the column exists to record.** Until that migration, the ledger
   has **seven** columns, and the writer **preserves the file's header verbatim**.

   > **This is the correction M0 made to itself.** The first version of `scripts/ledger.py`
   > *appended* the canonical columns the file was missing, so editing one row grew all 703
   > inherited seven-column rows to nine — two empty fields on every one — and the diff was
   > **704 lines removed and 711 added**. The module's own header said *"a ledger that has not
   > yet gained a column does not gain one because somebody edited a row,"* and its
   > implementation did exactly that. **And the test that should have caught it asserted the
   > opposite of its own name** — it was called `test_header_is_preserved`, its docstring said
   > *"a ledger with seven columns must not silently grow to nine,"* and its assertion required
   > the header to be exactly `…,verified_at,gates`.
   >
   > A test whose assertion contradicts its own name is worse than no test, because it buys the
   > confidence of coverage while checking nothing. Both are fixed.

4. **The ledger's line ending is CRLF and stays CRLF.** This is not a detail. The repository's
   own `scripts/add-m45-57-row.py` carries the warning: *"That file is CRLF. Rewriting it with
   Python's `csv` module converts every line ending and turns a one-line edit into 698 — a trap
   this repo has hit before."* `scripts/ledger.py` is now the only writer, it reads with
   `newline=''` and writes with `lineterminator='\r\n'`, it preserves the existing header
   **verbatim**, and its upsert is idempotent.
5. **A row is not "done" because it says so.** `implemented-fully-tested` means the code runs,
   the tests drive the real path and the evidence is recorded. `implemented-locally-tested`
   means it is built and unit-tested but **not held to the full-stack bar** — and the
   specification records that **360 of lorehaven's 703 rows sat there**. That is the number this
   milestone exists to reduce, and it is reduced by *re-deriving* the rows, not by editing
   them.

## Consequences

- The ledger's row count changes, and the change is the deliverable: rows merged, rows split,
   new rows added for the Darkhold-only capabilities (the taste crate, the instance theme, the
   voted cluster weights, the moods, the author identity, the tag de-prioritisation, the
   expansion workflow), and the refusals carried as `unsupported`.
- The re-triage is a **single mechanical pass with a reviewable diff**, not a hand edit of 703
   rows. A hand edit of 703 rows is 703 chances to be wrong and no way to see the shape of the
   change.
- Every row's `evidence` names a test, because *"a plan document cannot prove that a thing
  renders."*
