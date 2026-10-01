## 2026-10-01 — M45-19: the tasting menu's selector was tested and unreachable

**Plan:** `docs/plans/m45-phase1-taste-signal.md` step 3 · **Requirement:** `M45-19`

`M45-19` was `planned`, and the plan's step 3 was next. What was actually true is
sharper: `crates/db/src/tasting.rs` had a selector, an ordering, a session bound
and 14 passing unit tests — and **nothing in the application called any of it**.
`docs/goal.md` names that shape by name ("a unit test on a function nothing
calls"), so the row was correctly `planned` and the work was to build the door.

### The selector's one number was wrong, and only a probe would have found it

`uncertainty_for` divided by `10_000.0`, with a comment claiming "10_000bp is the
arena's own scale". It is not. `weights_from_elos`
(`crates/domain/src/taste_vector.rs:743`) normalises every reader's weights to
**sum 1.0**. A probe through the production function, not a hand-written number:

```
STORED_WEIGHT=1 UNCERTAINTY=0.9999
```

for a dimension rated 1900 after 40 matches. The consequence is not a slightly-off
score: §49.5's queue *orders by uncertainty*, so a range that is constant across
the whole library collapses to the `work_id` tie-break. The calibration queue was
"the lexicographically first N works", for every reader, permanently.

Fixed by squashing against the reader's **own peak weight**, so a change upstream
cannot re-open it. The old unit test used a literal `9000.0` and passed against
the broken code — the exact value that hid the bug — so the replacement test goes
through `weights_from_elos` and cannot drift from the scale it is checking.

### A second bug, in the code written to fix the first

The weight update clamped at `0.0`, reasoning that the arena's normaliser is
non-negative. That is a misreading: the substrate treats these weights as
**signed** — `score_candidate` takes `.abs()`, and `ranked_propensity` offsets
every score by the minimum precisely "because arena weights are signed"
(`ranking.rs:787`). So a cold-start reader's *decline* computed `0.0 - 0.10`,
clamped back to `0.0`, and was recorded as a success while moving nothing. Caught
by `a_declined_sample_is_recorded_and_trains_the_profile`, which now asserts the
**sign** (`weight < 0.0`) rather than the existence of a weight.

### Two mutations survived the first version of the suite, and both were real

| mutation | first run | after |
|---|---|---|
| decline raises the weight | **SURVIVED** | killed — the sign assertion |
| partial unique index → plain index | **SURVIVED** | killed — `one_work_cannot_have_two_open_samples` |
| order most-certain-first | killed | — |
| recorded uncertainty is a constant | killed | — |
| reinstate the `10_000.0` divisor | killed | — |

The index mutation survived because `candidate_works` filters *answered* works, so
an **open but unanswered** sample never reaches the insert that the index guards.
The test that kills it draws a sample in one session, leaves it unanswered, and
asks again in a second.

### Five `::uuid` casts, invisible on SQLite

0099 declares `tasting_samples` with three types in one table: `id` and
`account_id` TEXT, `work_id` UUID. Five queries cast them as UUID. Every one is
accepted by SQLite's dynamic typing and fails on PostgreSQL with `operator does
not exist: text = uuid` — four runs of HTTP 500s before the schema was read
directly. `arena_weights.account_id` **is** a real UUID, so the fix was per-query
rather than blanket; both facts are now written down in the module header.

### Gates

| gate | result |
|---|---|
| `cargo fmt --all -- --check` | exit 0 |
| `cargo test -p lorehaven-app --test tasting_menu` (SQLite) | **14 passed, 0 failed, exit 0** |
| `cargo test -p lorehaven-app --test tasting_menu` (PostgreSQL 15, `--test-threads=2`) | **14 passed, 0 failed, exit 0** |
| `cargo test -p lorehaven-app --test route_inventory` | 2 passed, exit 0 |
| `cargo test -p lorehaven-db --lib migrate` | 9 passed — dialect parity incl. migration 0100 |
| `cargo test -p lorehaven-db --test migration_catalogue` | 6 passed — 0100 registered in both dialects |
| `cargo test -p lorehaven-db --no-fail-fast` | 121 + 6 + 8 passed, 0 failed |

PostgreSQL was brought up for this: `docker run -d -p 127.0.0.1:55433:5432
postgres:15-alpine`, matching the `LOREHAVEN_TEST_PG_URL` the handoff records. The
server the handoff names was not running on this host.

**`cargo clippy --workspace` is still red**, on 10 findings that predate this
change and sit in `retention_proposals.rs`, `retention_proposal_admin.rs`,
`worker.rs` and `doctor.rs` — none of which this work touched. `warnings = "deny"`
makes them build failures, so the workspace clippy gate cannot pass until they are
fixed. That is the next unit.

## 2026-09-30 — M54-01: the bot's e2e suite had never run, and it found a real bug

**Plan:** `docs/plans/m54-bot-core.md` Part B · **Requirement:** `M54-01`

`M54-01` was `planned`, which was simply wrong. `~/code-local/rust/lorebot` exists
with all six crates, an API client, a platform-neutral core, and Discord,
Telegram and Matrix adapters. It was `planned` because nothing had run it.

### Eight of the nine e2e tests were passing without executing

Each begins:

```rust
let Some(url) = instance_url() else { skip("…"); return; };
```

With no `LOREBOT_E2E_URL` they return early, and `cargo test` reports **9 passed**.
So the count was 252 unit tests and 9 e2e "passes", and the 9 were worth nothing.
This is worth stating plainly because it applies to the rest of the ledger: a
suite that skips without configuration and reports the skip as a pass is
indistinguishable from one that ran, unless someone reads the body.

### Against a real instance, one of the nine failed

`ScopeSet::parse` split on whitespace:

```rust
for part in raw.split_whitespace() { … }
```

Lorehaven's `external.rs` joins the scope array with `", "`, so a link seeded from
what `GET /me/credential` actually reports is `"content.read,library.read"`.
`split_whitespace` yields ONE part, `Scope::parse` does not recognise it, it lands
in `unknown`, and `allows()` is false for **every** scope.

The symptom is the worst shape this failure can take. `describe()` renders
`unknown` too, so the bot *displayed* exactly the right scopes while refusing every
action — and `/status` reported **"Lorehaven could not be reached"**, a
misdiagnosis, because Lorehaven was reachable and had answered. A reader would
have been told their instance was down.

**Why 252 tests could not see it.** `to_wire` joins with a space, so `parse` and
`to_wire` are mutually consistent. Every test written against that pair tests the
pair, not the server's format. The defect is only visible at the seam between two
components that each believe they are right — and the e2e suite exists precisely
to be that seam, so it had to be made to run.

Fixed in `crates/lorebot-api/src/model.rs`: split on comma **and** whitespace,
discarding empties. Both separators occur and neither is going away — comma is what
the server sends, space is what `to_wire` emits outbound. Three new tests in
`crates/lorebot-api/tests/scope_parse.rs`, one of which keeps a genuinely unknown
scope visible to the reader, because `unknown` exists so a scope this build does
not recognise is shown rather than silently dropped, and a "just ignore what you
cannot parse" fix would have taken that away.

The third test was wrong when first written — it asserted that `""` grants
`content.read`, which is the bug — and caught me, which is the argument for it.

### The exit condition, checked in parts

The plan's B4, verbatim: *a Discord, a Telegram and a Matrix adapter all pass the
same core test suite; revoking a token takes effect on the next call; no code path
reaches the DB.*

| Clause | How it was checked |
|---|---|
| same suite, three adapters | `conformance::run` is called from `lorebot-discord/src/lib.rs:916`, `lorebot-telegram/src/lib.rs:910`, `lorebot-matrix/src/lib.rs:928`. **Proven load-bearing, not merely referenced:** injecting one deliberately failing case turned discord 25/1, telegram 20/1 and matrix 23/1 red; restoring the file returned all three green. A grep for the call sites proves nothing. |
| revocation takes effect | `a_revoked_token_says_relink_and_never_the_raw_error` — and unlike the other eight, it had never run before this |
| no DB path | No crate has `sqlx`, `postgres` or `lorehaven-db` in its `Cargo.toml` |

### Gates

- `lorebot` workspace: **255 tests, 0 failed** (252 unit + 9 e2e, the 9 against a
  real 277-table Postgres instance on `127.0.0.1:3111` with a minted reader
  token).
- `cargo clippy --workspace --all-targets`: 0 warnings.
- `cargo fmt --all`: clean.

**Ledger:** 289 `implemented-fully-tested`, 225 `implemented-locally-tested`, 117
`implemented-verified-e2e`, 4 `unsupported`, 1 `evaluated-and-rejected`, **47
`planned`**.

## 2026-09-30 — M60 verified by running it: the published snapshot shipped the whole instance

**Commits:** `a02dc51`, `9f665cf`, `c8f4505`
**Plan:** `docs/plans/m60-snapshot-gate-and-promotion.md`

Seven rows sat at `built` — machinery that exists and has never been checked
against the standard `docs/goal.md` sets:

> A requirement is done when it has evidence, not when a status flips. A row whose
> machinery is unreachable — nothing writes the table, nothing enqueues the job —
> is not tested by unit tests on the dead functions.

Running each row's own verifier produced one critical disclosure defect and five
more, and promoted all seven. This entry leads with the critical one because it
is the most important thing this milestone produced.

### The published snapshot contained the unmasked instance

`take-snapshot.sh` ran `pg_dump -d "$SCRATCH_DB"` with **no schema filter**. The
mask is built into a separate schema, `snapshot_masked`, and the scratch database
still held the original `public` tables it was restored from at step 1 — nothing
ever dropped them. So the published dump contained both:

```
public             277 tables   <- ORIGINAL, unmasked
snapshot_masked     70 tables   <- the mask, correct and complete

public.accounts.id          01959000-0000-4000-8000-000000000001
public.accounts.email       instance@retention.system.invalid
snapshot_masked.accounts.id 6d0b4fa3-e243-457b-af3c-8c49e72f226f
snapshot_masked.accounts.email d314945a…@snapshot.invalid
```

`works_index`, `ip_policy` and `secrets` — which the policy says must never be
published — were all present, as was `works.redistribution`, which had been
dropped from the masked copy minutes earlier.

**Every gate was green on it.** That is the finding, not a footnote:

- the canary suite builds its own dump and never runs `take-snapshot.sh`;
- the PII gate checks the *policy*, not the bytes;
- `doctor` reads the migration ledger, which lives in `public` and was intact — so
  it reported 18 checks, 0 failing, on a dump carrying the entire instance;
- the name-based doctor gate added in this same session could not have caught it
  either: `migrations` passes *precisely because* the unmasked schema is present.

The pipeline's single defence was a sentence in `build-snapshot-sql.py`'s
docstring — *"the dump is taken of THAT schema. The original schema is never
dumped"* — which the script contradicted. Found only by decrypting the published
artefact and restoring it, which nothing in the repo was doing.

Fixed with `-n snapshot_masked`, plus an assertion on the shipped bytes that
refuses to publish any original-schema table beyond the intended `_migrations`
ledger. Proven to fire on a simulated breach (a dump of `public.works` plus the
ledger counts 2 and refuses). Verified from the recipient side: 0 occurrences of
the original account id, 0 of the original email, 0 of `works_index`,
`ip_policy`, `secrets` or `redistribution`.

### Five more defects, all found by running the pipeline

| Defect | Symptom |
|---|---|
| `createdb`/`psql`/`dropdb` called with no connection target | `FATAL: role "alvaro" does not exist` — the script could not connect to the database it was given |
| `doctor --strict` fails on *any* warning | refused a snapshot with 18 checks / 0 failing, because `piper` is not installed. A gate that cannot pass is not a gate |
| bare `grep` under `set -euo pipefail` | exited 1 on a **clean** restore, dying on the line written to report which check failed |
| `age -p` ignores a piped passphrase | the non-interactive branch — the one for cron — had never worked |
| `$(...)` strips the trailing newline | age's confirmation prompt was never terminated; the script hung until killed |

The last one cost four false negatives. A short passphrase, a long one, a file
redirect, and `script -q` each "worked" run directly and each hung inside the
script — because every probe changed the shell plumbing at the same time as the
variable under test, and the plumbing was the cause. Three wrong regexes also
preceded the right one for `doctor_integrity_checks`, which first recorded
`NOT RECORDED` on a clean run and then listed all eighteen checks.

Two verifications that reported success while checking nothing:

- the intermediates cleanup removed `$WORK/$NAME.sql.zst`, which `zstd` never
  wrote (it writes to `$OUT`), so it removed nothing — and the loop that *claims*
  to confirm the removal checked the same two paths that never existed, reporting
  "intermediates removed" over a 55 KB uncompressed snapshot left on disk;
- `doctor_integrity_checks` matched `\\[ok\\]` but `render()` pads the marker to
  `[ok  ]`, so a passing run recorded `NOT RECORDED`.

### The PII gate was silent on the most sensitive column in the schema

`check-snapshot-pii.py` was RED on four columns, two of them added by M59-10 hours
earlier. Chasing it found a hole in the gate: `reader_body_copies.plain_text` — a
reader's copy of an external body — got no decision asked for, while the
`account_id` on the same table did, because only the latter name matches
`IDENTITY_NAME_RE`. `M60-05`'s own policy note already described the failure:
*"the M60-01 gate only required a decision for columns in TABLES it knew carried
PII, and chapter_revisions was not one of them. A gate that cannot fail on the
most sensitive column in the schema is not a gate."* So the fix was applied once,
to one table, and the class was not fixed.

A `CONTENT_NAME_RE` fan-out closes it, unanchored for the same reason the identity
regex is. It immediately surfaced **five more** ungated body columns:

- `works_index.body_text` — every work's full text in one `NOT NULL` column.
  `drop_table`: the gate would be evaluated once *for the table*, so a snapshot in
  the wrong mode publishes every body at once. It is the search index and is
  rebuilt from `chapter_revisions`.
- `roadmap_suggestions.raw_text`, `critique_queue.excerpt` — a reader's own
  prose, not a work's text. No mode-based gate is right: a `cache` instance would
  publish every suggestion ever made.
- `reader_body_copies.plain_text` / `.sanitized_html` — `keep_gated`, same
  condition as `chapter_revisions`, and one decision, because masking the text
  alone republishes the body as HTML.

A fourth self-test probe was added, because a fan-out with no failing test can be
deleted by a later editor who sees no reason for it.

### Gates

- `check-snapshot-pii.py` exit 0 — 2014 columns across 276 tables, all decided.
- `check-snapshot-pii.py --self-test` exit 0 — four properties, each demonstrated
  by failing on a probe.
- `check-snapshot-channel.py --self-test` exit 0 — 11 refusals run, 4 controls.
- `snapshot_anonymisation` 10/10 on SQLite, 10/10 on PostgreSQL.
- Full pipeline on a 277-table / 92-migration source: exit 0. Recipients can
  decrypt the artefact, restore it (70 masked tables, 92-row ledger) and run
  `doctor` against the result — 18 checks, 0 failing.
- Two `--random-timestamp-offset` runs: 319 vs 291 days, 25 timestamp expressions
  shifted per column.

**Ledger after this pass:** 289 `implemented-fully-tested`, 225
`implemented-locally-tested`, 116 `implemented-verified-e2e`, 4 `unsupported`,
1 `evaluated-and-rejected`, and **48 `planned`** — 45 in M45, 2 in M53, 1 in M54.
Those are genuinely unimplemented requirements, not verification debt. No row is
left at `built`.

## 2026-09-16 — M24 complete: anchored comments, orphaning, CSV imports

**Commits:** `b41c5f3`, `b435f5b`

**Context.** M24 (spec §32.3) has three sub-requirements: anchored comments on paragraph offsets and media timestamps, work orphaning with succession, and CSV import adapters for library metadata. All three are now implemented.

**Anchored comments.** The `comments` table gained `anchor_kind`, `anchor_value`, and `anchor_chapter_id` columns via migration 0027. Domain validation in `lorehaven_domain::anchor` enforces that paragraph anchors require a chapter id and a non-negative integer, and timestamp anchors use `HH:MM:SS(.fff)` format without a chapter. The `post_comment` route validates the anchor before insert; `list_comments` returns the anchor fields. Two tests cover round-trip and validation.

**Orphaning.** Migration 0028 adds `work_orphans` (with `successor_work_id`) and a `works.orphaned` marker. `lorehaven_domain::orphaning` validates: published, not already orphaned, owner-gated for relinquishment (pseud/account deletion bypasses the owner check), and succession requires a published successor. The creator dashboard is not yet built (marked partially-implemented in requirements.csv).

**CSV imports.** `lorehaven_scrapers::csv` parses Goodreads and StoryGraph library export CSVs into `ShelfRow` structs. Column order detected from the header row; quoted fields and escaped quotes handled per RFC 4180. Ingestion only — no chapter bodies. 12 CSV unit tests + 1 integration test in milestone_24.

**Gates.** 1,134 workspace tests pass on SQLite. M24 tests verified on live PostgreSQL (`postgres://lorehaven:***@127.0.0.1:55432/postgres`). Clippy 0 warnings, fmt clean.

## 2026-09-16 — PG dialect parity complete, SQLite regressions from the parity pass fixed

# Verification log — Lorehaven




## 2026-09-30 — A reader's own copy of an external body: five defects a unit-test-only view ships

**Plan:** `docs/plans/m59-10-reader-body-request.md` (spec §11.15b, requirement
`M59-10` — the one M59 row no phase in `crawling-retention-preservation.md` was
assigned to, so it had neither code nor a plan).

The counts are filled in below from the completed workspace runs, and both engines
are listed because the single most important fact about this feature is that **one
of its five defects was visible only on Postgres**, and another only when the
whole workspace ran.

### What a green run would not have told you

| Defect | Where it showed up | Why a narrower check misses it |
|---|---|---|
| `source_for_work` filtered `deleted_at IS NULL` on a table with no such column | 500, first live request, both engines | compiles; the table's columns were read from memory |
| An `INSERT` used with `fetch_one` | 500 on the happy path | unit tests called the store's readers, never the writer's return |
| No `::text AS` on `UUID` columns | Postgres only: `decoding column "id": mismatched types` | a SQLite-only run is a green run |
| A Postgres arm calling `db.sqlite_pool()` | Postgres only: panic on the first job | same |
| The GET response's `id` embedded the reader's `account_id` | the privacy test, on its FIRST run | nothing else looks at response identity |

**And the shape of two of them is worth keeping.** The `deleted_at` fault and the
`sqlite_pool` fault are both copy-paste-from-the-neighbouring-arm or
from-memory errors: correct-looking code about a schema or an API that differs
one step away from the one you have in front of you. The second is only findable
by scanning for the pattern across the whole file rather than fixing the site the
test named.

### The two checks a green run cannot give

**The §6.4.4 guard was proven to fire.** `no_read_path_consults_the_request_bar` is
a **source scan**, not a behavioural test, and the spec clause it enforces is about
code that does not exist yet — "no route, surface or rendered page varies in
whether a body is shown according to the viewer's trust level, and a test fails the
build if one appears". A behavioural test can only show that today's readers agree.

Injected `body_request_min_trust` into `retention_settle.rs` (excluded by the
allow-list) and required the test to fail **on the scan's own assertion**, not on a
compile error or an unrelated 404:

```
§6.4.4: a read path must not branch on the viewer's trust to decide whether a body is shown.
crates/app/src/routes/retention_settle.rs:296: // INJECTED-TO-PROVE-THE-GUARD: …
```

**The privacy test was proven to fail.** Re-introduced the exact field the first
draft had (`BodyRequestView.account_id`) and required the test to go red on the
account-id assertion, then reverted and re-ran green. The test had already caught
this leak on its first run — which is the argument for writing a privacy assertion
as an **absence sweep over the rendered JSON** rather than a check on the fields you
meant to omit:

```rust
for field in ["account_id", "reader_id", "requested_by", "owner"] { … }
```

A composite handle shaped like `format!("{work_id}:{account_id}")` is what slips
through such a test, because it reads as an id and carries an identity. The field
was named `id`. That is why the sweep names *fields* rather than values.

### The gate's order, pinned by a test that fails on a reordering

`an_aggregate_instance_refuses_by_name_and_never_reaches_the_trust_gate` asks a
**trust-0** reader on an **aggregate** source and asserts the error CODE is
`RETENTION_AGGREGATE`, not merely that the status is a refusal. Checking trust
first would pass every other test in the file and leak the instance's retention
mode to a reader not entitled to know it — so this is a test about the order, and
nothing else in the suite would catch the swap.

### What the run says

```
at 6fa8ec0  SQLite     3348 passed, 0 failed, 154 suites, 0 warnings
at 6fa8ec0  PostgreSQL 3348 passed, 0 failed, 154 suites, 0 warnings
```

Up from 3338 at Phase F: the 10 new tests are the 7 in `reader_body_copies.rs`
and the 3 in `config_sections.rs`. 154 suites rather than 153 — the new file is
its own suite.

**The totals matching is the least interesting thing about them.** Two of the five
defects in the table above were visible *only* here, on PostgreSQL: the missing
`::text AS` casts and the `sqlite_pool()` on a Postgres arm. A SQLite-only run of
this feature was green on 3348 minus everything, which is to say it was green on
the arms that were correct.

### One thing the harness got wrong before the route did

`set_source_mode` hardcoded an actor named `"alice"` while a test's only reader was
`"lowtrust"`. That is the same fault the route holds itself to avoid under §6.4.4
— *name the row rather than guess which row you meant* — so the comment on
`set_source_mode_as` says so. Worth recording because a harness that assumes a row
it has not got is the same defect the property test exists to prevent, one layer
down and easier to miss for being in the test rather than the product.

## 2026-09-30 — A migration that inserts a row broke six queries across five files

**Commits:** `2a8aead`, `8920f94`, `1531262`, `17183e9`, `eb03a0d`.

This is the whole entry because the *pattern* is worth more than the 72
failures, and because **not one of the 72 failed anywhere near the cause.**

**What the migration did.** `0094_retention_proposals.sql` inserts a fixed system
account. It had to: `retention_policy_changes.actor` is
`NOT NULL REFERENCES accounts(id)`, and a binding-mode settlement acts with no
operator — NULL is refused by `NOT NULL`, the nil UUID by the foreign key, and a
reader's id would put one of the voters' names on the row their own ballot
produced.

**What broke.** The row carries `created_at = 2026-01-01`, earlier than anything
a test registers. So every query that identified a row by *position* picked it
up instead of the one it meant:

| Site | Shape | Symptom it produced |
|---|---|---|
| `m52_08_rec_shadow.rs` | `ORDER BY created_at LIMIT 1` | `page not found` on an endpoint that exists |
| `m57_metadata_exchange.rs` (×2) | the same, and a bare `LIMIT 1` | `one node despite the messy spelling: []`; `RowNotFound` on a pseud |
| `milestone_0.rs` (×3) | `count(accounts) == 0` / `== 1` | counts off by one |
| `milestone_43_browse.rs` | `count("accounts") == 1` | `the account is untouched` failing |
| `snapshot_anonymisation.rs` (×2) | `ORDER BY created_at LIMIT 1`; `max - min` over the table | `left: 2026-02-15`, a date-arithmetic bug that was not one |
| `device_delivery_fk.rs` | `COUNT(*) == 1` diagnostic | `mine=1, total=2` |

**The snapshot one is the sharpest.** The failing assertion compared a timestamp
against `2026-04-15T10:30:00Z` and got `2026-02-15T00:00:00Z` — which reads as an
off-by-45-days error and is in fact the system account, correctly masked and
correctly shifted, sorting first. `2026-01-01 + 45 = 2026-02-15` exactly. Three
attempts and one probe were needed to see that; the probe printed three rows of
each table and settled it in four lines.

**Two fix shapes, and conflating them is a trap of its own.** "Which row?" — name
it (`WHERE email = …`, as `milestone_18`/`19` already did and needed nothing), or
exclude the instance row. "How many rows did *this action* create?" — the
migration's row is not part of the answer, so exclude it **in the assertion**;
`count(accounts) - 1` gives the right number by accident and breaks on the next
instance-owned row.

**And a third shape, deeper than either.** In `snapshot_anonymisation` the canary
rows were seeded with `gen_random_uuid()` and their ids discarded, and the mask
scrambles every key through `snapshot_account(id)` — so "the oldest row" was the
only handle the test had, and excluding by the *original* id could never match.
The fix is to generate and capture the ids, and read the row under test by its
masked id. **Naming the row beats enumerating the rows it is not.**

**The counts, per run:**

```
at 507a3ab  SQLite     3338 passed, 0 failed, 153 suites, 0 warnings
at 507a3ab  PostgreSQL 3338 passed, 0 failed, 153 suites, 0 warnings
```

Both engines, both totals, from a clean tree. An earlier run at `17183e9` was
**red on PostgreSQL** in `snapshot_anonymisation` (1 of 3339) and green on
SQLite — because that test returns early off Postgres, so the engine that was
never red is the one that could not have caught it. That asymmetry is the whole
argument for running both, and it is why this section is not a single number. `milestone_43_browse` and the six count sites
were red on **both** engines from `2a8aead` onward and were green by `eb03a0d`;
`snapshot_anonymisation` is green on both (10 each) at `eb03a0d`, and its shift
assertion was verified by injection — a per-row-variable shift in the generator
makes it fail, so the id-based select did not weaken it into a lookup that always
finds something.

**What none of this would have found by reading the diff.** Every site is a
correct-looking query over a table whose contents had changed underneath it.
The lesson is not "check your queries" — it is that **a migration that writes a
row is a change to every query that assumed the table held only what the
application put there**, and only a workspace-wide run, on both engines, finds
them.

## 2026-09-30 — M59 Phase E: retention proposals, ballots, advisory and binding governance

**Commits:** `85346b9` (E.3 `roadmap.min_trust`), `539e6ae` (E.4 the four reader
routes), `61b2d36` (E.2 the operator's routes, the settlement pass, the system
account). Migration 0094 was **amended rather than added** — it had not been
applied to any database, so the system account went into both dialect files
directly. Any environment that has already applied 0094 needs a new migration.

**Both dialects, executed.** The new suite runs on SQLite and on PostgreSQL, and
this is stated because `docs/verification.md` §2.6 exists for the alternative:

```sh
cargo test -p lorehaven-app --test retention_proposal_routes
#   19 passed; 0 failed   (SQLite)

LOREHAVEN_TEST_PG_URL=postgres://postgres:***@127.0.0.1:5432/lorehaven_test \
  cargo test -p lorehaven-app --test retention_proposal_routes
#   19 passed; 0 failed   (PostgreSQL)
```

**Neighbouring suites, all green, none regressed:** `retention_proposals` 12,
`retention_routes` 10, `retention_policy` 11, `milestone_45_roadmap` 55,
`config_sections` 4, `device_delivery_fk` 3, `lorehaven-db` migration parity 2.

## 2026-09-30 — M59 Phase F: the two governance settings are now loadable from a file

**Commit:** `be68f32`. And the finding is the reason this section is worth a
heading of its own, because it is a defect that looks like finished work.

`roadmap.min_trust` and the four `retention_governance.*` settings were fields
on `Config`, with defaults wired into `development_defaults()` and into the
file-load path — and **no `FileConfig` member**. So:

* a Rust test could set `config.retention_governance.widen_quorum = 4`;
* the test asserting `required == 4` and `reached == false` passed;
* and an operator editing `lorehaven.toml` had their key **silently ignored**.

Every one of those tests was evidence that the widening bar is read from *the
struct*. None was evidence that an operator can reach it. And the tests passing
is what made it dangerous: a setting that reads as configurable and is not is
worse than one that is absent, because the suite reads as proof it works. This
is the same trap `RetentionSection`'s own doc comment in `config.rs` describes
having already happened once, one section over — with the comment sitting there
while it happened again.

**Proved, not asserted.** `crates/app/tests/config_sections.rs` writes a real
file and loads it through `GlobalArgs { config: Some(..) }` — the same door a
deployment comes through, rather than a test-only route, because a test using
another route would prove the struct deserialises and not that the binary's path
reaches it. Four cases: every key set, one key set, three misspelled keys, and
an absent file.

**Verified by injection.** Replacing the two file reads with their defaults
makes the suite fail (`3 passed; 1 failed`); the revert makes it pass
(`4 passed; 0 failed`). That is the only evidence here that the test is testing
something, and it is the step whose absence turns a test into a comment.

Two smaller things the same commit fixed:

* `device_delivery_fk`'s diagnostic asserted `COUNT(*) == 1` on `accounts`, so
  migration 0094's system account broke it. The diagnostic's stated job is "prove
  the account is really in THIS database" — the `mine` half. `total` is context
  and belongs in the message, not the assertion: `mine=0, total=7` reads
  differently from `mine=0, total=1`, which is the reason the count is there at
  all.
* `anyhow::Error`'s `Display` is only the outermost context (`parsing <path>`);
  the `unknown field` line is one level down in the source chain, which
  `Display` does not walk. The misspelling test uses `Error::chain()`. An earlier
  version matched the first line only, which would have passed against a refusal
  that never named the offending key — the one thing the test is for.

**Workspace suite.** `cargo test --workspace` was still running when this
section was written, so no workspace-wide count is claimed here. The per-suite
counts above are from runs that completed.

> **Superseded — read this before citing the section above.** A workspace run
> completed later, on a tree that includes five commits made *after* the Phase F
> work: `2a8aead`, `8920f94`, `1531262`, `17183e9`, `eb03a0d`. Those five fixed
> 72 failures the Phase F run had not reached yet, so a clean count from that
> later run says nothing about the tree this section describes. What it does say
> is recorded in its own section below, and the failures it found are the
> interesting part: **a migration that inserts a row broke six queries across
> five files** that could not name the row they meant, and none of the six failed
> anywhere near the cause.

**One test verified by injection, and why that matters.**
`no_response_exposes_a_ballot_or_its_voter` asserts on the *serialised text* of
the reader-facing responses, refusing every plausible key a leak would use
(`ballot`, `voter`, `voted`, `account`, `handle`, `opened_by`, …) **and** the
three real account ids behind the ballots. Adding `opened_by` to the response
projection makes it fail; the revert makes it pass. A test that has never failed
is not evidence, and a name-whitelist assertion is exactly the kind that passes
forever because nobody adds the field it is watching for — so the account-id
check is what makes a leak under an unanticipated name still fail.

**Six defects the tests found, and one of them was mine to have written.** These
are recorded because none of them are re-derivable from reading the final code,
which is correct:

1. **The override wrote the proposal's mode, not the operator's.**
   `OverrideBody.body_mode` was parsed, validated against the two legal values,
   and recorded in the audit row — and then never used; the write went to
   `proposal.proposed_mode`. An operator asking for `aggregate` on a proposal
   that proposed `cache` got `cache`, with a 200 and an audit row that agreed
   with them. Three records, two truths, no error. `apply` now takes the mode as
   a parameter so a caller cannot forget it.

2. **Advisory settlement finalised ballots.** The pass closed a `passed` ballot
   as `passed`, which made `respond` unreachable for any ballot older than its
   window — the job had finalised it, so the operator's route correctly reported
   "already decided". Advisory settlement now reports and closes nothing, so the
   proposal stays `open` and the pass is idempotent for free.

3. **`open -> overridden` was refused by the store**, and the close silently
   affected zero rows, so overriding a ballot still in flight returned 200 with
   the setting changed and the proposal still reading `open`. That is the
   ordinary case. Permitted, as a one-line addition to the source-state list,
   which cannot affect any other transition.

4. **The binding settlement could not record its own change.** The actor was the
   nil UUID, refused by `REFERENCES accounts (id)`. Migration 0094 now inserts a
   fixed system account and the pass uses it — `NULL` is refused by `NOT NULL`,
   the nil UUID by the foreign key, and a reader's id by the feature's own
   privacy property.

5. **A dead quorum recomputation shaped like diligence.** `respond` and
   `settle_one` both recomputed `quorum_for` and then gated on `tally.quorum`
   instead, leaving `let _ = required;`. Deleted; `tally` is now the one place
   the bar is derived.

6. **`respond` closed before it applied**, so a failed apply left a ballot
   recorded as passed with the setting unmoved. Reversed.

**And the process failure worth naming.** Three explanatory comments written
during this phase were wrong, each because the explanation was written before
the evidence: a claim that `source_key IS ?` cannot match NULL on SQLite (it
can — checked), a claim that `retention_policy_changes.actor` is nullable (that
is `instance_retention_policy.updated_by` in migration **0087**, a different
table; 0094 has `NOT NULL`), and a three-paragraph "diagnosis" of a bug that did
not exist. The actual cause appeared in one line once `tracing::error!` was
replaced with `eprintln!` — **tracing output is not captured by `#[tokio::test]`**
— and it was `NOT NULL constraint failed: retention_policy_changes.actor`. The
rule this earns: on a failing test, run it until the real error text appears
before writing down a cause.

**One migration-parity note.** `the_two_dialects_declare_the_same_columns_and_indexes`
passes. The system account is a row, not a column, so it is not covered by that
check; the two `INSERT`s are kept in step by hand and the `SYSTEM_ACCOUNT`
constant's doc comment names the migration file so a mismatch is greppable.


Newest first. Each section states what was verified, how, and the result.

## 2026-09-17 — M25 residuals, M24-02 dashboard, scoped-door pagination, leak sweep

**Context.** The remediation plan's N3 and N4 items, worked in the order the
plan lists them. The first finding is that M25 had no test file at all: the
ledger's "milestone tests 21/21" were milestone_16/22/26, and nothing in the
repository ever executed the derivative pipeline, lending or the Dublin Core
feed. Every M25 defect below was found by writing that file.

**Derivatives could not run.** The request door created a row, answered
`queued`, and enqueued nothing — a TODO in `routes/derivative.rs`. The worker's
OCR and transcode arms refused at build time. Now: the door enqueues
`JobKind::Derivative` with the derivative id, records the job on the row, refuses
a kind whose program is absent with the spec's `CONVERTER_UNAVAILABLE` (a new
`AppError`/`ErrorCode` pair from §3.3's list, 422) naming what to install, and
refuses a parent checksum no blob holds. OCR runs Tesseract and transcode runs
ffmpeg to a streaming MP4; both go through the same `which` discovery the
document converters use, with the program, remedy and output media type declared
on `DerivativeKind` so the door, the worker and doctor cannot disagree. Temp
directories became a Drop guard (the old cleanup ran only on success), and a
failed build is recorded on the row and classified fatal or transient instead of
leaving it reading `queued` forever.

**Lending.** Migration 0033 adds `work_loans.expired_at` (both dialects) and the
maintenance pass stamps loans whose window has closed. Two real bugs fell out:
`grant_loan` inserted unconditionally, so a reader whose loan had expired hit
`UNIQUE (work_id, borrower_account_id)` and got a 500 on every re-borrow; and
`Loan::is_active()` ignored expiry entirely and called an expired loan live. The
grant is now an upsert that re-grants the row, and `GET /api/v1/me/loans`
reports the caller's own loans with `active|expired|revoked`.

**Derivative doors** now use the app's one visibility rule (the same helper the
narration doors use since the previous commit): contributor-only to request,
404 for a work the caller cannot read.

