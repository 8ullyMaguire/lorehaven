# M45-51 — GDPR: subject access and erasure

Spec and plan, 2026-10-05. Written BEFORE any code, per the standing rule.

Row: `docs/requirements.csv` `M45-51`, status `planned`, note "Gaps review G1. Profiling
purpose in privacy notice; private-library copy survival policy."

## 0. What was measured first, and why the measurement is the spec's first section

A spec written without reading the code is how this project produced four wrong tracker rows
and a commit that left the frontend unbuildable. So before designing anything:

**The erasure graph is measured, not assumed.** Parsing `migrations/sqlite/*.sql` for every
`CREATE TABLE` and every `REFERENCES` clause:

- **191 tables** total.
- **71 tables** whose foreign-key chain reaches `accounts`.
- **Exactly one** column into `accounts` does not cascade: `jobs.requested_by`, which is
  `ON DELETE SET NULL` — correct by design, because a job's audit trail must survive the
  requester who asked for it.

So the "erasure cascade" is **already 99% present** as schema. This item is not a migration.
Building a second parallel erasure path would be the classic mistake this project has made
before: `did_not_finish` vs `reading_history_entry` are two status tables where one richer
one already existed, and standing up a third would make the real one harder to reason about.

**The subject-access side does not exist.** `exports.rs` is an *admin* export job with
download grants. There is no route a reader calls to obtain everything held about them. The
`analytics.rs` scope vocabulary already contains `OwnResonanceLabel`, `OwnReadingBasic`,
`OwnContributionHistory` and 10 others — **a scope that nothing serves**. So the disclosure
side is a renderer over a vocabulary that is already written, not a new data model.

**The forbidden list constrains what may be disclosed, and it is data, not prose.**
`analytics.rs:876` `FORBIDDEN_SCOPE_NAMES` names 13 things that must never be disclosed —
including `ab.resonance_numeric` ("own-only, label form only"). §11 of this spec is that
constraint made load-bearing rather than documentary.

## 1. Why this item and not another

28 requirements are `planned`. They were grouped by what they need:

| group | count |
|---|---|
| render existing data | 0 |
| new query/store | 25 |
| migrations | 1 |
| external dependencies | 2 |

So none of the remaining work is cheap render work any more, and this one was chosen for
**correctness stake per unit of work**: both halves are the class where a silent bug is a real
harm rather than a cosmetic one, the erasure half is nearly free because the schema already
does it, and the disclosure half is a renderer over an existing scope list.

The two items with arguably higher stakes were rejected for stated reasons, not silently:

- **M45-53 (import visibility default)** is flagged "biggest reputational and copyright risk"
  and it is a *visibility default* — the exact class of bug this project has shipped
  repeatedly (`is_public` dropped from a query, invisible on one engine). It should be next,
  and it deserves its own spec rather than being folded in here.
- **M45-39 (instance-death protection)** needs ActivityPub and mutual backup. Two external
  protocols before any code, which is the shape that goes wrong quietly.

## 2. Design

### 2a. Erasure — `POST /api/v1/me/erasure`

`RequireSession`. An account deletes itself.

**What happens, in order, and why the order matters:**

1. **Verify intent.** The request body carries `confirm_handle`, which must equal the
   account's handle. A session cookie is a weak confirmation for an irreversible action — a
   CSRF-shaped mistake must not be able to trigger it. This is the one irreversible surface
   in the item and it gets a second factor.
2. **Refuse while works are published under the account's pseuds.** Not block: *return a
   count and require `force`.** Silently orphaning a published work is worse than a 409, and
   silently refusing is worse than telling the reader what they would orphan. This is a real
   decision, not a convenience.
3. **Cancel open export jobs first.** An export mid-flight would re-materialise data moments
   after erasure. `find_open_export` needs a subject, not an account, so this is a direct
   `UPDATE ... WHERE state IN ('queued','running')` — NOT a one-line call on the existing
   function, and worth saying because that function's signature invites the mistake.

   **Measured correction, 2026-10-05: this ordering is belt-and-braces, not load-bearing.**
   `export_jobs.account_id` is `ON DELETE CASCADE` and `download_grants.export_job_id` is
   `ON DELETE CASCADE`, so the `DELETE` takes the export row *and* its delivery grant. The only
   thing that survives is the `jobs` queue row (`requested_by` is `SET NULL`, deliberately, so a
   failure can be diagnosed after the requester is gone) — and a worker whose export row has
   vanished has nothing to assemble. The defensive `UPDATE` stays because a running worker may
   already hold a grant in memory, but no test can observe it through the database, and the
   test file says so rather than implying coverage it does not have.
