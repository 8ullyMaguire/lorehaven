# M65-07 — coverage gate: the catalogue proves it read every brainstorm

Spec §59.7: "the catalogue must not silently lose items". The generator reads a
NAMED list of sources, and three idea documents sit outside it (`docs/spec-summary-brainstorm.md`,
`docs/known-gaps.md`, `docs/ROADMAP-guest-trust-governance.md`). A new `docs/ideas/*.md` is picked up
only because that directory is globbed. A document the list forgot contributes nothing, and nothing
reports the omission — silence reads as "no ideas there".

The gate must be **structural**: fail when a tracked markdown file whose name matches
`brainstorm|ideas|roadmap` (or lives in `docs/ideas/`) is NEWER than `docs/ideas-catalogue.json`
and contributes no rows. So silence is never mistaken for coverage.

## Order of work

1. Fix the two known generator defects first (they are coverage bugs of their own):
   - the summary doubling ("…narrowing in the gate per-user module overrides: …") — `add()`
     concatenates context onto an existing entry; the requirements pass passed the requirement text
     while the title already IS its leading clause. `add()` keeps the LONGER context, not the glue.
   - `escape()` applied into `why_summary` AND again in `write_rust()` — the JSON holds the raw text.
2. The coverage gate itself, as a function in the generator:
   - enumerate candidates: every tracked `.md` under `docs/` (git ls-files) whose basename matches
     `(brainstorm|ideas|roadmap)` case-insensitively, plus every file in `docs/ideas/`, plus the
     vault idea notes already named.
   - for each candidate, extract its idea titles with the same rules the catalogue uses
     (`###` headings for sweeps; the document itself for amendments/designs; table rows with
     >= 2 word-ish tokens for audits) and compute the slug set.
   - a candidate CONTRIBUTES if any slug it produces appears in the catalogue's idea set
     (a row the catalogue holds). A document whose every heading is a label or a duplicate of a
     row another source also holds still "contributes" through the shared row — the gate is about
     READING, not about uniqueness.
   - FAIL when a candidate is newer than the JSON output and contributes nothing, unless it is on
     the declared-ignore list with its reason (e.g. a roadmap whose entries are all milestone rows
     already counted from requirements.csv — the ignore must name the rule, not the file).
   - The gate runs inside `main()` BEFORE writing, so a red gate produces no artefacts at all.
3. Test the gate with the three named documents as the first live case: whatever they contribute
   (new rows) or are declared-ignorable (with reasons) — either way the run reports it.
4. Gates: `python3 scripts/ideas_catalogue.py` exit 0 with the per-source counts printed; drift test
   green; a mutation that empties the ignore list and a stale mtime fails the run.
5. Regenerate `ideas.rs` + `docs/ideas-catalogue.json`, run the Rust domain test
   (`every_idea_in_the_json_is_in_the_registry`), then the full gate set.
6. Commit, deploy to thinkcentre, live-prove `/api/v1/modules` carries the new counts.

## Verification

- `python3 scripts/ideas_catalogue.py` → exit 0, per-source contribution counts, retired list.
- `cargo test -p darkhold-domain --lib modules` → the catalogue test green with the new row set.
- Mutation: `touch docs/known-gaps.md` after the JSON → the run FAILS by name; then restore.
- Live: `/api/v1/modules` `counts.ideas` matches the regenerated count; `dh module ideas | head`.

## Divergences to record, not silently absorb

- A document that contributes only rows another source also holds: counted as contributing
  (reading is what the gate asserts), reported in the per-source counts so a human can see it.
- A document whose ideas are all one-word headings: contributes nothing BY RULE, and the run says
  which headings were dropped, so the loss is a decision, not silence.

## Not built (filed, with reasons)

- M65-06 (frontend nav from the payload) and M65-08 (idea→module join) are separate tickets and
  separate turns. This one ends the "silence reads as coverage" failure mode.