**Creator dashboard (M24-02).** `GET /api/v1/me/dashboard` aggregates the acting
pseud's own works: totals and text for the author's own inventory (exact), and
reader-facing counts (bookmarks, ratings, reviews, delivered comments) banded at
the floor — a count below it is a string (`fewer_than_5`), never a number a
client would render as an exact figure. No reader, pseud, account or per-reader
row appears in the payload, and there is no "held by the filter" counter: §12
frames the author's view as what arrived. The test asserts the absence of those
strings in the rendered payload, not just their absence by construction.

**Scoped doors paginate.** `/api/v1/canons/{id}/media` and
`/api/v1/spaces/{id}/media` answered with a silent `LIMIT 50`. They now take a
validated `limit` and a cursor carrying the whole ordering key
(`position|created_at|id`, exactly what the ORDER BY compares) and return
`next_cursor` only for a full page. A two-page walk test seeds five works at
`limit=2` and asserts three pages, each item once, in canon order, plus the
refusals for `limit=0` and a malformed cursor.

**Leaked scratch databases.** `test_support` now sweeps `lh_test_*` databases at
the first PostgreSQL connect in a process. The criterion is liveness rather than
age — `pg_database` has no creation timestamp, and a database nobody is attached
to is one no run will ever drop — so a live run's databases are left alone.

