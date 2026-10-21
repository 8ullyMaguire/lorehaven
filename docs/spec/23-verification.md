# 23. The verification discipline

**Status:** reserved
**Milestone:** M1
**Source:** the corresponding section of the Darkhold specification

**The most important section in the document.** The product is the easy part.

## 23.1 The ledger is the authority, and it is re-derived

`docs/requirements.csv` is the row-by-row authority for what is built. Every status document
carries the command to recompute it, and **the number is re-derived, never believed**. A status
document rots faster than the code it describes, because nobody re-derives it.

**Columns in use today:** `id, area, requirement, milestone, status, evidence, notes` — **seven**,
and every one of the 710 rows has exactly seven fields.

`verified_at` and `gates` are **specified and deferred**. They are worth having — a row that
names its own gate is a row that can be re-verified without a reader reconstructing the
command — and they arrive when a **deliberate migration** populates them, which is its own
commit, because populating 703 inherited rows means knowing each one's gate command and
**guessing would be fabricating the evidence the column exists to record**.

**Until that migration, the ledger's writer preserves the file's header verbatim.** That is not
a convenience; it is the rule, and M0 found out why the hard way.

> **The defect M0 found in the ledger's own writer.** The writer appended the canonical
> columns the file was missing, so editing **one** row grew all 703 inherited seven-column rows
> to nine — two empty fields on every single one — and the diff was **704 lines removed and 711
> added**. The module's own header says *"a ledger that has not yet gained a column does not
> gain one because somebody edited a row,"* and its implementation did exactly that.
>
> **And the test that should have caught it asserted the opposite of its own name.** It was
> called `test_header_is_preserved`, its docstring said *"a ledger with seven columns must not
> silently grow to nine,"* and its assertion required the header to be exactly
> `…,verified_at,gates`. It passed, against a writer doing the wrong thing, and it read like
> coverage.
>
> **A test whose assertion contradicts its own name is worse than no test**, because it buys
> the confidence of coverage while checking nothing. The rule this estate follows is the one
> the failure teaches: **the assertion is the rule, and if the two disagree, the assertion is
> what is wrong** — because the name was written from the rule and the assertion was written
> from the code.
>
> Both are fixed. The header is preserved verbatim, and the test asserts what its name says.

| status | meaning |
|---|---|
| `implemented-fully-tested` | the code runs, the tests drive the real path, the evidence is recorded |
| `implemented-locally-tested` | built and unit-tested, but **not held to the full-stack bar** |
| `implemented-verified-e2e` | verified end to end |
| `planned` | not built — and it has a spec section first |
| `unsupported` | consciously declined — a standing exclusion, never counted as remaining |
| `evaluated-and-rejected` | evaluated and rejected, with the reason recorded |

**The `evidence` field must name the test.** A plan document cannot prove that a thing renders.
**The `notes` field is provenance, not description.**

**A requirement is complete when it has code that runs, tests that drive the real path, and
evidence recorded.** It is *not* complete because: a field no configuration can set; a migration
that adds a column no query reads; a unit test on a function nothing calls; a test asserting a
value the spec never promised.

**"A row whose machinery is unreachable — nothing writes the table, nothing enqueues the job — is
not tested by unit tests on the dead functions."**

## 23.2 The gates

| gate | command | when |
|---|---|---|
| fmt | `cargo fmt --all --check` | always |
| clippy | `cargo clippy -p <touched> --all-targets` | always, no diagnostics (workspace `warnings` denied) |
| SQLite | `cargo test -p <touched> --no-fail-fast` | always, for schema or query changes |
| PostgreSQL | the same, with the PG URL set | **for anything touching SQL** |
| doctests | `cargo test -p <touched> --doc` | **standalone — never mixed with target flags** |
| frontend | `svelte-check` + vitest | anything user-facing |
| E2E | Playwright, **one suite at a time** | anything user-facing |
| budget | `make budget` | anything touching the taste engine |
| tier parity | the T0/T1/T2 engines on the reference corpus | anything touching ranking, tiers or the arena |
| catalogue parity | the taste engine's output against the reference numbers | anything touching the graph or the embedder |

