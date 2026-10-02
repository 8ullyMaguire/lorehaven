# M45-18 — Faucet/sink dashboard

**Status:** planned → in progress. This plan is executable by an LLM with no other context.

## What this row is

`docs/requirements.csv` M45-18:

> Faucet/sink dashboard; credits stay closed-loop and never cashable
> Notes: Gaps review A11. Faucets pay for signal or supply; sinks convert credits into supply.

## What already exists

| Thing | Where | Done? |
|---|---|---|
| `Flow` enum (Faucet/Sink/Neutral) | `crates/domain/src/flows.rs` | yes, 8 tests |
| `MechanismDeclaration`, `MechanismDeclaration::is_valid` | same | yes |
| `Mechanism`, `FlowSummary::compose`, `FlowSummary::exceeds` | same | yes |
| The data to feed it | `credit_entries` (migration 0017): `reference` is the mechanism key, `amount_bp` is signed | yes |
| **A store that reads real ledger rows into `Mechanism`** | — | **no** |
| **A declaration registry (which keys are faucets, which are sinks)** | — | **no** |
| **An operator-only route** | — | **no** |

**So the remaining work is the store, the registry, and the route.** The domain types were
built first and are not in question.

## The load-bearing constraint

**Credits are closed-loop and never cashable.** The dashboard's job is to make an operator
able to see that property holds, not to enforce it. Three rules follow, and each is a
requirement rather than a style choice:

1. **A mechanism declares its side; nothing infers it from the sign of its entries.**
   `flows.rs` already says why: a bug in a faucet produces a *negative* amount, which is
   precisely the case where "negative means sink" is wrong. The store must read the
   registry, never the sign.
2. **`FlowSummary::undeclared` must never be silently dropped.** A dashboard that omits the
   mechanisms nobody classified reports a *smaller economy than exists* — which is the
   inflation problem restated. If the registry lookup misses, the mechanism is counted as
   undeclared and still contributes to `net_credits`.
3. **`exceeds()` warns and does nothing else.** No clamping, no suspension, no automatic
   adjustment. §0.3 makes bought ranking and bought trust non-negotiable, so a threshold that
   acted would be the economy deciding what a reader may earn.

## Step 1 — the declaration registry

**File:** `crates/db/src/flow_registry.rs` (new)

A table of known mechanism keys and their side. This is data, not code: §53.1's point is
that classification is *declared*, and a new mechanism added to the product must be
declared here or show up as undeclared.

**Migration 0112**, both engines. `mechanism_key TEXT PRIMARY KEY`, `flow TEXT NOT NULL`
CHECK in ('faucet','sink','neutral'), `label TEXT NOT NULL`, `created_at TEXT NOT NULL`.

Note the sixth→seventh site of the TEXT/uuid split does **not** apply: no uuid column is
involved, so the two files differ only in the CHECK syntax and nothing else.

Seed the rows the existing economy already uses. Read them off
`crates/db/src/payout_store.rs` and `crates/db/src/economy*` before writing: every distinct
`reference` value written today needs a row here, or it reports as undeclared on day one.
**This is the step most likely to need iteration — do not guess the key list.**

**Verify:**

```bash
cd ~/code-local/rust/lorehaven && export PATH="$HOME/.cargo/bin:$PATH"
LOREHAVEN_TEST_PG_URL='postgres://postgres:smoke_pw@127.0.0.1:5432/postgres' \
  cargo test -p test-support -- --test-threads=2
```

Expected: migrations apply on both engines, no checksum error.

## Step 2 — the store

**File:** `crates/db/src/flow_store.rs` (new)

```rust
pub async fn mechanisms_in_window(db: &Database, since: &str, until: &str)
    -> Result<Vec<Mechanism>>
```

- Group `credit_entries` by `reference` over the window, `SUM(amount_bp)` per group.
- Look each key up in `mechanism_declarations`. A miss yields
  `MechanismDeclaration::invalid()` so the mechanism counts as **undeclared** while still
  contributing its net — rule 2 above.
