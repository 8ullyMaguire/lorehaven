# Plan — calibrated decision models (Laya) on thinkcentre

Implements `docs/spec-amendments/calibrated-decision-models.md`. Every step
carries the exact command and its expected output, so the whole plan can be
followed by an LLM with no other context.

## Step 0 — the model is already deployed and answering (done, verified)

Unsloth `2026.9.12` is installed at `/home/alvaro/.local/bin/unsloth`, Studio is
running on `127.0.0.1:8888`, and `~/.unsloth/systemone.key` exists. Verified:

```console
$ ssh thinkcentre 'curl -s -X POST http://localhost:8888/v1/systemone \
    -H "Authorization: Bearer $(cat ~/.unsloth/systemone.key)" \
    -H "Content-Type: application/json" \
    -d "{\"model\":\"laya\",\"state\":\"charged twice, need a refund today\",
         \"questions\":{\"refund\":{\"type\":\"noul\",
         \"instructions\":\"Does the customer ask for a refund?\"}}}"'
{"model":"laya-multilingual","answers":{"refund":{"type":"noul","noul":0.9965}},
 "usage":{"input_tokens":41,"output_tokens":0}}
```

Expected for the build's own smoke test: `"noul":0.9965`. The exact float is
model-version-dependent; the assertion is `> 0.9`, never an equality.

**No deployment step is required.** The remaining work is the Lorehaven side.

## Step 1 — a decision client crate

New file: `crates/decisions/Cargo.toml` and `crates/decisions/src/lib.rs`.
Register in the workspace `Cargo.toml` members.

```toml
[package]
name = "lorehaven-decisions"
version = "0.1.0"
edition = "2021"

[dependencies]
serde = { workspace = true }
serde_json = { workspace = true }
thiserror = { workspace = true }
reqwest = { workspace = true, default-features = false, features = ["json", "rustls-tls"] }
tokio = { workspace = true, features = ["time"] }
```

The API surface, in full:

```rust
pub struct ClientConfig { pub base_url: String, pub model: String,
                          pub api_key: Option<String>, pub timeout_ms: u64 }

/// The question kinds Unsloth's Decision API accepts.
pub enum Question { YesNo { key: String, instructions: String },
                    Choice { key: String, instructions: String, criteria: BTreeMap<String, String> },
                    Score  { key: String, instructions: String, levels: Vec<String> } }

/// One answer, as the model returns it. The three shapes are distinct because
/// they are distinct, not because a sum type needs variety.
pub enum Answer { YesNo { probability: f64 },
                  Choice { choice: String, probabilities: BTreeMap<String, f64>, confidence: f64 },
                  Score  { score: f64, confidence: f64 } }

pub struct Decision { pub key: String, pub answer: Answer }

#[derive(Debug, thiserror::Error)]
pub enum DecisionError {
    #[error("the decision model at {url} did not answer: {source}")]
    Unreachable { url: String, #[source] source: reqwest::Error },
    #[error("the decision model answered {status}")]
    Status { status: u16 },
    #[error("the decision model answered without {key}")]
    MissingKey { key: String },
    #[error("the model returned {0}, which is not a probability")]
    NotAProbability(f64),
}

impl Client {
    pub fn new(config: ClientConfig) -> Self;
    /// Ask. `state` is a string or any JSON — the API takes either, and which
    /// one is used must not change the answer, so it is always a JSON value.
    pub async fn ask(&self, state: &serde_json::Value,
                     questions: &[Question]) -> Result<Vec<Decision>, DecisionError>;
}
```

**A rejected non-probability is a refusal, not a clamp.** A `NaN` or a value
outside `0.0..=1.0` returns `NotAProbability` rather than being clamped, because
a clamped `1.9` becomes `1.0` — the strongest possible answer — out of garbage.

Verification:

```console
$ cargo build -p lorehaven-decisions
# expected: no errors
$ cargo test -p lorehaven-decisions
# expected: N passed; the tests use a local mock server, never the real model
```

## Step 2 — the one policy that makes a model safe here

New file: `crates/decisions/src/reconcile.rs`. This is the load-bearing file of
the whole change.

