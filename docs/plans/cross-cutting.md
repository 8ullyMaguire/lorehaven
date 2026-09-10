# Cross-cutting work

Things every milestone touches. They are not a milestone; they are the
conditions under which a milestone counts as finished.

---

## 1. The end-of-milestone checklist

Copy this into the milestone's entry in `docs/sessions/` when you finish.

```text
[ ] migration exists for both dialects, with deletion/retention stated
[ ] cargo fmt --all -- --check                       clean
[ ] cargo clippy --all-targets --all-features -D warnings   clean
[ ] cargo test --workspace                           all pass
[ ] frontend build, test and check                   all pass
[ ] the journey was driven by hand in a real browser
[ ] every failure the journey produced is in docs/verification.md
[ ] requirements.csv rows updated with a command, not a file path
[ ] docs/tutorial/ has the chapter (see §25 of the spec)
[ ] committed, tagged with the name reserved in docs/tutorial/README.md
```

The three checks that are most often skipped, and should not be:

* **The browser journey.** Automated tests have not yet caught any of the eight
  defects listed in `docs/sessions/2026-09-10.md` that a browser caught.
* **`clippy -D warnings`.** The workspace lints `unsafe_code = "forbid"`. A
  warning in CI is a failure; finding it locally costs seconds.
* **`requirements.csv`.** A requirement whose status still says `unsupported`
  when it is finished is a lie in the one file a reviewer reads.

## 2. Migrations

* Next free number is **0004**; increment per migration, never reuse.
* Both dialects, same id, same order of statements. PostgreSQL validates a
  foreign key when it is created, so a **circular** reference is declared
  inline on SQLite and added with `ALTER TABLE … ADD CONSTRAINT` at the end of
  the PostgreSQL file — see `0003_works.sql` for the worked example.
* A column that must be cheap to add later gets a default, so an `ALTER TABLE`
  on a large table is not a rewrite.
* Index every foreign key you look up by, and lead a composite index with the
  column you filter on first.
* State retention: what cascades, what soft-deletes, what is kept for audit,
  and what a deletion workflow must reach (including backups).

## 3. Performance and bounded resource use

Spec §1.5 makes "bounded resource use" a foundational policy, and §3.8 puts
ceilings on expensive operations. Concretely:

* No endpoint returns an unbounded collection. Cursor pagination, with a
  default page size in configuration and a hard maximum.
* Every request body has a ceiling (`RequestBodyLimitLayer`, already installed).
  A *parser* (the query language, an imported document, a manifest) has its own
  smaller ceiling and its own depth limit — `crates/domain/src/document.rs`
  shows the shape: `MAX_DEPTH`, `MAX_NODES`, `MAX_TEXT_BYTES`, each enforced by
  a test that feeds a document larger than the bound.
* Anything that can be made quadratic by input (a tag intersection, a
  reorder, a merge) gets a limit and a documented failure, not a timeout.
* Measure before claiming a budget is met. `docs/verification.md` has an entry
  for the performance profile; keep it honest.

## 4. Accessibility

Milestone 1 set the standard; later milestones must not fall below it.

* Every control reachable and operable by keyboard, with `:focus-visible`
  styling that is not removed.
* Every form control has a label that is programmatically associated. The
  field primitives do this; use them rather than raw `<input>`.
* Errors are announced (`role="alert"`) and attached to their field.
* Content that updates live (job progress, save state, streaming messages) is
  in a `role="status"` or `aria-live` region, and does not steal focus.
* Reduced motion is respected (`prefers-reduced-motion`), and the reader's
  distraction-free mode must not trap a keyboard user.
* The outstanding measurements from Milestone 1 — layout at **320 CSS pixels**
  and at **200 % zoom** — are still `implemented but not executed`. Take them
  during M18 and record the numbers.

## 5. Privacy classifications

Spec §3.7 requires every endpoint to be classified. Practically, for each new
table ask: *who may see this row, and what query would leak it?*

* Private reading data (`reading_progress`, `reading_history_entry`,
  `reader_note`, `rating` unless published) is **never** joined into a query
  another account can reach.
* Source credentials are `source-secret`: encrypted at rest, never in a job
  payload, never in a log, never in an error message.
* Aggregates are their own classification. An "aggregate-public" number must be
  computed so that it cannot be inverted to a single account — the minimum
  publication threshold on public ratings exists for this reason as well as for
  statistical honesty.
* Cache keys, index documents and notification payloads carry the same
  classification as the row they describe. A search index that holds a draft's
  text is a leak with a delay on it.

## 6. The frontend bundle

Milestone 3 split the rich-text editor into its own chunk and the entry bundle
dropped from 462 kB to 133 kB (46 kB gzip). Keep that discipline:

* A heavy dependency belongs behind `import()` and is loaded when the surface
  that needs it opens.
* The reading pages must stay small; they are what most visitors load.
* After a build, read the size table `vite build` prints and notice when the
  entry chunk jumps. A jump is either explained in the commit message or it is
  a mistake.

## 7. When the spec and the code disagree

The spec is the requirement; the code is the current truth. When they differ:

1. If the code is right and the spec is stale, change the spec in the same
   commit and say why.
2. If the spec is right and the code is a shortcut, fix the code — or record
   the deviation in `docs/verification.md` as `partially implemented`, with
   what is missing. A silent shortcut is how a project ends up claiming
   behaviour it does not have, which `docs/spec.md` §1.2 exists to prevent.
3. If the disagreement is a design decision, write an ADR
   (`docs/adr/000N-name.md`) with: problem, decision, alternatives considered,
   consequences, and the conditions that would justify revisiting it. The
   spec's list of expected ADRs (0004–0007 in §1.4) is already partly consumed
   by ADR 0004 on timestamp and identifier storage; number the new ones after
   whatever exists.
