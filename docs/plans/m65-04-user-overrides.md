# M65-04 — per-user module overrides

Status: implementing. Owner decisions needed: none.
Spec: §59.3 (resolution chain), §59.6 (the table), §59.8 (`PATCH /api/v1/modules/{id}`), §51 (erasure).

## What the row asks for

> per-user module overrides: `module_user_state` table, `PATCH /api/v1/modules/{id}`, and the
> per-request narrowing in the gate.

M65-01 built the resolution chain down to the instance (`compiled default → [modules] disabled /
enabled`). This row is the third link: **the account**, and the rule that a user setting may only
*narrow*, never widen.

## The rules, stated before the code

1. **Only a `per_user` row is the reader's to change.** `arena`, `quiz`, `tasting`, `concierge`,
   `narration`, `draw`, `spoilers` carry `Toggle::PerUser`. An `Operator` or `Fixed` row answers
   **403 `MODULE_NOT_USER_TOGGLEABLE`** naming who does decide, because "no" without a reason sends
   the reader looking for a settings page that does not exist.
2. **A user row may only narrow.** `enabled: false` writes a row; `enabled: true` *deletes* it. A
   reader cannot switch on a surface the instance switched off — that is the operator's line, and
   the two would otherwise disagree invisibly about one URL.
3. **A planned row cannot be overridden.** `idea.*` / `home.trending` have no routes; a 404 already
   says so. `422` here would imply the request was the problem.
4. **Narrowing is invisible to everyone else.** The gate reads the account from the session the
   `load_session` layer attached; no session, no lookup, no narrowing.
5. **A read error in the lookup fails open.** A transient database error must not turn `arena` into a
   404 for every reader: the surface is on, one preference could not be read, and the request
   continues. Logged at `warn`.
6. **Deleting the account deletes the rows** (§51). `ON DELETE CASCADE` on `account_id`, and a
   snapshot-policy entry of `rekey_account` — a preference is not content, so it is rekeyed with the
   account rather than dropped.

## Deferred, with its reason

The cheap-common-case optimisation — an atomic "does any account hold an override" flag so the gate
skips the lookup when nobody has narrowed anything — is **not** built here. It saves one indexed
primary-key read on the seven `per_user` surfaces for authenticated requests, and it introduces a
cache that must be invalidated on write, at startup, and on a second process. Filed as **M65-09**
with this reason; the lookup is correct today and is one row.

## Files

| file | change |
|---|---|
| `migrations/{sqlite,postgres}/0134_module_user_state.sql` | new table |
| `docs/snapshot-column-policy.json` | `module_user_state.account_id` → `rekey_account` |
| `crates/domain/src/error.rs` | `ModuleNotUserToggleable { module, toggle }` → 403 |
| `crates/domain/src/modules/mod.rs` | `Module::may_user_toggle()`, `modules::find_any` |
| `crates/db/src/module_state.rs` | `is_off`, `overrides_for`, `set_off`, `clear`, `count` |
| `crates/app/src/routes/modules.rs` | `PATCH /{id}`, caller-resolved `enabled_for_caller` |
| `crates/app/src/server.rs` | the per-request narrowing inside `module_mount` |
| `crates/cli/src/module.rs` | `dh module off <id>`, `dh module on <id>` |

## Steps and their proofs

1. `cargo test -p darkhold-db --lib` → green, 0 new warnings.
2. `cargo test -p darkhold-db --test module_user_state` (SQLite) → `5 passed`.
3. `sudo docker start lh-pg-test`;
   `DARKHOLD_TEST_PG_URL=postgres://…@127.0.0.1:55433/postgres cargo test -p darkhold-db --test module_user_state -- --test-threads=1`
   → `5 passed`. Unset the variable before step 2's re-run — a shared DSN points the SQLite test at
   Postgres.
4. `cargo test -p darkhold-domain --lib modules` → `16 passed`.
5. `cargo test -p darkhold-app --test m65_user_overrides` → `6 passed`.
6. `cargo test -p darkhold-cli` → `16 passed`.
7. `cargo clippy -p darkhold-domain -p darkhold-db -p darkhold-app -p darkhold-cli --all-targets` →
   only the three pre-existing `work_ratings.rs` warnings.