- `amount_bp` is basis points, `Mechanism.net_credits` is credits. Divide by 100 at the
  boundary, once, with a comment saying where. Integer division truncates; that is correct
  here because a truncation of at most 0.01 credit cannot flip a sign, and rule 1 means the
  side never comes from this value anyway.
- **`since`/`until` are the caller's strings, interpolated as today** — the codebase does
  this throughout for dates. Do not introduce a binding style change in this step.

## Step 3 — the route

**File:** `crates/app/src/routes/flows.rs` (new)

`GET /operator/economy/flows?since=&until=` — `RouteClass::Default`, operator-only via the
existing `RequireOperator` extractor if one exists, otherwise the same shape
`admin_tiscovery.rs` uses. Response:

```json
{ "faucet_credits": 0, "sink_credits": 0, "net_credits": 0,
  "undeclared": 0, "mechanisms": [ { "key": "...", "flow": "faucet",
                                     "net_credits": 0, "declared": true } ] }
```

`mechanisms` is the per-mechanism breakdown, not just the summary — §53.2's dashboard is
"a balance and a composition", and the composition is useless without the parts.

**No threshold action.** The route reports; it never enforces. See rule 3.

## Step 4 — tests

**File:** `crates/app/tests/flow_dashboard.rs` (new)

The store-level behaviour plus what only HTTP can get wrong. `TestDb::applied_migrations()`
now returns the full ledger, so migration assertions work (see `8cbba50`).

Cases that matter, in order:

1. A declared faucet contributes to `faucet_credits` and a declared sink to `sink_credits`.
2. **A negative faucet stays a faucet.** Seed a faucet with a negative `amount_bp`; assert
   it is still classified `faucet` and lands in `faucet_credits`. This is the test that
   proves rule 1, and it is the one an inferred implementation fails.
3. An **undeclared** key lands in `net_credits`, appears in `mechanisms` with
   `declared: false`, and increments `undeclared`. Its amount is still counted — rule 2.
4. The window boundary: an entry outside `[since, until]` is excluded, one exactly on
   `since` is included. Both endpoints inclusive, and the test says so.
5. **An operator cannot reach this without the operator role**, and a non-operator gets the
   same 404 the rest of the admin surface uses rather than a 403 that confirms the route
   exists.
6. `exceeds()` crossing the configured threshold **does not change the response** — no clamp,
   no suspension field. Assert the response is byte-identical either side of the threshold.

Cases 2, 3 and 6 are the ones that will not pass by accident.

## Step 5 — frontend

Optional, and last. §53 calls this an operator dashboard, so it belongs on an existing
operator page rather than a new public route. A component that renders the balance, the
composition bars, and **an explicit undeclared warning** — the undeclared count is not a
footnote, it is the number that tells the operator their classification is incomplete.

Do not build a route for it. There is nothing here a reader should see.

## What this plan deliberately does not do

- **No cash-out, no withdrawal, no conversion to anything.** "Never cashable" is a property
  of the absence of such a mechanism. Adding one would violate it.
- **No automatic threshold response.** Rule 3.
- **No inference of flow from the sign of entries.** Rule 1.
- **No per-account faucet/sink view.** This is the instance economy, not a personal
  statement; §20.3's payout ledger already covers per-author.

## Verify the whole step

```bash
cd ~/code-local/rust/lorehaven && export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all
cargo clippy -p lorehaven-db -p lorehaven-app --all-targets   # must print nothing
cargo test -p lorehaven-db -p lorehaven-app --no-fail-fast -- --test-threads=4
LOREHAVEN_TEST_PG_URL='postgres://postgres:smoke_pw@127.0.0.1:5432/postgres' \
  cargo test -p lorehaven-db -p lorehaven-app --no-fail-fast -- --test-threads=4
```

**Do not run a mutation harness concurrently with any of the above.** A gate edits the source
under test; a concurrent `cargo test` compiles mutated source and reports the gate's intended
RED as a failure, and a broad `git add -A` will stage whatever mutation is applied at that
moment. That has produced four false results in this project already.