4. **Delete the account row.** The 71-table cascade does the rest. `jobs.requested_by` goes
   NULL. **`api_tokens` already cascades** (`0001_identity.sql`, `ON DELETE CASCADE`), so
   there is no separate revocation step to get wrong — which is why step 7's first mutation is
   the export cancellation rather than a token revoke.

**No soft delete.** `accounts.deleted_at` exists and is documented as "accounts soft-delete
via deleted_at so that audit [survives]" — so this item is a deliberate departure from a
documented intent, and that is worth flagging rather than quietly overriding. The reason the
departure is right here and wrong for moderation: erasure is the reader exercising a legal
right, and a soft-deleted account is still personal data. The audit trail does not need the
row, because `audit_log` and `admin_actions` do not FK to `accounts` — confirmed in §0. The audit trail survives in `audit_log` and
`admin_actions`, which do not FK to `accounts` — verified above, since they were not in the
71.

### 2b. Subject access — `GET /api/v1/me/data`

`RequireSession`. Returns a JSON object keyed by the scope vocabulary.

**The disclosure set is `FORBIDDEN_SCOPE_NAMES`-filtered, and the filter is tested.** A
disclosure endpoint that accidentally emits a forbidden field is the worst bug this feature
could ship. So the test enumerates all 13 forbidden names, asserts none appears anywhere in
the response body as a key, and that is a standing test rather than a one-off.

**Resonance is a label, never a number.** `OwnResonanceLabel` exists precisely so
`ab.resonance_numeric` does not. The response carries the label and the disclosure names the
constraint in a comment, because the temptation to add the number is the obvious future
mistake.

### 2c. What is deliberately NOT built

- **No admin-side DSAR tooling** (M45-55 covers notice forms, SOR templates, operator export).
  Operators are legally responsible for their own instance; building it twice is worse.
- **No per-table cascade enumeration in application code.** The database does it. A hand-rolled
  table list would be 71 rows to keep in sync with the migrations and would drift silently —
  the failure mode this project has hit repeatedly.
- **No deletion receipt or cooling-off period.** A grace period that resurrects data conflicts
  with the request itself. If a cooling-off is wanted it is a separate reversible action.

## 3. Gates

- **Both engines.** The erasure test must run on PostgreSQL, because the cascade is enforced
  by the database and SQLite's FK enforcement settings differ. A cascade test that only runs
  on SQLite is a cascade test that has not been tested.
- **The erasure mutation, run twice.** Reverting the token revocation must turn exactly one
  test red; removing the `confirm_handle` check must turn exactly one red. Restored
  byte-identical both times (`diff`).
- **Both engines, `ON DELETE` behaviour asserted on the actual rows**, not on the schema text.
  The 71-table measurement above is a *specification*; the test is a fact about one account.
- **`{"works": []}`, never null** — applies to the disclosure object's sections.
- **No numeric resonance anywhere in the response**, checked by walking the parsed JSON
  rather than by grepping the string, because a nested number is the case that matters.

## 4. File layout

| file | what |
|---|---|
| `crates/db/src/erasure.rs` | `subject_data`, `plan_erasure`, `erase_account` |
| `crates/app/src/routes/erasure.rs` | `GET /me/data`, `POST /me/erasure` |
| `crates/app/tests/erasure.rs` | integration, both engines |
| `crates/app/tests/route_inventory.rs` | two `ROUTE_TABLE` rows, `Audience::Authenticated` |

## 5. The plan, step by step

Each step names the exact command and its expected output, because a plan an LLM cannot
execute is a plan that has not been written.

**Step 1 — the store, failing first.**
`crates/db/src/erasure.rs` with `subject_data`, `plan_erasure`, `erase_account`.
`cargo test -p lorehaven-app --test erasure --no-run` → compiles, 0 tests run.
The three tests are written to fail: a fixture account with a bookmark, a reading-progress row,
a pseudonym and a published work.