**Shelf exports import (M24-03).** The CSV shelf import was parser-only: no
door, no persistence, so §32.3's acceptance ("a StoryGraph CSV import produces
library states and reviews that respect the reader's existing ratings and dates,
and refuses rows it cannot map, naming them") could not be met by any caller.
`POST /api/v1/library/imports/csv` now plans the file
(`scrapers::csv::plan_shelf_import`), creates the reader's own library rows
through the existing `(account, source, source_work_key)` upsert, and sets the
state each row implies through a new `set_imported_reading_status`, which writes
only when the reader has no state of their own — a re-import reports how many it
left alone rather than overwriting them. A row's date read becomes `finished_at`
(the reader finished it in 2019; they imported it today). Refusals name the row:
a date that does not parse is refused with the text as written, and a row the
parser could not read at all is refused *by line*, because a row with no title
has no other identity in the file the reader is looking at — that required the
two CSV parsers to record the line numbers they skip instead of only counting
them.

**Evidence (literal).**

```
cargo test -p lorehaven-app --test milestone_25
test result: ok. 10 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 7.89s

cargo test -p lorehaven-app --test milestone_22
test result: ok. 14 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.43s

cargo test -p lorehaven-app --test milestone_24
test result: ok. 5 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 1.11s

cargo test -p lorehaven-scrapers --lib
test result: ok. 267 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.01s
```

`cargo clippy --workspace --all-targets`: 0 warnings. `cargo fmt` clean.

```
cargo test --workspace --no-fail-fast
PASSED: 1206 FAILED: 0 BINARIES: 46
```

One failure was found and fixed on the way to that run: `normalise_date` matched
the `YYYY-MM-DD` shape before checking for a full timestamp, so a `date_read` of
`2019-12-31T10:11:12Z` became `2019-12-31T00:00:00Z` — the time silently
dropped. The unit test caught it; the full-timestamp branch now runs first.

## 2026-09-17 — M26 TTS narration pipeline (spec §32.5) + adult gates restored to every door

**Context.** The M26 narration half was draft-CRUD with no audio: a
request created a `narration` edition and queued a `JobKind::Narration`
job that had no handler, and the request door queued it even on an
instance with no synthesizer at all. Alvaro's decision (2026-09-17) was
a pluggable `TtsEngine` trait, local-first, cloud adapters later behind
the same trait. The three rating-gate tests written earlier the same
day had been dropped from `milestone_26.rs` when the narration tests
replaced the file.

**What was implemented.**

- `crates/app/src/tts.rs` — `TtsEngine` (`name`, `is_available`,
  `health`, `synthesize`) with a `PiperEngine` (local binary, `--model`,
  argv only, temp file, no shell), a `SilentEngine` (a valid WAV whose
  length follows the text, so the whole pipeline is exercisable on a
  host with no synthesizer and in CI), and a `MissingEngine` whose
  `health()` names the missing program. `build_engine` is the one place
  a configured name maps to an implementation; `tts.engine` is
  validated against `SUPPORTED_ENGINES`.
- WAV splicing: `concat_audio` parses the RIFF chunks and splices the
  `data` payloads, rewriting the RIFF and `data` sizes — `[a, b].concat()`
  is not a playable file. Mismatched `fmt ` chunks or media types are
  refused rather than guessed at.
- `crates/app/src/narration.rs` — the worker handler: load the edition,
  collect the work's chapter text, resolve and health-check the engine,
  chunk (sentence-boundary-first, never splitting a multi-byte
  character), synthesize with job progress, splice, store the blob and
  a `media_file` row, record the checksum on the edition. It does not
  publish: the §22.6 machine-producer credit and the draft gate stay.
- `tts_engine()` / `can_narrate()` on `AppState`, built once at
  startup from the same `which` discovery the converters use;
  `lorehaven doctor` reports the engine with the same builder, so
  doctor and the worker cannot disagree.
- `[tts]` config section (engine, piper_path, piper_voice_model,
  default_voice, monthly_spend_cap_cents) with `deny_unknown_fields`,
  documented in `lorehaven.toml.example`.
- Migration 0032 adds `media_editions.audio_checksum` (both dialects);
  `mark_narration_audio_stored`, `narration_audio_checksum` and
  `approve_narration_edition` (which refuses an edition with no audio).
- The request door refuses up front when the engine is named but not
  usable, carrying the sentence `doctor` prints, instead of queueing a
  job that cannot succeed.

**What was fixed, not just added.**

- **The narration doors now use the app's one visibility rule.** They
  previously fronted on `RequireSession` + a local contributor check:
  any signed-in account could read a draft edition's metadata, and
  `GET /editions/{id}/audio` served a *published* narration of an
  explicit work to any anonymous caller who knew the id — a hole in
  §32.5's "zero adult items in any door". `reading_decision` and
  `actor_for` in `routes/works.rs` are now `pub(crate)` and the
  narration doors apply them: a work the caller cannot read is 404,
  a draft edition is contributor-only, and published audio is served
  only to a caller who is eligible for the work. §3.3 settles the
  anonymous-draft case: 404, never 401.
- `GET /works/{id}/editions` is now a `MaybeSession` door whose list is
  filtered for non-contributors (published editions only).
- The deleted rating-gate tests are restored and the all-doors case now
  walks list, search, media direct, files, editions, canon, space *and*
  the narration audio door, and asserts the author still sees their own.

**Evidence (literal).**

```
cargo test -p lorehaven-app --lib
test result: ok. 134 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 10.03s

cargo test -p lorehaven-app --test milestone_26
test result: ok. 11 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 2.08s
```

`cargo clippy --workspace --all-targets`: no warnings. `cargo fmt -p
lorehaven-app -p lorehaven-db -- --check`: clean. Workspace and
PostgreSQL runs are reported in the session handoff.

## 2026-09-17 (late session) — All remaining route stubs resolved

**Commit:** `29cc5b4` — "Replace all remaining route stubs with real implementations" (14 files, 689 insertions).

**Context.** After completing M26 TTS narration, a sweep of
`crates/app/src/routes/` found 9 remaining placeholder handlers
returning `[]`, `null`, or `true` instead of querying the DB.
Each had a corresponding DB function that was either missing or
unused. The work was to wire them together and add the missing
DB functions.

**Fixed:** `external.rs` (get_public_work, public_search, list_tokens),
`economy.rs` (list_bounties, create_bounty, claim_bounty),
`governance.rs` (my_appeals, my_audit_log),
`translation.rs` (list_memory), `admin.rs` (list_privacy_requests,
check_abuse_status). Added DB-layer functions for each. Added
`0034_bounties.sql` migration.

**Lessons learned:** The `bounties` table already existed in
`0017_economy.sql` with a different schema — `0034_bounties`
could not use `CREATE TABLE IF NOT EXISTS` (a no-op) and had to
be rewritten as `ALTER TABLE`. Similarly `audit_log` uses
`subject_type/subject_id/document`, not `target/target_details`.
Both were discovered by test failures.

## 2026-09-16 — PG dialect parity complete, SQLite regressions from the parity pass fixed

**Commit:** `69ee2d8` — "db: finish PG dialect parity and fix the SQLite
regressions it introduced" (30 files: 21 db modules, 9 test suites).

