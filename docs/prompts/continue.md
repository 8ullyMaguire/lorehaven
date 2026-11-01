# Continue darkhold — standing prompt (filled)

Filled from `~/secondbrain/10-Projects/continue-any-project-prompt.md`. The slots are darkhold's;
the evidence rules are the template's, kept verbatim because they are what stops a report from
being a claim.

---

Work on **darkhold**. It is an existing, deployed project: you are continuing it, not starting it.

Read, in this order, before deciding anything:

- **`docs/requirements.csv`** — the work list. Ordered by milestone id, then row id; each row names
  the area and the evidence that would close it. `status=planned` is the queue.
- **`docs/spec.md`** — the sections governing the item. The catalogue is not yet in the spec: for it,
  `docs/ideas/2026-11-01-idea-sweep-power-users.md` and the item's own plan are the authority, and
  the plan says so.
- **`docs/handoff.md`** — the last session's carry-over, including what broke and why.
- **`docs/plans/`** — the item's plan; and `docs/adr/` for every decision it cites.
- **`docs/plans/deploy-lineage-decision.md`** — read before promising any deploy.

Then load, with `skill_view`:

| skill | why |
|---|---|
| `darkhold-thinkcentre-deploy` | the deploy loop, and the traps that cost a day each |
| `reconciling-a-foreign-lineage-deployment` | the live instance is not this checkout |
| `two-engine-store-queries` | every query here runs on SQLite and PostgreSQL |
| `verifying-wired-not-just-tested` | a route that accepts a parameter and drops it passes every unit test |
| `spec-plan-first` | plan before code, verification command per step |
| `db-migration-integrity` | only if the item touches `migrations/` |

Traps that cost a day each, in this repo:

1. **Two lineages, one database.** `origin/master` and this checkout are ahead/behind in the
   thousands; the live host's tree does not contain this line's commits. Never assume `git push`
   deploys, and never rsync over the host tree.
2. **`library_items` is per-account.** A read that appears to be "the catalogue" is really the
   library rows of any account that imported it. `account_id` and `provenance_json` are the private
   side; anything exported or newly exposed must name its columns.
3. **The timestamps are one format, and the database enforces it.** Since M63-31 (migration 0132)
   `library_items` stores `YYYY-MM-DDTHH:MM:SSZ` only: PostgreSQL has a `CHECK`, SQLite has
   triggers, and a write in any other shape fails with "must be canonical". This is what makes the
   window a plain index-able range — do not reintroduce a normalising function in a predicate. The
   writer that produced the old bare dates is the external XenForo importer, which is not in this
   repo, so a fixture that writes `2026-10-13` or `2026-01-01 00:00:00` is a bug in the fixture.
4. **A clippy warning is a build error** (`build.warnings` denies). Three pre-existing warnings in
   `crates/db/src/work_ratings.rs` already fail `--all-targets`; your files must add none, and do
   not "fix" theirs inside an unrelated item.
5. **The frontend `dist` is embedded at compile time.** A backend-only change still needs the
   deploy script's frontend step if the served bundle must match; check `assets/index-<hash>`.
6. **The SPA answers 200 for every unknown path.** `/api/v1/…` is the contract; a 200 with HTML is
   the shell, not the route.

Take the work order's order. Do not re-rank by effort or by what is easiest to demo. If evidence
contradicts the order, say which evidence and what it changes, then proceed.

**One milestone row per turn.** Finish it, verify it, ship it, write it down. Do not start the next
one to fill the turn.

For each row, the order of work is fixed:

1. Plan under `docs/plans/<item>.md` — exact file paths, the verification command for every step,
   the expected output of each. No code before the plan exists.
2. Implement.
3. Tests, and watch them fail for the right reason before they pass. Then **mutate the
   implementation** so the test must notice: flip a comparator, drop a normalisation, or make the
   route accept the parameter and not pass it. Report which mutation each test caught.
4. Gates, in this order:
   - `cargo test -p darkhold-domain -p darkhold-db -p darkhold-app --lib`
   - `cargo test -p darkhold-db --test <item> ; DARKHOLD_TEST_PG_URL=postgres://lorehaven:lorehaven@127.0.0.1:55433/postgres cargo test -p darkhold-db --test <item>` (start `lh-pg-test` first; unset the var before the SQLite run — a shared DSN silently points the SQLite test at Postgres)
   - `cargo test -p darkhold-app --test <item>`
   - `cargo clippy -p <touched crates> --all-targets`
   Report the real counts from the output.
5. Deploy per `darkhold-thinkcentre-deploy`. The lineage question is closed
   (`docs/plans/deploy-lineage-decision.md`): `~/code/rust/darkhold` is the thinkcentre tree over
   the mount, and both trees are one history now — fetch `local`, reset, build from the tree on the
   host. Run `migrate` **from the tree root** or it silently migrates a fresh sqlite dev database.
6. Docs: the `requirements.csv` row's `status`, `evidence` (paths that exist) and `notes` (what
   landed, what was deferred and why); the plan's own "what landed" section if it diverged; a new
   row for any defect the item uncovered.
7. Commit; push to `origin` (`git.polarisocial.xyz`) and `github`. The two remotes are ~1046 commits
   behind this line and merging them rewrites published history, so a push is an operator decision,
   not a step: commit locally and say the push is held until they say otherwise.
8. A session note in `~/secondbrain/10-Projects/`, named for the date and the project.

Evidence rules, because they are the difference between a report and a claim:

- A 200 is not proof a surface works — read what it rendered. The darkhold SPA answers 200 with
  HTML for every unknown path.
- A green test is not proof until the mutation that should break it does.
- A compile error is not a killed mutation. Read the mutation actually applied.
- Live behaviour is checked against the running artifact (`curl 127.0.0.1:8081` on the host), not
  against your reading of the code.
- An empty result is a result. Report it; do not widen the query until something appears.
- If something cannot be verified — deploy is blocked, the host has another session's work, an
  endpoint needs a credential you do not have — say so plainly and leave it unverified.

Decisions that are yours: implementation shape, names, where a surface lives, which of two
equivalent queries to keep, test design, and what to defer — provided the deferral is written into
the plan with its reason.

Decisions that are **not** yours — stop the item, write a decision doc with two or three options
and your recommendation, and say it is waiting:

- which lineage is the deploy trunk, and when to merge it (see `deploy-lineage-decision.md`);
- any change to what a reader's data exposes (new public field, new export, changed k-anonymity);
- dropping or rewriting a migration that has already run on the live database;
- renumbering or reprioritising milestones against `docs/requirements.csv`.

Guardrails: migrations are append-only once applied anywhere; never put a pseudonym, email or
account id into a public artefact; never weaken a gate to make an item pass; never edit the host
tree directly — build from a checkout or a bundle.

End the turn with a short report: the item, what shipped, the exact command and output that proves
it, what you decided and why, what is blocked and on whom, and what you will take next. Nothing
else.

First turn: read only. Report the order you intend to take, item by item, for the next several
turns, with one line of justification each. Start the first item in that same turn only if you can
finish it; otherwise end after the report.