```rust
/// §11.14's three outcomes, plus the model's contribution.
pub enum QualityCall {
    Accepted,
    Rejected { reason: String },   // structural, from the deterministic path
    Held { reason: String },
}

/// Reconcile the deterministic classification with a calibrated posterior on
/// "is this a work?".
///
/// The asymmetry is the feature, and it is not negotiable: the model can turn
/// an ACCEPT into a HOLD and can do nothing else. It cannot reject, because
/// §11.14 reserves rejection for *structural* facts (an empty title, an empty
/// author) that a text model has no access to and cannot outvote. It cannot
/// accept either — acceptance is the deterministic path's to grant, on
/// evidence the model never saw.
pub fn reconcile(deterministic: QualityCall,
                 posterior: Option<f64>,
                 accept_threshold: f64) -> QualityCall;
```

The table this implements, which is the amendment's §3.1 verbatim:

| deterministic | posterior | result |
|---|---|---|
| `Rejected` | anything | `Rejected` — the reason stands |
| `Held` | anything | `Held` — §11.14 makes zero-word-count a rule |
| `Accepted` | `None` (model absent) | `Accepted` |
| `Accepted` | `p < accept_threshold` | `Held { reason }` |
| `Accepted` | `p >= accept_threshold` | `Accepted` |

**Tests, and they are the real work of this step.** A table like this is
trivially writable and trivially wrong, so pin every cell, and pin the
direction — a test that accepts a wrong monotone behaviour is worse than no
test:

1. `Rejected` + `p = 0.99` (high confidence it IS a work) → still `Rejected`.
   This is the anti-regret cell and it is the one most likely to be written
   wrong.
2. `Held` + `p = 0.01` → still `Held`.
3. `Accepted` + `p = 0.0` → `Held`.
4. `Accepted` + `p = 1.0` → `Accepted`.
5. `Accepted` + `p = accept_threshold` exactly → `Accepted` (inclusive).
6. `Accepted` + `p` just below → `Held`.
7. `Accepted` + `None` → `Accepted`.
8. **Monotonicity:** sweeping `p` from 0.0 to 1.0 in 1000 steps produces a
   `Held` prefix and an `Accepted` suffix, and never the reverse. A crossing
   more than once is a failure even if every individual cell passes.

Verification:

```console
$ cargo test -p lorehaven-decisions reconcile
# expected: 8 passed
```

**Mutation gate — run this before the change is considered done.** Each of
these must make a test fail:

```console
# in reconcile, swap `p < accept_threshold` for `p > accept_threshold`
$ cargo test -p lorehaven-decisions reconcile   # MUST fail
# restore, then let the model reject:
#   Rejected => return Call::Held { .. } instead of returning the call
$ cargo test -p lorehaven-decisions reconcile   # MUST fail
# restore, then let the model accept a Held:
#   Held => return Call::Accepted
$ cargo test -p lorehaven-decisions reconcile   # MUST fail
```

Restore in the same shell as the mutation, and confirm with `git diff` that the
file is back — a restore that is not re-verified is an unreported revert.

## Step 3 — the §11.14 call site

`crates/domain/src/imports.rs`: `ImportWork::classify_quality` stays exactly as
it is. Add beside it:

```rust
impl ImportWork {
    /// The calibrated call, when a provider is configured (§11.14, amendment §3.1).
    ///
    /// Returns the deterministic answer untouched when there is no posterior,
    /// which is the deployed state of every instance that has not opted in.
    #[must_use]
    pub fn classify_quality_calibrated(&self, posterior: Option<f64>,
                                       accept_threshold: f64) -> ImportQuality {
        lorehaven_decisions::reconcile::reconcile(
            self.classify_quality().into(), posterior, accept_threshold)
    }
}
```

`ImportQuality` and the call above are in different crates, so a small
`From<ImportQuality> for QualityCall` is the bridge. It must be total: every
variant maps, and there is no default arm.

The import worker is the only caller. `crates/app/src/imports.rs`, where
`classify_quality()` is called, gains the posterior from the provider when one
is configured and `Ok(vec![])` — a cache-mode instance, where no body is
fetched — never consults it, because there is no text to classify.

Verification:

```console
$ cargo test -p lorehaven-domain --lib imports
# expected: all pass, including a new test that a disabled provider is a no-op
```

## Step 4 — configuration

`crates/app/src/config.rs`, beside `RetentionConfig`:

```rust
pub struct DecisionsConfig {
    pub provider: DecisionProvider,     // Deterministic | Calibrated
    pub base_url: String,
    pub model: String,
    pub api_key: Option<String>,        // env: LOREHAVEN_DECISIONS_API_KEY
    pub timeout_ms: u64,
    pub accept_threshold: f64,
    pub consult_floor: f64,
}
```