8. Deploy: `migrate` (no flag) → `131 applied, 0 pending`; unit `active`; then the live proof in §Live.

## Mutations the tests must catch

| mutation | test that must fail |
|---|---|
| gate ignores the user row | `a_reader_who_switched_it_off_gets_404` |
| gate applies narrowing to an operator row | `an_operator_surface_is_not_narrowable` |
| `enabled: true` writes a row instead of deleting | `switching_it_back_on_removes_the_row` |
| a reader may enable a surface the instance disabled | `a_reader_cannot_widen` |
| `enabled_for_caller` reports the instance value | `the_catalogue_reports_the_callers_state` |
| `set_off` accepts a non-per_user id | `the_store_refuses_a_surface_that_is_not_per_user` (domain) |

## Live

```text
curl -s -o /dev/null -w '%{http_code}\n' -X PATCH .../api/v1/modules/quiz   → 401 (anonymous)
curl -s -X PATCH -b <cookie> -d '{"enabled":false}' .../modules/quiz         → 200, enabled false
curl -s -o /dev/null -w '%{http_code}\n' -b <cookie> .../api/v1/quiz/works   → 404 MODULE_UNAVAILABLE
curl -s -o /dev/null -w '%{http_code}\n' -b <other> .../api/v1/quiz/works    → 401
curl -s -X PATCH -b <cookie> -d '{"enabled":true}' .../modules/quiz          → 200, row gone
```

Registration may be closed on the host. If no session can be obtained, the live arm reports
`unverified — no session` and the milestone stays `implemented-fully-tested`, exactly as M45-25 did
for its operator doors.

---

## What landed, and the three divergences

`module_user_state` exists on both engines (migration 0134), the store is
`crates/db/src/module_state.rs` (five functions, no widening path), `PATCH /api/v1/modules/{id}` is
the only write, and the gate in `module_mount` reads the row for `per_user` surfaces when the request
carries a session. `dh module off|on <id>` drives the same door with a bearer token.

1. **No `ON DELETE CASCADE`.** §51 erases by rekeying the account rather than deleting it, so a
   cascade would never fire and would promise a guarantee the table does not get. The snapshot
   policy is `rekey_account`; the erasure pass is **M65-10**.
2. **The "only pays when someone holds an override" cache is not built.** It saves one indexed
   primary-key read on seven surfaces and adds invalidation at startup, on write and across
   processes. **M65-09** carries the reason.
3. **`enabled_for_caller` is computed with the gate's own rule** (`per_user` and overridden), not
   with "is this id in the override list". The first version used the list alone and reported
   `catalogue` as gone for a caller who held a stray row on it — while the gate went on serving it.

Two bugs the tests found, both worth the sentence:

- `overridden` returned `clear()`'s "was there a row", so switching a surface **back on** answered
  `overridden: true` and `enabled_for_caller: false` — the exact opposite, on the happy path.
- The stray-row case above. It needed a test that writes a row the route refuses to write, because
  a route-level test cannot reach the gate's mistake.

## Mutations

| mutation | caught by |
|---|---|
| the gate ignores the caller's row | `a_reader_who_switched_it_off_gets_404_and_their_neighbour_does_not` |
| the gate narrows an operator surface too | `a_row_for_an_operator_surface_does_not_silence_it` |
| `enabled: true` stores a row instead of clearing it | `switching_it_back_on_removes_the_row` |
| a reader may enable what the instance disabled | `a_reader_cannot_widen_a_surface_the_instance_switched_off` |
| `enabled_for_caller` is just `enabled` | `the_catalogue_reports_the_callers_state_beside_the_instances` |
| every row is the reader's to toggle | `only_a_per_user_row_is_the_readers_to_change` |

The second mutation was **missed on the first run** and the sixth was a **compile error**, not a
killed mutation: the first had no test that could reach the gate's decision for a non-per_user row,
and the second invented a `Toggle` variant that does not exist. Both were re-cut — a test added and
the mutation rewritten to keep the enum intact — and both then failed the right test.