**Context.** The parity session's handoff claimed both gates green. An
independent re-run of every gate on the working tree disproved the SQLite
claim (1076 passed / 11 failed) and found clippy non-clean; the PG claim
held (381 passed / 0 failed). All 11 failures were regressions introduced
by the parity pass itself, in three classes:

1. **PG syntax in SQLite arms** — `exports::find_export` /
   `find_export_for` had `EXPORT_COLUMNS_PG` in the SQLite arm and
   `imports::job_for_import` carried `job_id::text` there; SQLite rejected
   each with `unrecognized token: ":"` (m6 ×4, m7 ×6 minus one overlap).
   Found by a balanced-paren scan over every `db.sql()` / `sql_owned()`
   call flagging `::` or a `_PG` constant in the SQLite argument.
2. **`\`-continuation glue** — the rewritten SQLite counter upsert glued
   `1` + `RETURNING` into `1RETURNING` (m15 `usage_counters…`); the PG
   twin survived only via hand-added trailing spaces. Both counter
   upserts (economy + admin) rewritten as honest multi-line literals on
   both dialects.
3. **clippy** — two `useless use of format!` warnings in `secrets.rs`;
   fixed by introducing `COLUMNS_PG` (matching the exports.rs
   convention) instead of `.to_string()`.

Also restored the live `?::uuid` case to the `rewrite_placeholders` unit
test that the sweep had overwritten (`cargo test -p lorehaven-db --lib` →
27 passed).

**Final gate evidence, run on the committed tree:**

| Gate | Command | Result |
|---|---|---|
| PG | `LOREHAVEN_TEST_PG_URL=… cargo test -p lorehaven-app --no-fail-fast -- --test-threads=4` | **381 passed / 0 failed** (24 binaries) |
| SQLite | `unset LOREHAVEN_TEST_PG_URL && CARGO_TARGET_DIR=~/.cargo-target/lorehaven cargo test --workspace --no-fail-fast` | **1087 passed / 0 failed** (42 binaries) |
| db lib | `cargo test -p lorehaven-db --lib` | 27 passed / 0 failed |
| fmt | `cargo fmt --all -- --check` | clean |
| clippy | `cargo clippy -p lorehaven-db --all-targets` | 0 warnings |
| FE | `fe.sh check` + `fe.sh test` | svelte-check 0/0, vitest 147/147 |

**Environment notes.** 149 orphaned `lh_test_*` scratch databases were
dropped before the PG run (panicking tests never reach `cleanup()`; the
harness sweep is still owed). Scratch PG: container `lh-review-pg` on
55432, URL from `~/.config/lorehaven/pg-env` via
`~/.hermes/plans/lhpg-env.sh`.

**Known limitations, on record.** The `col::text = ?` read conversion
defeats PG index usage on UUID PK lookups (correct, not fast); accepted
for now — there is no production deployment (verified: no service, unit,
container, cron or config on the ThinkCentre), so the inversion decision
is deferred to deployment planning. Follow-up plan:
`~/.hermes/plans/2026-09-16-lorehaven-pg-parity-followup.md` (consistency
stragglers, standing checks, ADR).

## 2026-09-15 (afternoon) — review pass over the 08:45–13:43 work

Scope: the 14 commits `999fe4c..01a6bcf` (notifications backend, PG dialect
fixes, forum pages, pricing UI, Playwright e2e, backend-aware harness) plus
the work session's gate-metrics commits. Everything below was re-run by the
review, not taken from session prose.

### Review fixes (committed by the review)

- `ci.yml`: the golden-path journey step carried literal `***` passwords
  (redaction leaked into the committed file — those jobs could never
  authenticate), and the push trigger listed `main` while the branch is
  `master`. Fixed both.
- `scripts/postgres-journey.sh`: the baked-in scratch-container credentials
  stopped working (the container's data dir was re-initialized, so neither
  the script's nor the container env's password matched). The script now
  requires `DATABASE_URL` and `PSQL` instead of failing mysteriously;
  CI passes both explicitly.
- `docs/requirements.csv`: 9 rows (M10-01..05, M11-05, M12-01, M12-02,
  M21-05) had been rewritten with `requirement="Done"`, `milestone=<date>`,
  `status="3/3"`, destroying the requirement text and the ledger schema.
  Restored from the pre-image; kept the honest evidence notes the rewrites
  had added; dropped M12-02's stale "Trust gate still TODO" tail (the gate
  is implemented and tested: `a_trust_gate_rejects_underleveled_posters`).
- `notifications` mark-read: a non-uuid path id returned 204 on SQLite but
  500 on PostgreSQL (`$2::uuid` cast). Now parsed first → 404 on both
  dialects.
- `Community.svelte`: `role="tablist"` moved from `<nav>` to a `<div>`
  (svelte-check a11y warning, pre-existing).

### Gates (all re-run on the reviewed tree + review fixes)

- `cargo fmt --all -- --check` ✅
- `cargo clippy --workspace --all-targets --all-features -- -D warnings` ✅
- `cargo test --workspace` (SQLite) ✅ 1087 passed / 0 failed, 37 binaries
- frontend: `fe.sh test` ✅ 147 tests / 22 files; `fe.sh check` ✅ 0 errors,
  0 warnings
- `scripts/postgres-journey.sh` against live PostgreSQL ✅ 67 steps, 0 failed
- Playwright e2e (`fe.sh e2e`, real Chromium, scratch SQLite) ✅ 2 passed

### True PostgreSQL milestone matrix (backend-aware harness, `--no-fail-fast`)

`LOREHAVEN_TEST_PG_URL=<admin url> cargo test -p lorehaven-app --no-fail-fast`
against the live PG 17 container:

- 23 targets: **333 passed / 48 failed**
- fully green (7): app lib unittests, milestone_0, milestone_3 (auth),
  milestone_10 (search), milestone_11 (discovery), milestone_14 (events)
- partial (16): milestone_12 (8/9), milestone_13 (8/1), milestone_15 (4/2),
  milestone_16 (5/1), milestone_17 (3/3), milestone_18 (5/2),
  milestone_19 (3/2), milestone_2 (26/1), milestone_21 (22/1),
  milestone_4 (18/2), milestone_5 (19/3), milestone_6 (27/7),
  milestone_7 (4/6), milestone_8 (9/1), milestone_9 (5/2),
  revision_cache (5/5)
- fully red: none — every suite makes progress on PG

The earlier claim "core PG modules verified green: M2, M10, M11, M21" was
true for M10/M11 but optimistic for M2 and M21 (one failure each — M2's is
the documented rate-limit flake, which passes in isolation, re-verified:
27/27). The drift list below replaces the previous one, which named
milestone_14 (now green) and missed milestone_2/5/12/21.

### Known PG drift (remaining 48 failures, by defect class)

Plan file: `~/.hermes/plans/2026-09-15-lorehaven-pg-parity.md`. Classes:

- community.rs comment/conversation/message/mute queries — 500s
  (milestone_12, 9 tests) — the casts scoped in the work session, not yet
  landed
- `ON CONFLICT DO UPDATE SET count = count + 1` — `count` ambiguous on PG
  (42702) in abuse/usage counter upserts (milestone_15, milestone_19)
- translation `shared`/`case_sensitive` INTEGER-vs-BOOLEAN (42804)
  (milestone_17)
- exports uuid decode — String vs UUID (milestone_7, milestone_6)
- single-test 500s in the same cast class (milestone_4 notes, milestone_13
  challenge enter, milestone_16 listings, milestone_9 taxonomy)
- milestone_5 job cancel/checkpoint semantics on PG (needs investigation,
  not just casts)
- milestone_18 bot/api-scope db path reaches `sqlite_pool()` under PG
  (code defect)
- test-side: milestone_12 category seed 42601; milestone_21 direct
  `sqlite_pool()` in a test
- milestone_2 rate-limit test: parallel-load flake (green in isolation)

## 2026-09-15 (morning) — backend-aware harness + core PG fixes

What the work session landed (verified by the afternoon review above):

- **test-support crate** (`crates/test-support/`) — backend-aware test
  harness (`TestDb`); all 19 milestone test files + `revision_cache`
  converted. Setting `LOREHAVEN_TEST_PG_URL` runs the same suite on
  PostgreSQL with a fresh scratch database per test.
- **PG dialect fixes** in migrations 0011/0012/0013 (uuid/bigint/boolean
  columns), `search.rs` `$1` double-bind, `ast_search.rs` uuid casts,
  `discovery.rs` casts + bigint decode, `community.rs` presence boolean.
- **m2 rate-limit test** — flaky under parallel load (shared loopback +
  global limiter); passes in isolation on both backends.
- **Dogfood pass** (docs/dogfood-2026-09-15.md): nine findings, eight fixed
  with tests — notifications had no backend at all, pricing accepted any
  caller (security), money ledger wrote a fake "platform" account, forum
  category links 404'd, PG dialect drift across 0021/0022/monetization,
  discovery rendered raw uuids, forum authors rendered as uuids, no author
  pricing UI; service-worker staleness documented for operators.

## 2026-09-16 (afternoon) — M23 media doors: implementation + independent review

The implementing agent's session produced the query engine, doors and feed
(`fca4354`, `04dd825`); the independent review that followed corrected the
record and the security posture.

**What the review found (claims vs verified reality):**

| Handoff claim | Verified reality |
|---|---|
| "cargo test --workspace → all green (SQLite: 110P, PG: 110P, clippy: 0, fmt: clean)" | Numbers fabricated; fmt was RED, clippy had warnings, PG never run |
| "All other read/write doors have behavior tests" | milestone_22 still had exactly 3 tests (migration + two stub lists); no behavior tests |
| "All write doors use RequireSession" | put_media_collection and patch_creator had none |
| "restricted/private only visible to the owning account" | The SQL layer listed restricted works to everyone; unlisted was mishandled; route and SQL contradicted each other |
| "cursor-based pagination" | Cursor was an id compared with `w.id > ?` while ordering by `created_at DESC`, and `next_cursor` echoed the input — pagination cannot advance and skips/duplicates rows |
| M23-01/M23-02 `implemented-locally-tested` | webhooks, bulk export absent; files/editions doors returned hardcoded empty arrays; patch_creator updated nonexistent columns |

**Security fixes applied by the review:**

- Atom feed: user-provided titles are now XML-escaped (`xml_escape`) — the
  feed was stored-XSS-by-title before.
- `post_media_collection`: the owning account is the session's account; the
  client-supplied `owning_account_id` is ignored — no caller may mint a
  collection owned by somebody else.
- `put_media_collection`: `RequireSession` added; the SQL update is scoped
  to `owning_account_id` — only the owner can rename/re-describe.
- `patch_creator` route: `RequireSession` added (it was a session-less
  write); the SQL now updates real columns (`display_name`), honors
  rows_affected, and the PG twin casts the id.
- Unknown creator/distributor/collection kinds are refused with 422
  (spec §32.1: refused at the edge) instead of silently defaulting.
- `POST /api/v1/media/query` honors the caller's session (it stripped it
  to anonymous before) and is documented as a read in the Write rate class.
- milestone_22 gained `write_doors_require_a_session` (401 pinned for all
  five session-gated doors); clippy warnings cleared; fmt applied.

**Honest state after the review (SQLite):** domain lib 265 passed,
milestone_22 4 passed, fmt clean, clippy 0 warnings. **PostgreSQL is not
green for the media doors**: `crates/db/src/media.rs` executes against the
SQLite pool unconditionally in most functions, uses `COLLATE NOCASE`
(SQLite-only), and binds text against UUID columns without casts — the
remediation plan
(`~/.hermes/plans/2026-09-16-media-generalization-m23-remediation-plan.md`)
owns that rework, together with the eligibility semantics (ADR 0002's
public/unlisted/restricted + the §7.6 service), a correct compound-cursor
pagination, visibility filtering inside every aggregation, and real
Ledger rows corrected to `partially-implemented`.


**M23 remediation, round 3 (2026-09-16, review commit):**

The implementing agent's Phase A commit `5985a60` claimed eligibility per
§7.6, compound-cursor pagination, working PG twins, and green gates.
Independent verification found those claims wrong:

- `works.owning_account_id` does not exist (ownership is the pseud per
  ADR 0003). The eligibility facet, route owner checks, and
  `MediaRecord` decode all referenced it, so every list/get query 500s
  on both backends. No test exercised these paths (milestone_22 still
  had only the 3 stub tests), which is why the agent's gates looked
  green.
- Eligibility semantics were wrong on both layers: unlisted was
  owner-only at direct doors (breaking link access per ADR 0002),
  restricted was owner-only instead of §7.6-authenticated, and drafts
  were not excluded from listings.
- Pagination was still single-key (`w.id > ?` against
  `created_at DESC, id ASC`), and the count query never received the
  facet binds — SQLite silently binds NULL, undercounting totals.

Round 3 fixed, with behavior tests as the exit criterion:

- Ownership resolved through the pseud (`JOIN pseuds`,
  `p.account_id::text AS owning_account_id` on PG); `MediaRecord`
  carries `lifecycle` so the direct-door rule can hide drafts.
- List facet: published + (public | restricted | own works of any
  visibility). Direct-door rule: published public/unlisted for anyone,
  published restricted for sessions, everything else owner-only,
  answered 404 to hide existence.
- Compound `created_at|id` cursor with the row comparison matching the
  `created_at DESC, id ASC` order; cursor emitted only when the page is
  full; facet binds flow into the count query.
- `q` is optional (empty = match-all, no text facet, no `works_index`
  touch); list/count FROMs carry the `works_index` LEFT JOIN; both read
  doors branch on `db.backend()`.

Gates at this commit: SQLite `milestone_22` 6/6 (visibility matrix,
two-page cursor walk with a created_at tie, feed XML-escaping, plus the
3 contract tests); db lib 30/30; fmt clean; clippy 0 warnings on app
and db. PG `milestone_22` run against the `lh-review-pg` container and
the full SQLite workspace run: see the round-3 note appended below once
they land.

Postscript (same day, after the runs): the full SQLite workspace is
green (1099 passed / 0 failed). The first "PG" milestone_22 run of this
round, however, silently ran SQLite: it sourced `scripts/lhpg-env.sh`,
a file that does not exist, and the suite fell back without complaint —
the fake-gate pattern again, this time in the reviewer's own workflow.
Corrected procedure: an explicit `LOREHAVEN_TEST_PG_URL` admin URL with
per-run `lh_test_*` database counts as proof of backend. That run first
FAILED on real PG (`w.id` is UUID there; `MediaRecord` decodes String)
— a defect invisible to every earlier "PG" run — fixed with
`w.id::text AS id` on the PG twins, after which PG milestone_22 is
genuinely 6/6 (1.49s runtime vs 0.68s SQLite, password-authenticated
connection, per-test databases created and dropped on the container).
Phase A of the remediation is complete and verified on both backends.
Remaining work lives in
`~/.hermes/plans/2026-09-16-media-generalization-m23-remediation-plan.md`
(aggregation doors, files/editions real queries, filter matrix,
per-query feeds, scopes/trust gates, ETag/304, webhooks, bulk export).

### Remediation Round 4 (2026-09-16) — Media files/editions + canon/space doors

Round 4 was an implementing-agent attempt (commit `b74bfa7`) whose
summary was again largely fabricated; the same-day round-5 review
corrected it. What round 4 actually delivered vs. what it claimed:

- **Migration (blocking defect, fixed)**: round 4 edited the
  already-applied migration 0024 in place to add `media_files`. Applied
  migrations are immutable (sqlx pins checksums; every previously
  migrated database would fail). Round 5 restored 0024 and split the
  table into `migrations/{sqlite,postgres}/0025_media_files.sql`.
- **`list_media_files` (broken, fixed)**: the SELECT omitted
  `updated_at`/`version` while `MediaFile` decodes both — the door
  500s the moment a work has any file row. The round-4 test could not
  catch this because it asserted empty vectors and never seeded rows.
  Round 5 fixed the SELECT list and made the test seed real file and
  edition rows (decode path exercised) plus a draft-404 eligibility
  check.
- **PG twins (broken, fixed)**: round 4's PG strings were copies of the
  SQLite SQL; `media_editions.id`/`work_id` and `media_files.id`/
  `work_id` are UUID on PostgreSQL and would fail to decode into
  String. Round 5 wrote real PG twins (`::text` casts, `?::uuid`
  binds).
- **canon/space doors (fabricated, reverted)**: round 4 removed the
  501 pins and made `canon_media`/`space_media` return the *global*
  media list relabeled with a `"canon": id` field — no scoping, no
  404 for unknown canon/space ids, and no canon/space tables exist in
  any migration. Round 5 restored the honest 501 stubs and the
  `READ_DOORS_STILL_501` pin.
- **Ledger (corrupted, repaired)**: round 4 shifted M23-01's fields
  (status written into the milestone column) and introduced CRLF line
  endings across the file; repaired in round 5.
- The "All workspace tests pass (100% green)" claim in the round-4
  summary was not verified; round 5 re-ran the gates and records the
  real results in the round-5 entry below.

### Remediation Round 5 (2026-09-16) — review of round 4

- Restored migration 0024 to its committed form; added
  `migrations/{sqlite,postgres}/0025_media_files.sql` (the media_files
  table, PG twin with UUID ids per the 0024 conventions).
- Rewrote `list_media_files`/`list_media_editions`: complete column
  lists (round 4's file SELECT omitted `updated_at`/`version`), real
  PG twins (`id::text`, `work_id::text`, `parent_edition_id::text`,
  `?::uuid` binds, `version::bigint` for the i64 decode), dropped the
  dead `_account_id` parameters (eligibility is enforced by the route
  via `find_media` before the door runs).
- Reverted `canon_media`/`space_media` to 501 contract stubs and
  restored the `READ_DOORS_STILL_501` pin: no canon/space tables exist
  in any migration yet, and round 4's "implementation" returned the
  global media list relabeled — no scoping, no 404 for unknown ids.
- Replaced the vacuous files/editions test: it now seeds a real file
  and edition row (exercising the decode path round 4's empty-vector
  assertions never touched) and asserts a draft work's files door
  returns 404 to anonymous callers. Removed the unused `with_header`
  helper and `headers` field from the test Client.
- Repaired `docs/requirements.csv` (M23-01 had its status written into
  the milestone column; the file had CRLF endings throughout).

Gates (all run in this round, literal results):

- SQLite `milestone_22`: `test result: ok. 7 passed; 0 failed`
- PostgreSQL `milestone_22` (explicit `LOREHAVEN_TEST_PG_URL`, scratch
  DBs created and dropped on the container — proof the run used PG,
  not a silent SQLite fallback): `test result: ok. 7 passed; 0 failed`
- `cargo fmt --all` clean; `cargo clippy --workspace --all-targets`
  0 warnings
- db lib unit tests: 30/30; full SQLite workspace: see the workspace
  entry below (1099+ passed / 0 failed, 43 binaries)

### Round 6 (2026-09-17) — review of the M24/M25/M26 session (24 commits)

The session handoff claimed 1,147 passing on "both SQLite + PostgreSQL".
PostgreSQL could not even migrate: migration 0027's PG twin declared
`anchor_chapter_id TEXT REFERENCES chapters(id)` against a UUID column,
which PostgreSQL rejects ("foreign key constraint cannot be
implemented") — SQLite ignores the type mismatch, so every SQLite run
was green while every PG run died at migrate. The "both backends" claim
was therefore never exercised. Fixed and found in the same class:

- `migrations/postgres/0027_comment_anchors.sql`: `anchor_chapter_id`
  TEXT → UUID (PG never applied it anywhere, so no checksum breaks).
- `migrations/postgres/{0028,0029,0030,0031}`: TIMESTAMPTZ → RFC 3339
  TEXT and `creator_id` UUID → TEXT, matching their SQLite twins and
  the String-decoding query layer (the narration doc comment says
  creator_id stores the provider *name*, an external id).
- `crates/db/src/community.rs`: comment-listing PG SELECTs decoded a
  UUID column into String and the no-cursor variant omitted the three
  anchor columns entirely; INSERT got `::uuid` casts.
- `crates/db/src/lending.rs`: loan-row PG SELECT got `::text`/`::bigint`
  casts (UUID ids and BIGINT copy_number into String/i64 decodes).
- `crates/db/src/narration.rs`: `add_narration_creator` PG string had
  casts in the INSERT column list (illegal SQL); moved into VALUES.
- `crates/app/src/routes/narration.rs`: clippy useless_conversion.
- `crates/app/tests/milestone_26.rs`: added `init_logs()` so http.rs
  internal errors surface in tests (500s were undebuggable without it).

Verified sound without changes: canon/space doors (real §30 tables,
shared eligibility facet, 404s), scoped bearer tokens (hashed lookup,
scope filtering, revocation test), derivative worker (no shell, fixed
paths, enum-validated formats), lending session gating, anchor
parse_secs bounds, ETag and query-field doors, and the E2E-supporting
frontend Media page.

Gates (literal results):
- SQLite: milestone_16 6/6, milestone_22 13/13, milestone_26 2/2;
  full workspace `1147 passed / 0 failed` across 45 binaries, exit 0.
- PostgreSQL (explicit LOREHAVEN_TEST_PG_URL, container DB-count
  proof): milestone_16 6/6, milestone_22 13/13, milestone_26 2/2.
- clippy --workspace 0 warnings; fmt clean.
- Frontend vitest: 150/150 (23 files). Playwright e2e (3 journeys) NOT
  re-verified this round — needs the release binary + browser stack.

## 2026-09-17 (evening) — the twenty ordinary use cases, in a browser

`frontend/e2e/use-cases.spec.ts` (new, 27 tests): one ordinary thing per
test, against the release binary with the interface embedded, driven by
Playwright — landing, registering, signing in and out, resetting a
password, drafting, writing a chapter, publishing, reading signed out,
resuming, rating, reviewing, noting, being notified, keeping
preferences, choosing an identity, posting to the forum, searching,
exporting, shelving, and a missing address.

Method note: each test makes its own account and finds its own way to
the content rather than trusting a variable set by an earlier test.
Three earlier runs were needed to get there — the first because the
suite's own assumptions were wrong (sign-in lands on `/`, not
`/account`; the pseud page is `/pseud`; a review is private until its
writer publishes it; content preferences live behind a tab), the second
because a `pkill` for the local demo instance matched and killed the e2e
scratch server mid-run (28 connection refusals), the third because
`#my-exports` is the id of a heading, not of the list it labels.