`Default` is `provider: Deterministic`, `base_url: "http://127.0.0.1:8888"`,
`timeout_ms: 5000`, `accept_threshold: 0.90`, `consult_floor: 0.10`.

**Validate at load, and reject rather than clamp:** `accept_threshold` and
`consult_floor` must be in `0.0..=1.0` and `consult_floor <= accept_threshold`.
A `NaN` threshold fails this, which is why the check is `!(a <= b)` and not
`a < b` — the latter is true for `NaN`.

Verification:

```console
$ cargo test -p lorehaven-app --lib config
# expected: a test that a NaN threshold is refused, not clamped
```

## Step 5 — the audit trail  *(DONE, 2026-09-29 — commit 99039aa)*

Delivered as specified. One deviation from the plan's SQL, and it is the plan's
own draft that was wrong: `created_at` is declared `TEXT` in both dialects
rather than a timestamp type, because the repository's existing rows store
timestamps as TEXT and the Postgres read casts with `::text AS created_at`. A
narrower `TIMESTAMPTZ` here would have been more correct and would have
inconsisted with every neighbouring table.



`/decisions/audit` currently returns `{ "items": [] }` — an endpoint that has
always been empty. Give it a table:

`migrations/sqlite/0088_decision_audit.sql` and the Postgres twin:

```sql
CREATE TABLE decision_audit (
    id             TEXT    PRIMARY KEY,
    task           TEXT    NOT NULL,
    subject        TEXT    NOT NULL,   -- what was classified; never body text
    deterministic  TEXT    NOT NULL,   -- accepted | rejected | held
    posterior      REAL,               -- NULL when no model answered
    threshold      REAL,
    outcome        TEXT    NOT NULL,   -- what was actually applied
    provider       TEXT    NOT NULL,
    created_at     TEXT    NOT NULL,
    updated_at     TEXT    NOT NULL,
    version        INTEGER NOT NULL DEFAULT 1
);
CREATE INDEX decision_audit_created ON decision_audit (created_at DESC, id DESC);
CREATE INDEX decision_audit_task ON decision_audit (task, created_at DESC);
```

**`subject` holds an id or a URL, never the classified text.** A moderation
audit that stores the text it moderated is a second copy of the content with
none of the first copy's audience rules, and the whole point of §12.1 is that
the text stays with its author.

Verification:

```console
$ cargo test -p lorehaven-db --lib decision
# expected: pass on BOTH backends. The multi-backend comparison is
#   the_two_dialects_declare_the_same_columns_and_indexes
```

## Step 6 — disclosure on `/api/v1/meta`  *(DONE, 2026-09-29)*

Delivered, with one addition the plan did not ask for and should have: the
no-leak test pins the disclosure's **key set** to exactly
`{provider, accept_threshold, consult_floor}`. A test that only asserts the
three values are correct passes just as happily against a fourth field
carrying the model host.

A test bug worth recording, because the first version of it failed and the
failure was the *test's* fault: scanning the whole `/meta` body for
`base_url` and `localhost` fails against `base_url`'s pre-existing top-level
occurrence, which is the instance's own public address. A whole-body scan is
the wrong scope, and "delete the failing assertion" is the wrong fix.



`crates/app/src/routes/meta.rs` gains the provider name and the two
thresholds. §0.4.3 requires an instance to disclose what it is that decides;
an instance whose comments are filtered by a local model owes the same
disclosure as one that discloses its trust floor.

Verification:

```console
$ cargo test -p lorehaven-app --test meta
# expected: pass, and a test that a deterministic instance still discloses
#          "deterministic" rather than omitting the field
```

## Step 7 — deploy on thinkcentre  *(BLOCKED, see below)*

### Status as of 2026-09-29: the model is deployed; the Lorehaven service is not, and must not be

What is verified working on `thinkcentre`:

| check | result |
|---|---|
| Unsloth serving Laya | `gravity-decision-provider.service`, active, `Restart=on-failure` |
| `/v1/systemone` answers | yes, via `curl` and via `lorehaven-decisions` |
| `cargo build -p lorehaven-decisions --bin probe` on the host | ok |
| probe → real model, correct key | `refund = 0.9816`, exit 0 |
| probe → unreachable host | refuses, exit 1, `is_transient() == true` |

So the model is deployed and configured and Lorehaven's client drives it. What
is **not** done is rebuilding or restarting `lorehaven.service`, and the reason
is not technical friction:

**The deployed instance runs a different branch, 263 commits behind, and its
database is 15 migrations behind.** `/personal/documents/code/rust/lorehaven`
is on `fix/pawchive-tag-and-author-parsing` at `953dea0`; its binary was built
Sep 25 and its `migrations/postgres/` has 72 files where this branch has 87.
Deploying means applying migrations 0072–0091 to a **live Postgres database**
that currently has 72 applied.

That is a much larger change than "turn on a decision model", and it carries
15 migrations belonging to other milestones (M45 roadmap cards, M59 retention,
body audience, taste settings, taxonomy review) that have never run on this
host. The audit that made this decidable:

- **All 17 pending migrations are additive.** The single `DROP TABLE` match
  across all of them is inside a comment in
  `0075_fix_device_deliveries_export_job_fk.sql`.
- **The numbering gaps (0084, 0088, 0089) are harmless.** sqlx records applied
  versions, not sequence positions, so a gap is not a failure.
- **0084 is absent on this branch too** — not a divergence introduced here.

So the migrations would very likely apply cleanly. "Very likely" against a
production database, at the cost of dragging five other milestones' schema
changes into a live instance unrequested, is exactly the irreversible-and-costly
case where the decision belongs to the operator and not to me.

**The unblocking step is one command, once the branch question is settled:**

```console
$ ssh thinkcentre 'cd /personal/documents/code/rust/lorehaven && \
    git merge --ff-only origin/feat/calibrated-decisions'
# then, in the config, add the [decisions] section from
# lorehaven.toml.example, and:
$ systemctl --user restart lorehaven
```

Note the deployed binary is built with `CARGO_TARGET_DIR=/home/alvaro/.cargo-target/lorehaven`,
per the unit file — not the default. A rebuild that omits it will not replace
the running binary, and the service will restart onto the old one and look like
it worked.

### The original step, for the record


The Lorehaven service reads its config from a file; the model is a separate
long-running process. On thinkcentre:

```console
$ ssh thinkcentre 'systemctl --user status unsloth-studio --no-pager | head -3'
```

If that unit does not exist, the model is started by hand and must survive a
reboot — an instance whose decisions silently degrade to `deterministic` on
Monday because a shell exited on Sunday is worse than one that never had the
model, because the operator believes they configured it. Either a user unit
with `Restart=always`, or the setting stays off.

Set in the service's environment:

```toml
[decisions]
provider = "calibrated"
base_url = "http://127.0.0.1:8888"
accept_threshold = 0.90
consult_floor = 0.10
```

plus `LOREHAVEN_DECISIONS_API_KEY` from `~/.unsloth/systemone.key`, which is
readable only by `alvaro` and is never written into the config file or into a
git-tracked file.

## Step 8 — end-to-end verification, in this order

Each is a separate command because each can fail alone and the failure is the
information:

```console
# 1. the model answers through the client, on the host that will run it
$ ssh thinkcentre 'cd <repo> && cargo run -p lorehaven-decisions --bin probe'
# expected: one question, a posterior printed, exit 0

# 2. the app still classifies with the model OFF — the floor
$ cargo test --workspace
# expected: every suite green, provider deterministic

# 3. the app classifies with the model ON and the model reachable
$ LOREHAVEN_DECISIONS_PROVIDER=calibrated cargo test -p lorehaven-app --test m11_14_quality
# expected: green, and the calibrated cases actually consulted the model
#           (assert the posterior is Some, not None — a green run where
#            every posterior is None has tested nothing)

# 4. the app classifies with the model ON and the model UNREACHABLE
$ LOREHAVEN_DECISIONS_PROVIDER=calibrated LOREHAVEN_DECISIONS_BASE_URL=http://127.0.0.1:9 \
    cargo test -p lorehaven-app --test m11_14_quality
# expected: green, every posterior None, and the deterministic answers intact
#           — this is the degradation path and it is the one that must never
#            be green for the wrong reason

# 5. no unauthenticated POST learned anything new
$ cargo test -p lorehaven-app --test route_inventory
# expected: green
```

Step 4 is the one that matters most. An instance whose model is down must keep
importing, keep holding works, and keep its §11.14 answers. A run that is
green because the provider silently did nothing — rather than because the
fallback is correct — proves nothing, so the test asserts the *reason* the
posterior is `None`.

## Step 9 — the ledger

`docs/requirements.csv`: add rows for the decision provider, the reconciliation
rule, the audit table, and the disclosure, each with the test that pins it and
the mutation that kills it. Mark nothing `implemented-verified-e2e` until
step 8 has run on the deployed host.