**A result is green only when the exit code is zero.** `2901 passed, 0 failed` with exit code 101
is a *failed* run.

**The gate reads the exit code, not a tally.** Counting `test result:` lines is blind to a target
that never ran.

**Redirect with `> /tmp/x.log 2>&1` and read the file.** `rustc`, `clippy` and `cargo test` report
on **stderr**; `cargo clippy ... 2>/dev/null | grep -cE '^(warning|error)'` returned a confident
zero from an empty stdout while the tree held 30 warnings — three of them real defects, including
a test that had never run. **Never pipe a gate through `grep -c` without `2>&1`.**

**If a gate has never once failed, suspect the gate before you trust it.**

## 23.3 Two engines, both gated

**"The second engine is not only a deployment target, it is the cheapest available check on the
first one."**

The proof: lorehaven's `set_password_hash` bound one parameter list for two statements with
different placeholder orders. On SQLite this was **silent** — the surplus parameter shifted every
value, the `WHERE` compared an account id against a timestamp, no row matched, and the function
returned `Ok(())`. **A password change did nothing, reported success, and every test passed.**
PostgreSQL refused it outright.

**Verify dialect parity, not symmetry.** The known PG-only traps, each with a check script: INT4
decoded as i64 (`CAST(1 AS BIGINT)`), `ALTER TABLE ADD CONSTRAINT` needing a newer SQLite than the
bundled one, `ESCAPE ''`, `TIMESTAMPTZ` comparison, TEXT/UUID pseudo-FKs, and partial-index
`ON CONFLICT` predicates.

**A PostgreSQL statement carries one numbering scheme.** The dialect helper renumbers every
`?` to `$1`, `$2`, ... and leaves a hand-written `$n` exactly where it is. A statement that
mixes the two spellings therefore has two numbering systems in it, and the helper reconciles
them only when the counts happen to agree. When they do not, **the failure is silent** — every
SQLite test passes, because SQLite never sees the PostgreSQL half — and the rewrite makes one
number name two columns. The Darkhold `create_work` insert shipped in exactly that shape, and
the work's discussion mode came back as the wrong one.

So the rule is stated positively, and it is the house convention rather than a prohibition:

- the PostgreSQL half carries **one** scheme — every bind a `?` (the helper numbers them) or
  every bind a `$n` (the helper leaves them alone);
- the PostgreSQL half may not use SQLite's **numbered** `?1` form, because the helper turns
  the `?` into `$1` and leaves the digit, so `?1::uuid` becomes `$11::uuid` — a parameter
  eleven, in a statement that binds one.

**A gate that reads the schema must follow the migrations as a sequence.** A table is created,
then possibly renamed, then possibly dropped, and the name the instance uses *now* is the last
of those. A checker that reads only the `CREATE TABLE` statements describes a schema that never
existed, and the two failures look opposite and are the same mistake: one reports **correct**
statements as faults, and the other **skips** the statements it cannot place and looks clean.

**The bundled SQLite version matters.** The sqlx build bundles SQLite 3.46.0, not the system
library. A script reads the version from `Cargo.lock` and the vendored header, so a lockfile bump
retires the rules itself — and **exits non-zero if it cannot read the version**.

## 23.4 Mutation gates

A mutation must kill **exactly one** test.

- "A kill that takes down everything is a broken artefact; a kill that takes down one thing is a
  pinned property."
- Every refusal test gets a **positive control** — `a_signal_with_a_valid_origin_is_accepted`.
  Without it, "refused" proves nothing.
- **Fixtures must clear every *other* gate**, or the gate under test is untested.
- **Solve for the comparator:** find a pair where the correct and the mutated behaviour land on
  opposite sides, and assert the *order*.
- **Prove guards by breaking them.** Green is not evidence until broken.
- **Encode and decode are a pair.**

**The theme and mood gates have their own mutation requirements, because they are the easiest
things in the design to get wrong:**

- A mutation that makes a **mood filter instead of reweight** must be killed by a test that
  asserts the *set* of reachable works is identical with and without the mood.