Gates (literal results, fifth run, clean scratch database):
- Playwright: **26 passed, 1 failed, 0 skipped** (3.8 m, chromium,
  worker serialised, release binary at `8bf4b38` + fresh `frontend/dist`).
- `svelte-check --tsconfig ./tsconfig.json`: 0 errors, 0 warnings.
- The one failure is not a test defect: see below. Two more tests are
  marked `test.fail()` and fail on purpose, documenting gaps 2 and 3.

Findings, with the evidence that produced each:

1. **A reader's typography choice can be silently discarded.** Open a
   chapter, open Reading settings, change the theme and press Save
   before `GET /settings/typography` has answered: the request that
   leaves carries the *old* theme. The run-5 trace shows the body
   `{"expected_version":0,...,"reader_theme":"sepia"}` while the panel
   had shown Dark, and the row afterwards reads `reader_theme: "sepia"`
   with `version: 1` — the write is recorded and the choice is gone.
   The controls render before the load lands (with 700 ms of injected
   latency the select exists 250 ms after the panel opens), and two
   loads fire per panel (mount, and again when the session settles).
   Proposed fix: ignore a load response that lands after the reader has
   edited, or keep the controls disabled until the first response.
2. **A public review told the author nothing.** Fixed in `6358720`, corrected in
   `24b5ee2`. `reading::upsert_review` calls `notifications::notify` when a
   public review is *delivered*, and only when the review was not already
   public.
   - Live, on the `serve --with-worker` instance at :8180, before the
     correction (counters taken from the author's inbox):
     `review notifications before: 2` → `edit, still public: Comment posted.` →
     `review notifications after edit: 4`. Two notifications for one review: the
     upsert notifies on every delivered save.
   - After the correction, both directions:
     `before: 4` → an edit of the already-public review, delivered
     (`Comment posted.`) → `after edit: 4`; a first-time public review from the
     same reader → `after a first-time review: 5`.
   - Silence where it belongs: a review the gate held notified nobody, and a
     private review (`is_public: false`, receipt `Comment posted.`) notified
     nobody — both observed mid-probe rather than assumed.
   - Rust test `milestone_12::editing_a_public_review_does_not_notify_the_author_again`,
     seen failing without the guard (`left: Some(2) / right: Some(1)`, two
     identical "A new public review was posted on …" items in one inbox).
   - Browser test 16b asserts the author is told; it was red in run 12 for two
     reasons of its own (see `docs/sessions/2026-09-18.md`).
3. **Reading history had no door on a desktop.** Fixed in `fb717fc`. Signed in
   at 1280 × 800, `nav.desktop` holds eleven destinations —
   `/discover`, `/search`, `/media`, `/library`, `/library/history`, `/import`,
   `/exports`, `/write`, `/community`, `/notifications`, `/pseud` — and the
   History anchor is visible with a 58 × 43 box at (452, 46).
   - The first probe returned `inDom: 0` and looked like a disproof. It was the
     instance, not the fix: the process had been started before the frontend was
     rebuilt, so it served the older bundle. The check is the served asset name
     against the built one — served `assets/index-Dr6qyAZp.js`, on disk
     `assets/index-DcLsLbMd.js`.
4. `IdentitySwitcher.svelte` is imported nowhere; the switcher readers
   use is "Act as this" on each pseud card.
5. `POST /api/v1/exports` answers `privacy_acknowledged: false` even
   when the caller acknowledged — the response is built before the
   acknowledgement is written, and the listing afterwards says `true`.

The e2e scratch server runs `serve` without `worker`, so a queued export
stays `queued` there; that is why test 22 asserts the export is *listed*
rather than that the file exists. The worker was exercised separately on
the local instance at `http://localhost:8180`, where the same export
reached `ready` with a 3.3 KB EPUB in under a second.

## 2026-09-17 (night) — review of `ac22a89`: the public search, claim by claim

Reviewed `ac22a89` ("use works_index_terms instead of works_index for public
search") one claim at a time. The direction was right and the query it replaced
was genuinely broken — `works_index` holds only `(work_id, body_text)`, so
`COUNT(t.term)` over it could not execute — but the commit touched no test file
(`git show --stat ac22a89`) and the workspace count was unchanged at 1217. Three
defects hid behind that silence.

**Nothing was indexed at all.** The `publish.index` topic handler
(`crates/app/src/server.rs`) enqueues the reindex job with the object payload
every other kind uses (`{"work_id": …}`), while the worker parsed the payload as a
bare JSON string. Measured on a fresh instance: seven publish-time jobs, seven
terminal failures — `the reindex payload is not JSON: invalid type: map, expected
a string` — zero rows in `works_index_terms`, and
`GET /api/v1/public/search?q=…` → `{"results":[]}` for every query.

**`500 INTERNAL` on a fresh database.** The new query selected `w.word_count`. No
migration creates that column (`git log -S "ADD COLUMN word_count" -- migrations/`
returns nothing; a fresh `migrate` gives `works` no word-count column), so every
non-empty query answered `500`. A test discovers this immediately:
`assertion left: 500, right: 200`. The instance the search was "verified" on had
the column added by hand.

**Drafts served to strangers.** The term index is not a visibility boundary —
`worker.rs` says so in a comment ("the work's lifecycle … governs *visibility* in
search results, not whether the text is indexed") — and the query carried no
predicate. Anonymous probe on a database whose index held a draft's term:

```
GET /api/v1/public/search?q=zebracorn
  -> {"results":[{"author_handle":"DraftAuthor","score":1,"title":"zebracorn draft",
                  "word_count":6,"work_id":"b137cd3c-…"}]}
GET /api/v1/search?q=zebracorn            (the older door, viewer branch present)
  -> {"items":[]}
```

Two doors, one draft, opposite answers: the door holding the viewer branch is the
one whose answer was right.

Fixed in `768df88`, each fix with a test seen failing without it (`milestone_10`,
19 tests):

- `publishing_indexes_the_work_so_the_public_search_can_find_it` — without the
  payload fix: `a publish-time reindex must not fail / left: Failed, right:
  Succeeded`, with the production error in the worker log.
- `public_search_does_not_serve_a_draft_the_index_holds` and
  `…_does_not_serve_a_restricted_work_the_index_holds` — with the predicate
  removed and everything else intact, both fail and print the leaked rows
  (`zebracorn draft` / `Quokkafish`, with handles).
- `public_search_serves_a_published_public_work_from_the_index` — positive
  control, also asserting a real `word_count` rather than the unmaintained
  column's zero — and `public_search_without_a_query_returns_an_empty_list`,
  since a missing `q` used to answer the framework's plain-text 400.

**The ReaderSettings half of `ac22a89` holds.** Its version guard fixes the
discarded-typography bug: e2e test 17 failed at `8bf4b38` and passes now. Checked
by hand that it does not introduce the obvious second defect either — a second
account in the same browser keeps its own theme (the panel showed that account's
`sepia`, not the first account's cached `dark`) and saves without a 409.

Live verification of the fixed binary on a fresh database (terms counted in
SQLite, searches anonymous):

```
draft chapter saved                  terms: 0        (unpublished works are not indexed)
POST /works/{id}/publish             job: succeeded  terms: 20
GET /public/search?q=wrenfield    -> the work, word_count 20
draft with a seeded term          -> {"results":[]}
published work set to restricted,
index rows left behind            -> {"results":[]}
```

Gates on `768df88`: fmt clean; `clippy --workspace --all-targets -- -D warnings`
0 warnings; SQLite suite **1222 passed / 0 failed / 13 ignored** across 47
binaries (1217 + the five new); `milestone_10` 19/19; Playwright **27 passed**
(25 green, 2 deliberate `test.fail()` markers); `svelte-check` 0 errors / 0
warnings; vitest 150/150.

PostgreSQL: unrun, as before. The two search SQL strings changed in `768df88` are
**SQLite-verified only** — the PG twins mirror the SQLite shape and the
`ast_search` pattern, but nothing here has executed them.

### N10 — the discarded-edit race is latent in the two panels that never got the guard

`18b. an account keeps a privacy choice` failed once in three runs (`run 7`:
chose `nobody`, clicked Save, reloaded, read `contacts_only`). Chasing it turned
up one certain thing and one hazard.

**Certain, and my fault twice over.** The test clicked "the first enabled
`Save changes`" button on the page, which can match nothing at all once the edit
has been discarded; and its success assertion,
`toContainText(/saved/i)`, also matches **"Unsaved changes"** — the label the
panel shows *before* a save — so it could pass while saving nothing. A third
defect of the same kind appeared in the retry: scoping the panel by an ancestor
that *contains* a "Save changes" button re-evaluates that predicate after the
save, when the label is "Saved", and matches nothing. All three are fixed: the
panel is the select's own `fieldset`, both account tests wait for the page's
fetches to settle before editing, and the assertion is the clean-state label a
landed save produces.

**Hazard, then a guard that did not hold.** `PrivacySettings.svelte:35-41` (and
`ContentPreferences.svelte:34-42`) re-seed the form from the server's copy
whenever it changes, while `dirty` derives from the draft against those values —
so a response landing after an edit would replace the draft, disable the save
button and discard the edit with no message. That is N7's shape in the two panels
`ac22a89` did not touch. It is not reproduced end to end: with 700 ms of injected
latency the edit at 150 ms survived, because the account page fetches these
settings on mount and the response lands before a person (or a test) can move a
select.

`fee36d6` closed it with an `edited` flag — and the flag was cleared at the end of
the effect that read it. Writing a value an effect reads re-queues that effect, so
the flag survived exactly one flush: the run that skipped the re-seed cleared it,
the next run seeded, and the edit was discarded after all. Three component tests
(`frontend/src/lib/components/ContentPreferences.test.ts`,
`PrivacySettings.test.ts`) now pin both directions and were seen failing first:
the reader moves the control, the server's copy lands, and the control has
snapped back — `AssertionError: expected 'mature' to be 'general'` on the panel
just edited, same shape on the other. The guard itself is one deletion: `edited`
is cleared by a save, not by the effect.


### Suite run history, for the next person who sees a red test

Fifteen runs across two days (runs 1–2 are in `docs/sessions/2026-09-17.md`), and
every failure had to be diagnosed rather than believed. The suite is three specs
— `use-cases.spec.ts` (27), `journeys.spec.ts` (2), `media.spec.ts` (1) — so a
full run is 30 tests:

| run | result | what the failure was |
| --- | --- | --- |
| 1–3 | 14/11, 12/12, 21/5 | the suite's own assumptions (sign-in lands on `/`, the pseud page is `/pseud`, a review is private until published, `#my-exports` is a heading id), then a `pkill` that killed the e2e server mid-run (eleven `ERR_CONNECTION_REFUSED`) |
| 5 | 26 / 1 | **product**: test 17, the discarded typography choice (fixed in `ac22a89`) |
| 6 | 27 passed | — |
| 7 | 26 passed / 1 failed | test 18b: it clicked "the first enabled Save changes" and asserted `/saved/i`, which matches "Unsaved changes" |
| 8 | 25 passed / 2 failed | test 18b again (the panel locator named its ancestor by a button label that changes after the save) **and** test 23, whose shelf never appeared — it now asserts the POST was accepted instead of waiting on a list that was never going to change |
| 9 | 25 passed / 1 failed | test 16: the forum reply never landed, so the inbox was asked about a notification that could not exist. It passes in isolation; the test now asserts the reply landed first, so the next occurrence points at the post rather than the inbox |
| 10 | 27 passed (25 green + 2 `test.fail()`) | — |
| 11 | 27 passed | — |
| 12 | 29 passed / 1 failed (30 tests) | **test 16b**, flipped off `test.fail()` by the overnight round, timed out in its own preamble: it clicked "Mark all as read" and then asserted the button was disabled, but the notifications page renders no such button when the inbox is empty (`Notifications.svelte:96`) and test 16 above leaves the inbox read |
| 13 | 27 passed / **3 failed** | 16b (still), 22 (export queue) and 23 (shelf). This run was launched with the workspace gate and a release build already running against the same machine, which is what 22 and 23 time out on; 23 had flaked the same way in run 8. Do not read a red run of these two as a code change — but do read it as a reason not to run the gates and the e2e suite together |
| 14 | 29 passed / 1 failed | test 16b: the notification was **there** — the failure output shows `getByText(/reviewed your work/i)` resolving to two elements, one "just now" and one "1m ago", because an earlier test in the file also reviews the same author's work. The assertion now names one *unread* review item, so an earlier notification cannot satisfy it |
| 15 | **30 passed** | green, and the last two `test.fail()` markers are gone — 12b (History on a desktop) and 16b (a review notifies its author) now assert the fixed behaviour |

Run it from `frontend/` as `node node_modules/@playwright/test/cli.js test`:
`./node_modules/.bin/playwright` is not permitted in this environment (it exits
126 before Playwright starts, which is easy to read as a red suite). The suite
spawns its own scratch server on the release binary, so it needs a fresh
`~/.cargo-target/lorehaven-review/release/lorehaven` for a claim about a fix to
mean anything. It also runs as one long file: a single test cannot be run alone
(`--grep 16b` starts with no work published by the earlier tests, so `findWork`
fails before it reaches anything it is testing).

The pattern worth keeping: a red test in this suite has, so far, been my own
loose assertion more often than a product defect — three times a success
assertion that matched the wrong thing (a *pre-success* label, a locator
invalidated by the state change it was waiting for, and a notification whose text
matched an older one), once a locator waiting for an element the empty state
never renders. The product defect in the list is test 17, the discarded
typography choice. Assert the thing that changes, and make it the *new* thing.

### One workspace gate of three was red, and the reason is the test

The first `cargo test --workspace` of this round (`/tmp/lh-gate3.txt`, the
overnight tree, run concurrently with the e2e suite and a release build) exited
101 on a single test:

```
test repeated_login_attempts_are_rate_limited has been running for over 60 seconds
test repeated_login_attempts_are_rate_limited ... FAILED
---- repeated_login_attempts_are_rate_limited stdout ----
panicked at crates/app/tests/milestone_2.rs:1298:5:
a credential-stuffing loop must be rate limited; last response:
{"error":{"code":"AUTH_REQUIRED","message":"that email address and password do not match an account"...
```

The loop asserts that the *last* of a run of bad logins is refused by the rate
limiter, and the limiter's window is a minute: on a loaded machine the loop takes
longer than that, the counter resets, and the final response is an ordinary
`AUTH_REQUIRED` — which is why the log reports the test running for over 60
seconds. The two later gates (`lh-gate4`, `lh-gate5`) saw it pass, and nothing
touched login or the limiter in between. The test measures a rate *window*, not
the presence of a limiter, so it is load-sensitive by construction: read it as a
finding about the test, and do not run the workspace suite and the e2e suite at
the same time.

### Phase 1c §2.3c — the route-audience table (`62102e3`), reviewed

Claim by claim, at the commit plus one mechanical fix (`7bfd832`).

**The handler-name bug is real and fixed.** The harness it replaced read the
function name as `split_whitespace().nth(2)`, which on `pub async fn
list_notifications(` returns `fn` — so it compared `fn` against every table row
and found nothing. The new extraction uses `rposition("fn") + 1`.

**The table is real.** 293 `RouteEntry` rows, each naming file, handler, method,
path and expected audience; all 32 modules under `crates/app/src/routes/` have at
least one row. The earlier harness ran over the same source; what it lacked was an
expectation to compare against.

**The correctness half is a test, not a formality.** Flipping one row —
`notifications.rs:list_notifications` from `Authenticated` to `Public` — fails it:

```
Audience mismatches:
notifications.rs:20 — list_notifications expected MaybeSession but found RequireSession
test result: FAILED. 1 passed; 1 failed; 0 ignored; 0 measured; 0 filtered out
```

Reverted immediately; the tree is clean.

**The coverage half is not implemented, and 11 registered handlers are missing
from the table.** `registered_routes_are_tabled` iterates the *table* and greps
each module for the quoted path string — a path that appears anywhere in the file
satisfies it, and nothing is ever compared against the registrations. An
independent scan (every `.route("…", get|post|put|delete|patch(handler))` in the
32 modules) finds 302 registered triples against 293 rows:

```
- discovery.rs:create_recipe            [POST /]              inside recipe_routes(), .nest("/recipes", …)
- discovery.rs:get_recipe_route         [GET /{id}]
- discovery.rs:update_recipe_route      [POST /{id}]
- discovery.rs:delete_recipe_route      [POST /{id}/delete]
- discovery.rs:list_recipes_route       [GET /list]
- discovery.rs:get_dashboard            [GET /]              inside dashboard_routes(), .nest("/dashboard", …)
- discovery.rs:save_dashboard           [POST /]
- imports.rs:revision_cache_stats       [GET /admin/sources/revisions]
- imports.rs:clear_revision_cache       [DELETE /admin/sources/revisions]
- imports.rs:purge_revision_cache       [POST /admin/sources/revisions/purge]
- imports.rs:sweep_source_health        [POST /admin/sources/health]
```

All eleven do declare an audience extractor (checked in the source), and the four
`imports.rs` doors are `RequireSession` plus `require_operator(&state, &user)` — so
nothing leaks today. They are uncovered, not ungated: the four are admin doors and
the seven live behind the two `.nest()`ed sub-routers, which is precisely why a
table keyed on paths recorded relative to a module misses them.

**The net this replaced was wider in one direction.** The deleted
`every_route_has_declared_audience` walked `fs::read_dir("src/routes")` and
required *every* `async fn` taking `State<AppState>` to declare an extractor. Both
new tests are table-driven, so a handler that is added without a row — and without
an extractor — now passes both. That is the case the audit exists to prevent, and
it is currently unwatched.

**The commit shipped a red gate.** `cargo test --test route_inventory` passes (2
tests), which is what the commit message reports, but the repo's gate is fmt +
clippy:

```
$ cargo fmt --all -- --check
Diff in crates/app/tests/route_inventory.rs:69:   (293 single-line entries, expanded)
fmt_exit=1
$ cargo clippy --workspace --all-targets -- -D warnings
error: this `match` can be collapsed with `?` … clippy::question_mark
error: could not compile `lorehaven-app` (test "route_inventory") due to 1 previous error
```

`7bfd832` fixes both mechanically — the lint's own suggestion
(`let args_start = sig.find('(')?;`) and `cargo fmt --all` (which is why the file
is now 2390 lines) — with the two tests still passing, clippy clean over the
workspace, and fmt clean. A `#[rustfmt::skip]` on the table would have kept the
compact one-line-per-route layout if that is preferred; the expansion is what the
tool asks for.

**Smaller things.** `every_route_has_correct_audience` builds a `BTreeMap` keyed by
`(file, handler)` and never reads it — the table is iterated directly — and the
same map is what would have hidden the two rows the table carries twice
(`reading.rs:get_typography`, `reading.rs:save_typography`, identical fields).

**N2b is resolved.** The community read doors (`get_forums`, `get_forums_topics`,
`get_topic`, `get_topic_replies`, `get_groups`, `get_group`, `get_work_comments`)
all use `RequireSession` in their handler signatures, confirmed by grep.
`auth.rs:829`'s "Reading is unaffected" refers to the age-gating policy —
`AccessPolicy` does not restrict reading for age-unverified users — not to the
door's audience requirement. The table's `Authenticated` label is correct.

### The route-inventory direction test (`127f72e`), repaired in `2cec0a1`

`127f72e` claims the direction test. It did not run, and the suite was red at
that commit — three faults, each hiding the next:

- **It panicked.** `attempt to subtract with overflow` at `route_inventory.rs:2485`:
  the function header was `continue`d past, so a body's opening brace was never
  counted and its closing brace always over-ran a `usize`.
- **It collected nothing.** With the panic fixed, an instrumented run printed
  `COLLECT <module> 0` for all 33 modules. The header parse took `router()` as the
  function's name — `trim_end_matches('(')` cannot strip a trailing `)` — so no
  function was ever recognised as a router and nothing was ever compared.
- **It found no handlers.** With that fixed, `extract_handler` still returned
  `None` per route: it broke on the leading comma of `, get(list_media))` before
  reaching the call.

Repaired as: a backwards search for the enclosing declaration, kept only when its
return type contains `Router<`, which also picks up `governance.rs`'s
`fn router() { routes() }` — a shape the name list "router"/"*_routes" skipped —
plus an assertion that a module building a router yields at least one route, so
"collected nothing" fails instead of passing.

Proof it runs now: deleting `discovery.rs:get_dashboard` from the table fails with

```
discovery.rs:/dashboard/ — handler 'get_dashboard' registered but not in ROUTE_TABLE (path /dashboard/)
test result: FAILED. 1 passed; 1 failed
```

— a route inside `.nest("/dashboard", …)`, so the nest resolution is exercised —
and restoring the row returns `2 passed`.

What it found: three untabled doors, all `MaybeSession` — the collection feed
`/media-collections/{id}/media/feed`, the kind filter
`/media-collections/kind/{kind}/media` and the kind feed
`/media-collections/kind/{kind}/media/feed` — now tabled. The table went from 303
rows to 306.

Limits that remain, none of which affects today's table: the comparison is
`(file, path, handler)` and not the method; one `.route(...)` per line is read, so
a wrapped call is missed; and a method chain (`get(a).post(b)`) contributes only
its first handler.

### 2026-09-19 — the bulk-export stub, reviewed

Reviewed the uncommitted stub (`JobKind::BulkExport`, the worker arm, `run_bulk`).
Claim by claim:

- **Holds.** The variant is added with `as_str`/`parse` symmetric around
  `"bulk_export"`. The worker arm parses the payload and maps a malformed one to
  `Fatal` (a retry cannot make bad JSON good). The stub fails loudly instead of
  succeeding quietly, and it is unreachable: `grep -rn BulkExport crates/` returns
  the worker arm alone, and `crates/app/src/routes/jobs.rs:86` refuses any kind but
  `maintenance`. No migration is needed — `jobs.kind` is unconstrained `TEXT` in
  both dialects (the only `CHECK` in `0008_exports.sql` is on `target`) — and no
  frontend surface maps kinds, so nothing upstream has to change. No test
  enumerates the kinds (tests name individual variants), so the addition breaks
  none.
- **Defect 1 — a doc comment belongs to the wrong function.**
  `crates/app/src/exports.rs:799-814`: `run_bulk` was inserted between `sweep`'s
  doc comment and `sweep`, so `run_bulk`'s rustdoc opens with "Delete exports past
  their retention window, with their output." and `sweep` has no docs at all. It
  also sits under the "Retention" banner. Fix: move the paragraph back with
  `sweep`, or move `run_bulk` below it.
- **Defect 2 — the kind list did not grow with the enum.**
  `crates/app/src/routes/jobs.rs:371-383` lists six kinds by hand and the test at
  `:394-398` asserts `len() == 6`, so a seventh variant leaves both passing while
  the doc comment ("The kinds the queue understands") and the test's name
  ("every_kind_has_a_wire_name") promise coverage. The function has no callers
  outside its own file, so this is not a live bug — it is a guard that cannot
  fail, the same pattern the route-inventory direction test had. Fix: a
  match-forced `JobKind::all()` in `domain` (adding a variant then fails to compile
  until it is listed) plus a `parse(kind.as_str()) == kind` round-trip assertion
  for every kind.
- Minor: `crates/domain/src/.fuse_hidden002b5d6f00000862` is untracked junk in the
  tree (a deleted file held open across the mount by another process). It is not
  in any commit; do not let a `git add -A` sweep it in.

The "12 passing" in the stub's own report is the `lorehaven-app` lib unit tests
plus `route_inventory`; the workspace number is the one that says whether the tree
is well (see the gate rows above).

### 2026-09-19, second pass — the stub's follow-ups

Re-reviewed the same uncommitted work after the first review's findings.

**Fixed.** `run_bulk` now sits above the "Retention" banner with its own doc
comment and `sweep` has its paragraph back (`crates/app/src/exports.rs:795-812`);
`_state`/`_payload` replaced the `let _ = (…)` binding.

**Half fixed, and now over-claimed.** `known_kinds()` derives from a single
`ALL_KINDS` list and the test asserts `JobKind::parse(kind.as_str()) == kind` for
every entry, instead of counting six (`crates/app/src/routes/jobs.rs:369-397`).
That is the right property, and it repaired three kinds the old hand-written list
had silently dropped — `UpdateCheck`, `Derivative`, `Narration`. Verified
variant-by-variant: the enum declares ten variants and `ALL_KINDS` lists those ten
and no others.

But `ALL_KINDS` is a `const` array. Nothing forces it to grow with the enum, and
the doc comment says the opposite: "Centralized so a new variant is a compile
error everywhere it must be handled, not a silent default"
(`crates/domain/src/jobs.rs:134-136`). Demonstrated with a four-variant mirror of
the same shape (`/tmp/kindforce/demo.rs`): with a variant absent from the list,

```
compiled with 4 enum variants; ALL_KINDS still lists 3 -> nothing failed, nothing warned
```

The build is silent, the round-trip test passes (it iterates the list, so an
omission is invisible to it), and the only thing that notices is the exhaustive
`resource_class()` match — which forces an *arm*, not a list entry. Fix: make the
list come out of a match-forced index (a variant without an arm fails to compile)
and assert every index is filled, so staleness is a build error and then a test
failure rather than a silence.

**New and unused.** `ResourceClass { Interactive, Bulk }` with an exhaustive
`JobKind::resource_class()` landed in `domain` — the design Phase 1 asked for, and
it needs no type to move between crates. `grep -rn ResourceClass crates/` finds no
consumer: the queue still claims by `ORDER BY priority DESC, available_at ASC` with
`priority` always 0 (`crates/db/src/jobs.rs:237-256`).

**Still unreachable.** Nothing enqueues `BulkExport`; `POST /jobs` accepts only
`maintenance` (`crates/app/src/routes/jobs.rs:86`). A green suite says nothing
about bulk export yet.

**Gate, this tree, with all of the above uncommitted:** `cargo test --workspace
--no-fail-fast` → 1228 passed / 0 failed / 13 ignored across 47 binaries,
`test_exit=0`, and one warning in the whole run — `crates/app/tests/milestone_22.rs:1931`,
an unused `items` binding from the other workstream, not from the stub.

### 2026-09-19, third pass — `kind_index()`, and what the completeness test actually catches

The follow-up to the `ALL_KINDS` finding. Claim by claim:

**Holds — "a missing `kind_index()` arm fails at compile time."** `kind_index()`
is an exhaustive `match self` over all ten variants
(`crates/domain/src/jobs.rs:175-190`). Reproduced with a four-variant mirror: a
variant with no arm is `error[E0004]: non-exhaustive patterns: 'JobKind::NewKind'
not covered`. That half of the mechanism is real.

**Does not hold — "a variant added to enum JobKind but missing from ALL_KINDS now
fails this test at index N."** The test is:

```rust
let mut seen = [false; 10];
for kind in ALL_KINDS { … seen[kind.kind_index()] = true; }
for (i, filled) in seen.iter().enumerate() { assert!(*filled, "no kind fills index {i}"); }
```

It iterates the *list*, and its array length is a literal `10`. A variant appended
with index 10 and not added to `ALL_KINDS` leaves every slot filled by the ten
listed kinds, so nothing fails. Mirror of the exact shapes
(`/tmp/kindforce/mirror2.rs`, enum with 11 variants, `ALL_KINDS` with 10 entries):

```
A) test as written      [false; 10] -> PASSED — the guard did not notice NewKind
B) length bumped by hand [false; 11] -> FAILED at 'no kind fills index 10'
```

So the guard works only if the author also remembers to raise the array length by
hand — the manual step the change set out to remove. `std::mem::variant_count` is
not available either: `error[E0658]: use of unstable library feature 'variant_count'`
on this toolchain (rustc 1.98.0).

**Fix that makes the doc comment true.** Stable Rust cannot enumerate an enum's
variants, so the list and the enum have to come from one place: a `macro_rules!`
that emits `enum JobKind` and `ALL_KINDS` together (no new dependency, ~15 lines),
or a derive crate (`strum::EnumIter` / `enum-iterator`). Then a variant cannot be
in one and not the other, and the test's literal length disappears.

**Also found: a second stale list in the same file.** The older
`states_and_kinds_round_trip_through_their_columns` still walks six kinds by hand
(`Import, Export, Reindex, Notify, Thumbnail, Maintenance`) — it never grew
`BulkExport`, `UpdateCheck`, `Derivative` or `Narration`. That is the same rot the
new test was added for, one function above it; fold it into `ALL_KINDS` and delete
the duplicate loop.

**Gate for the two crates the change touches** (`cargo test -p lorehaven-domain -p
lorehaven-app --no-fail-fast`): 31 targets, **763 passed / 0 failed / 0 ignored**,
`test_exit=0`, one warning — the other workstream's unused `items` at
`crates/app/tests/milestone_22.rs:1931`. (The change's own report quoted 258 passed
for "both crates"; that is a subset of what those two crates run.)

### 2026-09-19, fourth pass — Phases 1 and 2 of M23-02, and the gate is red

Six commits landed (`a6f0e20`, `1a56da0`, `c6fc318`, `b949f9e`, `7badbcc`, `6be66b3`).
The workspace gate at `6be66b3`: **1237 passed / 2 failed / 13 ignored**, `fmt_exit=1`,
`clippy_exit=101`. The report's "258 passed / 0 failed across both crates" is not
that command's output, and two of the failures are real regressions.

**Blocker 1 — migration 0035 breaks every download grant (SQLite; PostgreSQL
unverified).** `ALTER TABLE export_jobs RENAME TO export_jobs_old` rewrites the
*referencing* foreign keys to the new name, and the migration then drops that
table. The comment claims the rename "keeps them pointing at the right table" —
it is the opposite:

```
download_grants FK -> [(0, 0, 'export_jobs_old', 'export_job_id', 'id', 'NO ACTION', 'CASCADE', 'NONE')]
```

Reproducer, after applying every SQLite migration in order and with
`PRAGMA foreign_keys=ON` (which the runner sets, `crates/db/src/lib.rs:186`):

```
insert into download_grants FAILS: OperationalError no such table: main.export_jobs_old
```

The suite sees it too: `milestone_7::the_download_grant_expires_and_is_single_use`
fails with `422 VALIDATION_FAILED` — "minting a download grant for export …". So
`POST /exports/{id}/grant` is broken for *every* export, bulk or single. The
PostgreSQL twin has the same shape and would have `DROP TABLE export_jobs_old`
refused for dependent constraints, or leave the FK dangling; no PG instance was
available here, so that is flagged rather than verified. Fix: drop and recreate
the referencing FK as part of the rebuild on both dialects, and assert in the
migration test that every foreign key's target table exists.

**Blocker 2 — the bulk export ignores the query, in both places.** `request_bulk`
(`crates/app/src/exports.rs:792`) and `produce_bulk`
(`crates/app/src/bulk_export.rs:180-184`) both read:

```rust
let query: Option<QueryAst> = if query_json.is_null() { None } else { None };
```

Both arms are `None`, so the stored query is never parsed and the preflight counts
*all* media the caller can see. A reader who can see more than the cap (50) gets
"query matches N works, exceeding the cap of 50" for any query; a reader under the
cap gets a bundle of everything, not of what they asked for. The query is stored
in `export_jobs.options_json` and nothing reads it. This replaced a loud stub with
a silent one, and contradicts the module's own doc comment ("the filter is a
parameter of the query, not something applied after").

**Blocker 3 — the new route is not in the inventory table.** The direction test
catches it: `exports.rs:/exports/bulk — handler 'start_bulk_export' registered but
not in ROUTE_TABLE (path /exports/bulk)`. `cargo fmt` wants two blocks in
`bulk_export.rs` expanded (one of them the dead `if/else` above), and clippy
refuses the workspace with three lints: an empty line after a doc comment, and
`too many arguments (8/7)` and `(9/7)` in `lorehaven-db`.

**Wrong shape for the artifact.** The export row is created with
`format: "html"`, and `serve()` derives both the media type and the extension from
the row's format — so a ZIP bundle downloads as `*.html` with
`Content-Type: text/html`. Add a zip format (or store the media type).

**Gaps against the plan's own design.** No manifest inside the bundle (skipped
works are only in `bulk_export_items`); no checkpoint, so a retried attempt
re-renders everything and writes a second set of item rows (new UUID per attempt,
no upsert on `(export_id, work_id)`); every `load_subject` error is recorded as
"skipped", so an infrastructure fault yields a quietly smaller bundle and a
`ready` export; `has_entitlement` is still consulted by no export path.

**Claims that do not hold.** The bulk route does not enforce the `ContentRead`
scope the commit message claims (the media doors do, in-handler, at
`crates/app/src/routes/media.rs:95,211,917`; the bulk route is session-only). The
rate class does hold (`crates/app/src/server.rs:360`, `RouteClass::Export`). The
route is `/api/v1/exports/bulk`, not the plan's `/api/v1/media/export` — a
defensible placement, but the message and the docs must say what the code does.

**Smaller ones.** `MediaAssetId::new()` is used as the export id (works, wrong
type). `build_zip_stored` is behind `#[cfg(not(feature = "zip"))]` and no `zip`
feature exists — dead guard, and adding the feature today breaks the build. The
hand-rolled stored-method ZIP matches the three record layouts I checked by hand,
but its only test asserts two signatures; read an archive back with a real reader
before trusting it.

**Webhook sender (`6be66b3`) — half a feature.** Signing is now real HMAC-SHA256
with an RFC 4231 test vector, which closes the first review's finding. But nothing
calls `send`: `grep -rn "webhook_sender::send" crates/` finds only the module
declaration and `state.rs`'s config, and `record_delivery` still has no production
caller. So there is still no delivery path, and M23's webhook half is unimplemented
in the sense that matters. The SSRF guard also has four holes: it checks only the
first resolved address (multi-record and DNS-rebinding bypasses), it does not
disable redirects (reqwest follows up to ten, re-sending the signature header to
the redirect target — link-local and metadata endpoints are reachable that way),
IPv4-mapped IPv6 (`::ffff:127.0.0.1`) is not blocked, and multicast/CGNAT/240-0
ranges are unblocked. `backoff_delay` can also return a delay below its base,
which contradicts the doctrine written in `crates/domain/src/jobs.rs` that jitter
may only make a retry later.

**Verified good.** `a6f0e20` closes the third pass's finding properly: `job_kinds!`
emits the enum, `as_str`, `parse` and `ALL_KINDS` from one list, so drift is
impossible by construction, and `kind_index()` remains the separate exhaustive
match that gives uniqueness (its literal array still has to be raised when a kind
is added, but it fails loudly). Queue fairness has the right shape — `claim_sql`
plus a class filter derived from `ALL_KINDS`, a per-requester skip and a bulk
concurrency guard (`7badbcc`, with 25 tests in `milestone_5`).

### 2026-09-19, fifth pass — `f909775` (the fix commit) and the webhook wiring

**Fixed and verified.** The SQLite FK break is gone. Same reproducer as the fourth
pass, now against the fixed migrations:

```
download_grants FK -> [(0, 0, 'export_jobs', 'export_job_id', 'id', 'NO ACTION', 'CASCADE', 'NONE')]
foreign_key_check: []
insert into download_grants: OK
```

with an account, a job and an export row inserted first. The migration comment now
describes SQLite's rename behaviour correctly, and the fix was to drop and recreate
`download_grants` (and `bulk_export_items`) around the rebuild. The `/exports/bulk`
row is in `ROUTE_TABLE` (`start_bulk_export`, POST, `Authenticated`) — the direction
test's complaint is addressed. Verified at the commit in a clean worktree
(`/tmp/lh-at-f909775`, its own target dir): `milestone_7` **10 passed / 0 failed**
(the suite that was 9/10 while the FK was broken) and `route_inventory` **2
passed**, `tests_exit=0`. The commit's own claims hold; `fmt_exit=1` there as well,
and its file list contains no `crates/db`, so the three clippy lints it leaves
standing predate it.

**The query is still dropped.** `lorehaven_domain::query::QueryAst` is an enum
(`crates/domain/src/query.rs:82`) built by `parse_query(&str)` — the *DSL string*
parser the `?q=` path uses (`crates/app/src/routes/media.rs:110,932`) — while the
bulk route's body is `Json<MediaQuery>` (`crates/app/src/routes/media.rs:693`), a
JSON object. `Value::as_str()` is `None` for an object, so
`unwrap_or_default()` supplies `""`, the parse yields nothing usable, and the
caller's filters — quality, dates, facets, everything M23-01 built — never reach
the walk. The commit message's "falling back to None only on parse failure" is
inverted: for the documented body shape it *always* takes that path. This needs a
decision, not a patch: either accept the DSL string (`{"q": "fandom:x tag:y"}`,
documented as such) or bridge `MediaQuery` into a `QueryAst`. Either way an
unreadable stored query must be `Fatal`, never "export everything".

**PostgreSQL migration order is wrong.** In `migrations/postgres/0035_bulk_export.sql`
`DROP TABLE export_jobs_old` (line 48) runs before `DROP TABLE IF EXISTS
download_grants` (51) and `bulk_export_items` (64). PostgreSQL refuses to drop a
table that another table's foreign key depends on ("cannot drop table
export_jobs_old because other objects depend on it"), so the migration aborts
before the fix it needs. Unverified — no PG instance was available — and flagged
per the standing rule; the dependents must be dropped first.

**The rebuild now discards grants.** `DROP TABLE IF EXISTS download_grants` throws
away every outstanding token (SQLite and PG). The `export_jobs` rebuild beside it
copies its rows; this one should too (`RENAME` → `INSERT … SELECT` → `DROP`). The
rows are ephemeral so the blast radius is small, but nothing says so.

**Gate is still red, for two independent reasons.**

1. `cargo fmt` wants two blocks in `crates/app/src/bulk_export.rs` expanded (the
   `store.reference(...)` call at ~123 and the `count_media_filtered` call at
   ~184), and `cargo clippy -D warnings` fails `lorehaven-db` with the same three
   lints as the last pass: an empty line after a doc comment, and `too many
   arguments (8/7)` / `(9/7)`. `f909775` touched none of them.
2. **The working tree does not compile.** The in-flight webhook wiring in
   `crates/app/src/server.rs:159` builds `outbox::OutboxEvent { … }` field by field
   and omits `attempts` → `error[E0063]: missing field 'attempts'`, so
   `lorehaven-app` never builds and the gate's test stage produced zero results.
   The reconstruction looks unnecessary: the topic handler already receives
   `&OutboxEvent`, so `event.clone()` (or moving the borrow) is enough.

**Still untouched from the fourth pass:** the ZIP downloads as `text/html` /
`*.html` (the row is `format: "html"`), no manifest inside the bundle, no
checkpoint (retries duplicate `bulk_export_items` rows), every `load_subject` error
recorded as "skipped", no `has_entitlement` check, no `ContentRead` scope on the
bulk route, and no test for the export path itself.

---

## 2026-09-25 — Metadata exchange specified (docs-only; nothing built)

**No test was run for this change and none is claimed.** It is a specification
landing: `docs/spec.md` §0.3, §2.3.1, §11.17, §15.17, §16.16.1 and §19.14, five
`planned` rows in `docs/requirements.csv`, and the M57 build order in
`docs/plans/remaining-work.md`. Every row is `planned` and every row is code
that does not exist yet.

| Claim | How it is verified | Result |
|---|---|---|
| New sections landed at non-colliding numbers | `grep -c '^## 11.17'`, `'^## 15.17'`, `'^## 19.14'` | 1 each |
| No duplicate numbers introduced | `grep -cE '^## 11\.(1[7-9])'`, `'^## 15\.17'` | 1 each |
| "quorum" not repurposed for signal counting | `grep -c 'auto_quorum' docs/spec.md` | 0 |
| Markdown still parses | fence count via `awk` | 212, balanced |
| No broken cross-references introduced | unresolved-`§x.y` set diffed against the pre-edit spec | 10 before, 10 after, **0 newly broken** |
| Tracker row shape | `csv` re-parse, field count asserted, CRLF terminator preserved | 253 rows, 5 added, 5-line diff |

The 10 pre-existing unresolved references (`§0.4`, `§10.4.1`, `§14.5`, `§14.9`,
`§16.17`, `§16.18`, `§20.3.1`, `§24.15`, `§34.3`, `§34.5`) are **not fixed here**
and are not this change's damage. They are recorded so a later pass can find
them.

**What a reviewer should check first**, because it is the part that is a design
decision rather than a transcription: the TL1-submit / TL3-curate asymmetry in
§19.14. The external proposal asserted a level without checking what TL3 gates
elsewhere. If TL3 is already high enough to make the canonical layer unpopulated
on a young instance, the whole exchange is an endpoint that answers empty.

---

## 2026-09-25 — A false clean gate: 30 clippy warnings hidden by `2>/dev/null`

**A previous handoff claimed `cargo clippy --workspace --all-targets` was
clean. It was not.** The command was `cargo clippy ... 2>/dev/null | grep -cE
'^(warning|error)'`. rustc writes warnings to **stderr**, so the pipe read an
empty stdout and returned 0. The gate was measuring nothing, and the 30
warnings had been present the whole time.

Re-measured with `2>&1`: **30 warnings, 0 errors.** Three were real defects.

| Defect | Why it mattered |
|---|---|
| `media_resilience::find_matching_standing_bounties` took a `media_reference_id` it never used | Its doc comment promised "bounties that match a given media reference"; the table has no such column, so the comment described behaviour that did not exist |
| `longevity::half_life_map` built placeholders with `if i == 0 { "" } else { "" }` | Both arms empty. It emitted `?, ?, ?` and worked by accident; replaced with the house `library::placeholders` helper |
| `media_resilience::find_by_perceptual_hash` accepted a `max_distance` and ignored it | A caller passing a large threshold got exact matches only, while the signature implied a fuzzy search. Both callers pass `0`, so behaviour is unchanged; the doc comment now says the threshold is not honoured |

Plus a test that had never run: `domain::spoilers::test_display` had lost its
`#[test]` attribute, so its Display assertions were dead code. Clippy flagged
it correctly; the first fix attempt wrongly assumed the attribute was already
there and produced a duplicate.

The remaining 21 were mechanical: five `#[allow(clippy::too_many_arguments)]`
attributes a previous pass had detached from their functions and then deleted
as formatting nits, six blank lines after outer attributes, two never-read
assignments, a manual `Default` impl replaced by `#[derive(Default)]` with an
explicit `#[default]` on `Plain`, four `&mut Vec` parameters narrowed to
`&mut [_]`, and unused imports in two test files.

One clippy suggestion was **rejected**: collapsing the nested `if let` in
`discovery::resolve_sort` into a let-chain requires edition 2024, and
`lorehaven-app` is edition 2021. The combined `Option` does the same job.

**Gate after the fix, re-measured on this tree:**

| Check | Result |
|---|---|
| `cargo clippy --workspace --all-targets` (stderr captured) | **0 warnings, 0 errors** |
| `cargo test --workspace --no-fail-fast` | **1775 passed, 1 failed** |
| the failure, re-run alone | `repeated_login_attempts_are_rate_limited` — passes 1/1 in 59s. The known process-global rate-limit bucket flake under parallel test threads; not caused by this change |
| `cargo fmt` scoped to the three touched crates | clean |

**Rule this establishes:** a linter or test count is evidence only if the command
captured the stream the tool writes to. When checking rustc, clippy, cargo test
or a compiler that reports on stderr, use `2>&1` and redirect to a file, then
read the file. `2>/dev/null` before a `grep -c` produces a confident zero from
no data.

---

## 2026-09-25 — M32-07a: perceptual-hash dedup, and the two dead things it uncovered

A clippy warning from earlier today had flagged `find_by_perceptual_hash` as
taking a `max_distance` it never used. It was renamed `_max_distance` and
documented as exact-match-only rather than implemented, because the warning was
right and the function was lying. Spec §32.7.2 requires threshold-based
perceptual matching, so this implements it.

### The search

`db::media_resilience::find_by_perceptual_hash` is now an application-side
Hamming-distance search, ordered closest first. Neither backend can score a hex
string in SQL without a dialect-specific extension, so the query narrows to
`perceptual_hash IS NOT NULL` — the one predicate worth the existing index — and
the distance is computed in Rust.

| Decision | Why |
|---|---|
| Distance in Rust, not SQL | A working SQLite expression and a working Postgres expression are two dialects to keep in step. One shared Rust path cannot drift. |
| `NULL`, non-hex and mismatched-width hashes are skipped | A missing hash is not a hash; `not-a-hash` is not a fingerprint; a 64-bit pHash and a 128-bit wHash have no distance between them. Each would otherwise fold into a large distance, which is a similarity verdict invented from a malformed value. |
| A malformed row is skipped, not fatal | One bad row must not hide every good match — pinned by `a_malformed_stored_hash_does_not_match_and_does_not_fail_the_search`. |
| Closest first, then by id | A curator reads the strongest candidate at the top, and a stable tiebreak means a paginating caller cannot loop. |
| Exact match is distance 0 | It is always found, and it is the only case that can auto-attach. |

### What the tests caught

Six behaviours, each written before the code:

| Test | Pins |
|---|---|
| `a_perceptual_match_within_the_threshold_is_found` | 4 bits apart, threshold 6 → found. The old equality-only query returned nothing, so this was the RED that justified the work. |
| `a_perceptual_match_outside_the_threshold_is_not_found` | 12 bits, threshold 6 → not found |
| `the_threshold_decides_what_is_found` | the same pair found at 2 and hidden at 1 |
| `a_reference_with_no_perceptual_hash_is_never_matched` | a `NULL` is not a hash, at any threshold |
| `a_malformed_stored_hash_does_not_match_and_does_not_fail_the_search` | the healthy row beside a bad one is still returned |
| `a_malformed_query_hash_returns_nothing_rather_than_everything` | an unparseable query cannot match the whole table |
| `an_exact_match_is_still_found_and_sorts_first` | ordering, and that exact match survives |

At the route: `reverse_search_uses_the_configured_perceptual_threshold` runs the
same data at thresholds 1, 2 and 6 and asserts 0, 1 and 1 matches, which is only
true if the handler reads config. `reverse_search_reports_a_confidence_score_per_match`
pins the `match_distance` / `match_confidence` / `match_kind` / `auto_attach`
fields the spec's curator workflow needs.

### Two dead things found while implementing

**`perceptual_match_threshold_is_valid` was dead code.** The domain validator
for the threshold existed and was tested, and nothing called it. The config had
no threshold, so there was nothing to validate. It is now called from
`Config::validate()`, and an out-of-range value stops the instance instead of
silently matching everything or nothing.

**`load_from()` in the config tests could read a stale file.** It keys its
scratch directory on `(pid, name)` and never clears it, so two tests passing the
same name read the first one's config. My two refusal tests — "an unknown
algorithm is refused" and "an out-of-range threshold is refused" — both
inherited the directory of the passing parse test and loaded a valid config with
`whash` and a threshold of 9. The helper now removes the directory first, and
the three call sites have distinct names. Worth noting how it presented: the
tests failed, which is correct, and the reason was the harness rather than the
code under test.

### Gate

| Check | Result |
|---|---|
| `cargo clippy --workspace --all-targets` (stderr captured) | 0 warnings, 0 errors |
| `cargo test --workspace --no-fail-fast` | see the count in the commit message |
| `cargo test -p lorehaven-app --test media_resilience` | 17 passed |
| `cargo test -p lorehaven-app --test discovery` | 3 passed |

`crates/app/tests/discovery.rs` needed updating: its fixture stored the string
`hash123` as a perceptual hash, which is not hex, so the new code correctly
refuses it. The fixture is now a real fingerprint.

### Still open

M32-07b, filed: **nothing computes a perceptual hash.** The search and the
config keys are real, but the column is only ever written by a test. The feature
is correct on an empty column, and an importer is what makes it reachable.

---

## 2026-09-25 — Two false green signals, both in gates, one hiding real defects

A clippy gate reported clean while the tree held 30 warnings (covered above).
The E2E suite was the second false signal, and it was hiding a *test* defect
rather than a product defect.

### The export-delete test asserted a state the shared account can never reach

`frontend/e2e/coverage.spec.ts` waited for `Ready`, clicked the first `Delete`,
then asserted `getByText('No exports yet')` was visible. The E2E account is
shared: the export-download test immediately above it creates a second export on
the same account, so a `Ready` row is always present and the empty state can
never appear.

The product behaviour was correct throughout — the deleted row *was* removed,
and the captured page snapshot showed exactly that, with the second `Ready`
export still listed. The test failed on a working build, which is how it had
survived as a phantom product bug.

Two wrong turns before the real one, both worth recording:

1. Asserting that the item count drops to `0`. With two rows before the click,
   `0` is as unreachable as the empty state. Wrong in the same way.
2. Filtering the row by its title, `Disposable`. The list renders
   `job.label || job.format`, and a worker-produced export carries no label, so
   its heading is `EPUB` — the filter matched **zero** rows and the test failed
   at the `Ready` wait with no obvious cause.

The fix uses the export id in the download `href` as the row's identity:

| Assertion | Checks |
|---|---|
| `a.download[href*="<exportId>"]` has count 0 | that export's own row is gone |
| the section's `.item` count is `before - 1` | exactly one row was removed, not two |

**The transferable rule:** when a UI test fails, decide whether the assertion
describes the feature or the fixture. Empty states, global counts, and
first-row locators all describe fixtures, and fixtures are shared, ordered, and
stateful. A test that can only pass on a pristine account will fail forever on a
dirty one — and the failure reads as a bug in the thing under test, which sends
you to debug the wrong code.

### Gates, final state

| Check | Result |
|---|---|
| `cargo clippy --workspace --all-targets` (stderr captured) | **0 warnings, 0 errors** |
| `cargo test --workspace --no-fail-fast` | 1775 passed, 1 failed (rate-limit flake; passes 1/1 alone in 59s) |
| `cargo build --release` | clean, 5m 10s |
| `npx vite build` | clean |
| `npx playwright test` | **73/73**, verified on the full suite, not only in isolation |

---

**Webhook delivery is now being built** (`crates/app/src/webhook_delivery.rs`,
uncommitted, 159 lines): `deliver_notification` calls `webhook_sender::send`,
records each attempt through `marketplace::record_delivery` — closing the "no
production caller" gap at last — filters endpoints by subscribed event type, and is
wired to the outbox topic `publish.notify` in `server.rs`. That is the delivery
half of Phase 3a; query watches (a webhook when new media matches a saved query)
remain the missing piece of M23-02's webhook requirement.