**Step 2 — verify the failure is real.**
`cargo test -p lorehaven-app --test erasure 2>&1 | grep 'test result'`
→ `ok. 0 passed; 3 failed`. **If it says 3 passed, the tests are not testing the store and
must be rewritten before anything else happens.**

**Step 3 — implement, SQLite.**
`cargo test -p lorehaven-app --test erasure` → `ok. 3 passed`.

**Step 4 — PostgreSQL.**
`LOREHAVEN_TEST_PG_URL='postgres://lorehaven:***@127.0.0.1:55433/postgres' \
   cargo test -p lorehaven-app --test erasure -- --test-threads=1`
→ `ok. 3 passed`.
**`--test-threads=1` is required on PostgreSQL for this suite**: at 2 threads the concurrent
migration replays exhaust `/dev/shm` and the tests fail with `could not resize shared memory
segment` before any assertion runs. This is recorded in memory and in the tracker.

**Step 5 — the routes, plus the route inventory rows.**
`cargo test -p lorehaven-app --test route_inventory`
→ `ok. 3 passed`. Without the two rows it fails with
`registered but not in ROUTE_TABLE`, which is the exact message a missing row produces.

**Step 6 — the forbidden-name test.**
`cargo test -p lorehaven-app --test erasure -- --nocapture forbidden`
→ 1 test, passing, and its name contains `forbidden`.

**Step 7 — the mutation.** Run, with `diff` proving restoration byte-identical each time:

| # | mutation | result |
|---|---|---|
| 1 | cancellation `state IN ('queued','running')` → `state = 'never-matches'` | 1 red: `erasure_leaves_no_export_a_worker_could_still_deliver` |
| 2 | PG bookmarks arm gains `AND 1 = 0` | 1 red: `subject_data_reports_the_readers_own_rows` |

**Two mutations of mine failed to be verdicts, and both are worth recording.**

- Mutation 2 was first written as `bookmarks: Vec::new()` in the struct literal. That produced a
  `warning: unused variable`, and `build.warnings = deny` turned it into a compile error — so the
  test run never happened. **A compile-err mutation is not a verdict.** Rewritten as a `WHERE 1 = 0`
  in the SQL, which is also the more realistic bug: the rows vanish because a clause is wrong,
  not because a function returns nothing.
- The first version of the export test **passed 10/10 against mutated production code.** It
  asserted the cancellation by re-implementing the same `UPDATE` in the test, so it measured the
  test's own copy. This is the third time on this project that a test re-implemented the thing it
  was testing and went green while the code was broken — the same mistake as
  `profile_empty_arms_agree.rs` in item 7. The rewrite asserts the end state through the real
  `erase_account`, and its header states plainly that a green run is **not** evidence the
  cancellation is present, so the next reader does not trust it.

**Step 8 — privacy mutation.** Covered by step 7's row 2, which filters the reader's bookmarks
out of the disclosure.

**Step 8a — the schema correction that cost three attempts.** The export test could not read the
export row's state after erasure: `RowNotFound`, because the row is gone. Two schema facts were
measured rather than assumed, and both are now in the test header — `export_jobs` cascades from
`accounts`, and `download_grants` has no `account_id` (it hangs off `export_job_id`, so a grant
survives iff its export does). Also measured: `export_jobs.created_at` is `timestamptz` on
PostgreSQL while the reading tables are `TEXT`, so their fixtures need an RFC3339 literal and an
explicit `::timestamptz` cast. Three failed attempts on one test is the cost of writing it from
the assumption instead of from the schema.

**Step 9 — full verification, both engines.**
SQLite: `cargo test --workspace -- --test-threads=2` → 0 failed.
PostgreSQL: `cargo test --workspace --no-fail-fast -- --test-threads=1` → 0 failed.
`--no-fail-fast` is required, or one environmental failure hides the suites behind it.

**Step 10 — update `docs/requirements.csv`** status to `implemented-verified-e2e` with an
`evidence` field citing `crates/db/src/erasure.rs` and
`frontend/src/lib/components/…` paths. **Not a plan document** — see the tracker's probe rule:
an evidence field citing a plan cannot prove a thing is rendered.