- A mutation that makes the **house taste visible in another reader's profile** must be killed by a
  test asserting the absence of the theme's tags from a non-owner's public profile payload.
- A mutation that makes a **Supporter subscriber rank above a non-subscriber** must be killed by a
  test asserting the ordering of two works with equal scores, one seen by each.
- A mutation that makes a **theme's voted weights replace the owner's seed entirely** must be
  killed by a test asserting that with zero votes, `effective_weight(t) == seed_weight(t)` for
  every tag, exactly.
- A mutation that removes the **per-period rate limit** (§12.12.4) must be killed by a test
  asserting that a theme's applied weight moves at most `max_shift_per_period` of the remaining
  distance in one period.
- A mutation that makes a **theme's membership votable** (adding a tag by vote rather than by
  quorum) must be killed by a test asserting that a vote cannot add a tag to a theme.

## 23.5 The reachability probe

**Look for the consumer of the data, not the producer.** `grep -rl <term> frontend/src` answers
"does this string exist", not "is this mounted". The tracker said something was unbuilt while it
was shipped and mounted — five times.

- **A route-inventory test must assert in both directions.** Server → table is the easy half. The
  hard half is: *every `pub fn …router()` in `routes/*.rs` must appear in the composition root.*
  lorehaven's inventory test checked server → table and never the reverse, so a declared router was
  never merged and **five endpoints returned 404 for the lifetime of the feature**, with unit tests
  passing.
- **Every table in the schema must have a writer.** A table something writes and nothing reads is
  worse.

## 23.6 Known gaps are pinned, not papered over

`docs/known-gaps.md` — every open defect has a **test asserting the current behaviour**, so a fix
turns it red rather than passing silently.

## 23.7 The QA harness

- **The route walk parses the router source**, so the route inventory can never drift. It asserts
  the auth gate on every route and flags any 5xx or stub response.
- A **zero-token, deterministic** bug queue: console, network, HTTP, OPDS and link checks, with
  fingerprint deduplication, a doctor gate, repro and verify gates, work packets, and convergence
  metrics.
- A **shell exit-code contract** (0 clean · 1 bugs remain · 2 no progress · 3 environment failure ·
  4 flaky · 5 autofix rejected) so it is cron-friendly.

## 23.8 The manual gates that no test replaces

- Open the instance in a browser. Read a work. Search for a phrase in a body. Compare two works in
  the arena. Block a tag and unblock it. Name a mood, use the site for a while, and come back to
  the mood. Sign in as a guest, then fold the guest. File a report and watch the jury draw.
  **The chain is client-side steps through several handlers and a template; no unit test covers it.**
- The two kinds of failure a test cannot find: **"a button that looks enabled, is enabled, is wired
  to a real form, and does nothing"** (kindred's inert recommendation button — every one of ten was
  a no-op, and it took a real click), and **"a page that is 200 with one byte"** (lorehaven's
  template set collision).

## 23.9 The standing rules

- **No stubs.** Every branch returns a real value or refuses.
- **Evidence, not status flips.** Record every passed test in the ledger **in the same commit as
  the code.**
- **Corrections land in place, in the same commit** as the code that made them wrong.
- **A genuinely ambiguous decision gets an ADR.**
- **A plan nobody corrects is worse than no plan.**
- **Release tags are the owner's decision alone.**
- **Never fabricate a result.** A blocker reported honestly beats invented output every time.
- **The full suite runs once at the end of a task.**
- **One E2E suite at a time** — they share a database.
- **Never `git add -A` while a mutation gate is running**, and never run a test binary while a gate
  is running.
- **A mass rewrite is not done until `git ls-files --others --ignored --exclude-standard` and the
  build have both run** (§23.10.1). **A rewrite only reaches committed blobs**, and an ignored
  file is not a committed blob.

## 23.10 Two things the rename taught about the gates

Both of these were found by M0 and neither was in the spec before it. **They are recorded here
because a rule that is only in somebody's head is not a rule.**

### 23.10.1 A content rewrite only reaches committed blobs

The rename rewrote the history with `git filter-repo --replace-text`. The rewrite is exact — every
committed blob was rewritten, and the verification proved it: **zero tracked files** carried the
old name, and the build was green.

