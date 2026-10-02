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
| `MechanismDeclaration`, `::is_valid` | same | yes |
| `Mechanism`, `FlowSummary::compose`, `::exceeds` | same | yes |
| The data to feed it | `credit_entries` + `credit_transactions` (migration 0017) | yes |
| **A store that reads real ledger rows into `Mechanism`** | — | **no** |
| **A declaration registry** | — | **no** |
| **An operator-only route** | — | **no** |

The domain types were built first and are not in question. The remaining work is the
registry, the store, and the route.

## The load-bearing constraint

**Credits are closed-loop and never cashable.** The dashboard shows that the property holds;
it does not enforce it. Three rules follow, each a requirement rather than a style choice:

1. **A mechanism declares its side; nothing infers it from the sign of its entries.**
   `flows.rs` already says why: a bug in a faucet produces a *negative* amount, which is
   precisely the case where "negative means sink" is wrong.
2. **`FlowSummary::undeclared` must never be silently dropped.** A dashboard that omits the
   mechanisms nobody classified reports a *smaller economy than exists* — the inflation
   problem restated. A registry miss is counted as undeclared and still contributes to
   `net_credits`.
3. **`exceeds()` warns and does nothing else.** No clamping, no suspension, no automatic
   adjustment. §0.3 makes bought ranking and bought trust non-negotiable, so a threshold that
   acted would be the economy deciding what a reader may earn.

## Two facts established by reading the code

Both of these were established by reading the tree, not inferred, and the first one is a
trap.

### `amount_bp` holds whole credits, not basis points

The column name is a historical artefact. `economy.rs:173` binds the caller's `i64` straight
into it, and `preservation.rs:1279` says so in a comment. **Do not divide by 100.** This plan
originally said to, from the column name alone, and was corrected.

### Two mechanisms share a key prefix with opposite sides

Every `post_transaction` call in the tree, as of `b37c535`:

| `reference` | Written by | `TxnType` | Side |
|---|---|---|---|
| `tip:{work_id}` | `routes/monetization.rs:340` | `Spend` | **sink** |
| `{member_id}` | `preservation.rs:1177` | `Preservation` | **sink** |
| `{member_id}` | `preservation.rs:1221` | `PreservationReclaim` | **faucet** |
| `{work_id}` | `payout_store.rs:323` | `Earn` | **faucet** |

The two preservation rows share the literal `reference` value `{member_id}` and sit on
**opposite sides of the loop**. A registry keyed on the raw `reference` string cannot
classify them, and since a member id differs per member it would miss every one of them and
report the whole preservation mechanism as undeclared.

**So the registry key is a mechanism name, not a raw reference**, and deriving it is the
store's job:

| Mechanism | Derived from | Side |
|---|---|---|
| `tips` | `reference LIKE 'tip:%'` | sink |
| `preservation_dues` | `TxnType::Preservation` | sink |
| `preservation_reclaim` | `TxnType::PreservationReclaim` | faucet |
| `author_earnings` | `TxnType::Earn` | faucet |

This is the design decision of the row, and it is why the registry is *data*: a fifth
mechanism is a registry row plus a derivation clause, not a schema change.

`TxnType` lives on `credit_transactions`, not `credit_entries`, so the store must `JOIN` —
`preservation.rs:1249` already does exactly this, follow that shape.

## Step 1 — the declaration registry

**Migration 0112**, both engines: `mechanism_key TEXT PRIMARY KEY`, `flow TEXT NOT NULL`
CHECK in ('faucet','sink','neutral'), `label TEXT NOT NULL`, `created_at TEXT NOT NULL`.

No uuid column is involved, so the two files are identical apart from CHECK syntax — the
TEXT/uuid split does not apply here.

Seed exactly the four rows above. If a later audit finds a `post_transaction` call this
listing missed, that is a bug in this plan, not a reason to widen the registry.

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

- `JOIN credit_transactions`, group by the derived mechanism, `SUM(e.amount_bp)` per group
  within the window.