**And it did not touch `crates/app/tests/m53_source_credentials.rs` — 36 KB of tests that had been
untracked since 2026-10-04**, because the *global* gitignore at `~/.config/git/gitignore` contains
`*credentials*` and the repository's own `.gitignore` negated it only for
`migrations/**/*credentials*.sql`. The file was never in a commit, so it was never rewritten, and
the build died on `use of unresolved module or unlinked crate 'lorehaven_db'`.

The consequences, in order:

1. **Every checkout except the author's machine was missing those tests**, and `git add -A`
   reported success the whole time. **The suite's size was a machine-local fact.**
2. **The rename failed to compile** — and the failure pointed at the rename, not at the trap.

**So there are two checks, and a mass rewrite is not done until both have run:**

```bash
git ls-files --others --ignored --exclude-standard   # the files a rewrite cannot reach
cargo build --all-targets                             # the check that finds the gap
```

**A rename that is not followed by a build is a rename that has not been verified.** And the
repository's own `.gitignore` is now the place that says so, with a per-file negation —
`!crates/**/*credentials*.rs` — **because the global rule exists to do its job for actual
secrets, and loosening it would be the wrong fix.**

### 23.10.2 A rewrite that cannot be re-run cannot be checked

`git filter-repo --replace-text` has **no comment syntax.** A line in the rules file is a
rule, and a line with no `==>` is a rule that removes the text it contains — so **a line that
is exactly `#` removes every `#` from every blob in the repository.**

M0 found this by re-running the rewrite against the pre-rename bundle to check that it was
reproducible, and comparing the result with the committed tree. The re-run produced a tree in
which every comment in the repository had become `***REMOVED***`. **The tree was caught
before it was committed**, because the verification compares the re-run tree against the
committed tree rather than against an expectation.

**So the rules are now in a bare file and the explanation is beside it**
(`scripts/rename-replacements.README.md`), and the single command reproduces the committed
history exactly — the same 1,048 SHAs, the same 1,265 blobs, the same tip.

**A rewrite that cannot be re-run is a rewrite that cannot be checked.** The bundle is what
makes it re-runnable, and re-running it is what makes the bundle worth having.

### 23.10.3 A rename changes identifier lengths, so the formatter must run

`cargo fmt --all --check` is an "always" gate (§23.2), and M0 ran the rename without it.

**The rename left 27 files failing the formatter, and the pre-rename tree was clean** — so the
rename is what broke it. The mechanism is not subtle: **rustfmt's line-wrapping decisions are a
function of identifier length**, and a rename changes the length of every identifier it touches.

```
lorehaven_db::identity::AccountStatus::Active   # one shape
darkhold_db::identity::AccountStatus::Active    # a different shape, wrapped differently
```

**A rename is not finished until the formatter has run**, because the formatter's opinion is
part of the build, and "the build is green" is not true while `cargo fmt --all --check` is
red. This is now in the plan's T-M0-04 and in its completion gate.

### 23.10.4 A gate that has never run is not a gate

**The PostgreSQL arm of the suite had a clone path that had never once executed.**

`connect_with_dir` created the scratch database, and *then* called `cached_postgres`, which
clones by running `CREATE DATABASE {name} TEMPLATE {template}` — against the same name. The
clone therefore always failed with *"database already exists"*, the caller fell through to
migrating the empty database in full, and **every PostgreSQL test replayed 107 migrations.**
The arm took hours. Under load the admin pool starved, the acquire timeout expired, and suites
failed with *"pool timed out while waiting for an open connection"* — a message that reads like
a resource limit and is a logic error.

**And a second defect compounded it.** The template is built under a lock, but the check for an
existing template happened *before* the lock was taken, so two threads could both see `None`
and both build. The second one's build starts with `DROP DATABASE IF EXISTS {name}` — so it
**dropped the template the first one had just built**, while the first one's siblings were
cloning from it. PostgreSQL said *"template database does not exist."*

Both are fixed: the clone is attempted **first** and the empty database is the fallback, and the
template check is repeated **under** the lock. The measurement, on the file that failed:

| | before | after |
|---|---|---|
| `m57_metadata_exchange` on PostgreSQL | **146.83s**, 1 test failed | **55.71s**, 41 passed, 0 failed |
| the PostgreSQL log | `database already exists`, `template database does not exist` | neither |

**The rule this teaches: an optimisation that has never executed is not an optimisation, it is
a branch nobody has run.** The clone path was correct code in a place that could never be
reached, and the only reason it was found is that the gate had to be made green.

### 23.10.5 A gate that reads a tally is not a gate

`cargo clippy ... 2>/dev/null | grep -cE '^(warning|error)'` returned a confident **zero** from an
empty stdout while the tree held 30 warnings — three of them real defects, including a test that
had never run. `rustc`, `clippy` and `cargo test` report on **stderr**.

So `scripts/gate.sh` reads **the exit code**, writes every gate's output to `target/gate/`, and
prints the log's tail on failure. **No gate is ever piped through `grep -c`.** A tally of `test
result:` lines is blind to a target that never ran, and the clippy case is worse: the tally was
confident, green, and wrong.

And three gates **refuse** rather than pass:

- the **PostgreSQL** gate exits 3 when the cluster is not reachable, because a gate that quietly
  does nothing is worse than no gate — the record would say it passed;
- the **budget** gate exits 1 when the catalogue is unreadable, because **"measuring nothing is
  not a pass"**;
- `gate.sh taste-budget` exits **2** until M13-09 builds it, because **a gate that did not run
  must never report success**.

### 23.10.6 A gate that runs out of room has measured nothing

The harness's scratch directories are **leaked**: `TestDb` has no `Drop` that removes the
directory its SQLite file lives in, and `std::env::temp_dir()` is not swept. A measured run left
**1,399 directories, 6.9 GB**.

On the development host `/tmp` is a **16 GB tmpfs**, so the suite fills the filesystem and the
failures arrive as **`(code: 13) database or disk is full`** — which reads like a storage problem
and is a tmpfs problem. **Twelve suites failed that way** before the directories were swept.

**A gate that fails because it ran out of room has measured nothing**, and a failure that reads
as a disk problem sends the reader looking at the disk. The gate runner now points `TMPDIR` at
the real filesystem and sweeps the stale directories before a run. That is a **workaround**: the
directories are still created and still left behind, and the defect is recorded in
`docs/known-gaps.md` (M1-D01) and pinned by a test, so a fix turns the pin red.

### 23.10.7 The rate limiter's buckets are a process-global static

The limiter's bucket map lives in a `static` so that every `AppState` clone shares it — a
per-`AppState` map would let a credential-stuffing loop find a fresh bucket on every request.
**That decision is right, and it has two consequences the spec now names.**

1. **Every test in one test binary shares one auth quota.** The address bucket is a shared key
   (`ip:auth:unknown`, because a test router has no `ConnectInfo`), so a file that registers
   thirteen accounts across thirteen tests spends a forty-token budget long before the last test
   runs, and **the failure lands on whichever test the scheduler reaches next** — not on the one
   that caused it. M0 hit this exactly: the red test was
   `a_forbidden_name_in_the_never_shown_list_is_a_404_at_every_level`, and the cause was twelve
   *other* tests in the same file. **Three files in the repository have needed
   `limiter::clear_buckets()` in their setup for this reason, and the third one was missed for
   weeks.** A test file that registers accounts must clear the buckets, and the comment must say
   why — the estate's own comment in `m54_bot_link.rs` records the exact incident.
2. **A multi-process deployment multiplies the effective limit.** The bucket map is per
   *process*, so *N* workers enforce *N* times the configured rate. **One process, or a shared
   limiter.** This is a deployment decision and it is recorded here so that M32's Pi deployment
   states which one it is.

## 23.11 The status vocabulary for the reader

Because "built" and "works" are different claims:

- A page that is live shows real values from the server.
- A route that is linked but unbuilt **says so plainly**, with the row in the ledger.
- **No screen displays mock data.**
- A feature's state is discoverable from the API (`/api/v1/meta` carries the instance's feature
  state, the published theme, and the tier thresholds).

---