- Look each mechanism up in `mechanism_declarations`. A miss yields
  `MechanismDeclaration::invalid()`, so it counts as **undeclared** while still contributing
  its net — rule 2.
- **`since`/`until` are interpolated as strings**, matching the rest of the codebase. Do not
  introduce a binding-style change in this step.

## Step 3 — the route

**File:** `crates/app/src/routes/flows.rs` (new)

`GET /operator/economy/flows?since=&until=`, `RouteClass::Default`, operator-only via the
extractor `admin_discovery.rs` uses. Response:

```json
{ "faucet_credits": 0, "sink_credits": 0, "net_credits": 0, "undeclared": 0,
  "mechanisms": [ { "key": "tips", "flow": "sink", "net_credits": 0, "declared": true } ] }
```

`mechanisms` is the per-mechanism breakdown, not just the summary — §53.2's dashboard is "a
balance and a composition", and a composition without its parts is useless.

**The threshold is reported and nothing more.** See rule 3.

## Step 4 — tests

**File:** `crates/app/tests/flow_dashboard.rs` (new)

`TestDb::applied_migrations()` returns the full ledger as of `8cbba50`, so migration
assertions work here.

Cases that matter, in order:

0. **`preservation_dues` and `preservation_reclaim` classify oppositely from the same key
   prefix.** Seed both for one member; assert one lands in `sink_credits` and the other in
   `faucet_credits`. This is the test that proves the derivation, and an implementation that
   grouped on the raw `reference` cannot pass it.
1. A declared faucet contributes to `faucet_credits`, a declared sink to `sink_credits`.
2. **A negative faucet stays a faucet.** Seed a faucet with a negative `amount_bp`; assert it
   is still `faucet` and lands in `faucet_credits`. This is the test that proves rule 1, and
   an inferred implementation fails it.
3. An **undeclared** mechanism lands in `net_credits`, appears with `declared: false`, and
   increments `undeclared` — its amount is still counted. Rule 2.
4. Window boundaries: an entry outside `[since, until]` is excluded, one exactly on `since`
   is included. Both endpoints inclusive, and the test says so.
5. A non-operator gets the same 404 the rest of the admin surface uses, not a 403 that
   confirms the route exists.
6. `exceeds()` crossing the threshold **does not change the response** — byte-identical either
   side. No clamp, no suspension field.

Cases 0, 2, 3 and 6 will not pass by accident.

## Step 5 — frontend

Optional, and last. §53 calls this an operator dashboard, so it belongs on an existing
operator page, not a new public route. It renders the balance, the composition bars, and an
**explicit undeclared warning** — the undeclared count is not a footnote, it is the number
that tells the operator their classification is incomplete.

There is nothing here a reader should see, so build no route.

## What this plan deliberately does not do

- **No cash-out, no withdrawal, no conversion.** "Never cashable" is a property of the
  absence of such a mechanism; adding one would violate it.
- **No automatic threshold response.** Rule 3.
- **No inference of flow from the sign of entries.** Rule 1.
- **No per-account faucet/sink view.** This is the instance economy; §20.3's payout ledger
  already covers per-author.

## Verify the whole step

```bash
cd ~/code-local/rust/lorehaven && export PATH="$HOME/.cargo/bin:$PATH"
cargo fmt --all
cargo clippy -p lorehaven-db -p lorehaven-app --all-targets   # must print nothing
cargo test -p lorehaven-db -p lorehaven-app --no-fail-fast -- --test-threads=4
LOREHAVEN_TEST_PG_URL='postgres://postgres:smoke_pw@127.0.0.1:5432/postgres' \
  cargo test -p lorehaven-db -p lorehaven-app --no-fail-fast -- --test-threads=4
```

**Do not run a mutation harness concurrently with any of the above, and do not `git add -A`
while one runs.** A gate edits the source under test; a concurrent `cargo test` compiles
mutated source and reports the gate's intended RED as a failure, and a broad add stages
whatever mutation is applied at that moment. That has produced four false results in this
project, one of which shipped a bug.