# Lorehaven — what's left

Updated at the start of each turn. Last commit: `8b4e58e`.

## Done this project

| Gap | Feature | Where | Tests |
|---|---|---|---|
| G | §20.3 author payout multipliers | `crates/domain/src/payouts.rs` | 12 |
| G | §20.3 payout store (signals → ledger) | `crates/db/src/payout_store.rs` | 14 |
| G | §20.3 weekly recalculation job | `crates/app/src/payout_recalc.rs` | 12 |
| D | Series-aware recs (first *unfinished* entry) | `crates/db/src/series_recs.rs` | 17 |
| E | Earned-bookmark ratio + §53.6 definition | `crates/domain/src/earned_bookmark.rs` | 13 |
| F | Hidden-classics rec strategy | `crates/db/src/rec_strategy.rs` | 12 |
| B | Blind Date daily surface | `crates/db/src/discovery.rs` | 12 |
| C | §23.7 provider interface | `crates/domain/src/ai.rs` | 7 |
| C | Per-dimension pre-read report | `crates/domain/src/preread.rs` | 8 |
| C | OpenAI-compatible adapter | `crates/app/src/ai/` | 18 |
| C | Report persistence (migration 0111) | `crates/db/src/preread_store.rs` | 12 |
| C | Author-only pre-read route (§32.6) | `crates/app/src/routes/preread.rs` | 10 |
| C | Author-only panel, editor-only | `frontend/src/lib/components/PreReadPanel.svelte` | 6 |

Plus, as incidental fixes found by the above: migration 0110 FK divergence,
`hit_rate.rs` migrating the shared postgres database, a `kind_index` collision,
a race in the test-support schema cache, a `CostQuote::is_priced()` that reported an
unpriced local-model quote as priced, a `TestDb::applied_migrations()` that returned an
empty set for a fully-migrated database (8 suites), and a Blind Date seed that ignored the
day so every reader got the same work forever.

## Remaining ranked gaps (from docs/plans/100-ideas-audit.md)

All six of the audit's "real gaps" are now closed — A, B, C, D, E, F, G. Gap C took the
longest and is described below.

## Gap C is closed

All six steps are done and tested on both engines. What it consists of:

| Step | What |
|---|---|
| 1 | `AiProvider` trait, `AiAbstain`, `CostQuote`, `AiConsent` |
| 2 | `PreReadReport` — per-dimension, with the abstain path |
| 3 | The OpenAI-compatible adapter (also covers Ollama) |
| 4 | Persistence, migration 0111 |
| 5 | Three author-only routes |
| 6 | The editor panel — **a component, not a route** |

The step 6 decision worth keeping: there is no `/preread` path and no route id, so there
is nothing to link to or share. §32.6 says the report is never on the public work page, and
the cheapest way to guarantee that is for the surface not to exist as an address. The panel
renders inside `WorkEditor`, which already branches on ownership, and a 404 makes it render
nothing at all rather than an error message — a "not found" on somebody else's work would
confirm the draft exists, which is exactly what the server's indistinguishable 404 exists
to prevent.

**All six gaps (A–G) are now closed.**

## In flight

- Full SQLite suite on a verified-clean tree (`proc_6ab45cebe2c5`).
- Both mutation gates on a clean tree, **after** the suite finishes — they edit the source
  under test, so running them concurrently is what produced the false results this session.
- Full PostgreSQL suite, last.

**Do not run any of these concurrently, and do not touch the tree while they run.** Four of
this session's bad results came from doing so.

## Also outstanding

- 31 rows in the M45 tracker still marked `planned`. This is the largest remaining pool of
  named work in the repo.
- `preread_reports` exists and is tested, but **no provider is configured**, so nothing
  writes a row in practice. Running an Ollama instance and pointing the adapter at it is the
  last manual step; §23.7's "AI features disabled without configuration" is why that is a
  valid state rather than a gap.
- The §0.3 "three TEXT/uuid columns" note should become six: `rec_strategy.rs` and
  `discovery.rs` added two, `preread_reports.work_id` is the sixth site. Each needs its own
  dialect arm and its own `work_uuid()` helper.
- `docs/plans/100-ideas-audit.md`'s gap F section is already marked CLOSED — the note
  saying otherwise was stale and has been removed.

## Decisions worth keeping

**The AI trait uses boxed futures, not `async fn`.** A native `async fn` in a trait
cannot state `Send` on the returned future, so a provider holding a `reqwest::Client`
could not be awaited in a spawned worker task — which is where every caller belongs.

**The adapter is deliberately narrow**: reachability, a quote, a run, and what was
charged. Four capabilities is one `POST /v1/chat/completions` plus one
`GET /v1/models`, which is why a stub server is sufficient verification. Ollama and most
local runners expose the same API, so they get this adapter rather than their own.

**The adapter was verified against a real axum server on a loopback port**, not a mocked
`reqwest`. Mocking would verify that the adapter calls the methods the mock expects — a
statement about the test, not the adapter. Binding a port also catches what a mock cannot:
a URL built wrong, a body field misspelled, a status treated as success.

**A quote is unpriced rather than guessed.** Nothing in the code knows what a token costs,
because §23.7 gives that to the administrator. So `CostQuote` has an explicit `priced`
field rather than inferring it from a non-empty `unit` — a local Ollama quote has unit
"tokens" and no price, and the first version let §22.11's budget guardrail through on a
figure that was not money.

**Three things an adapter must never do**, each a §23.7 requirement rather than a style
choice: return unvalidated model output (a chat completion is prose until parsed against a
schema), send private text without consent, and report a transport failure as an empty
result (an empty list reads downstream as "the work was assessed and scored nothing").

## Lessons from the mutation gates (cost real time, keep these)

- **A test that passes for the wrong reason is a GREEN(BAD) with extra steps.**
  `an_unpublished_work_is_never_offered` seeded a draft with no completions, so
  `completions > 0` excluded it and the lifecycle clause was never exercised.
  Fixtures must clear every *other* gate, or the gate under test is untested.
- **Presence assertions don't kill ordering mutations.** The log-vs-linear mutation
  survived because the test only asserted both works appear; a linear denominator
  also returns both. Assert the ORDER, with a pair the two denominators disagree on.
- **Solve for the comparator.** Guessed comparators hold under both the real query
  and the mutation. Search for a pair where correct and mutated land on opposite
  sides, then use it.
- **A compile error is only a valid RED if the mutation was meant to compile.**
  Removing a `format!` placeholder makes the build fail — the tests never ran. Keep
  such mutations balanced (`OR 1 = 0`) so they genuinely change behaviour.
- **Scope mutation patterns.** `AND w.lifecycle = 'published'` matches the first
  sibling strategy in the file, not the one under test, so the result is meaningless.
- **Neither engine has a portable SQL function**: no `md5` on SQLite, no `log10`, and no
  shared JSON function. Compute the value in Rust and order there.
- **Prove guards by breaking them.** Removing the range check turns exactly
  `a_stored_score_out_of_range_is_dropped...` red; removing `ON DELETE CASCADE` turns
  exactly `deleting_a_work_deletes_its_reports` red. Green is not evidence until broken.

## Lessons from the store (cost time too)

- **Encode and decode are a pair, and testing only one half passes.** `encode_missing`
  wrote `{"abstained": "NotConfigured"}` (the `Debug` of the enum) while `decode_missing`
  read a bare string. The object form matched no case, so every abstain came back as
  `InvalidOutput` carrying the JSON text as its reason. Store the variant *name*: an
  `AiAbstain` carries a provider's error text that has no business being persisted as a
  type tag, and re-parsing a `Debug` dump is fragile.
- **A fixture writing the wrong JSON shape makes a test pass for the wrong reason.** The
  corrupt-row tests wrote `{"tone": 0.7}` where the store writes `{"tone": {"score": 0.7}}`,
  so the whole row decoded as empty. The "length is absent" assertion passed anyway; the
  "tone still loads" assertion is what caught it.
- **In a Rust string literal, `\` + newline strips the newline and `\\` + newline keeps a
  literal backslash.** The doubled form put `\` into the SQL at every wrap point and SQLite
  rejected it as an unrecognized token.
- **A test file that is uncommitted is one bad `open(p, "w")` from gone.** Two files were
  zeroed this way before being caught. The `write_file` tool refuses to overwrite a file
  changed since the last read, which is the only reason one was recoverable.
- **Never `pkill -f 'cargo test'`.** It kills the mutation harness mid-write and leaves a
  half-written source file (`.then_with(...)//! Recommendation strategies...` prepended to
  line 1 of `rec_strategy.rs`). Use `process(action='kill')`, which kills the shell rather
  than its children mid-write.
- **A SKIP from every mutation in a run means the file is broken, not the patterns.** The
  blind-date harness had `B1 date removed from seed` targeting `blind_date_seed(account,
  today)` and reported SKIP — correctly, because a contaminated commit had changed that
  exact line to `blind_date_seed(account, "")`. The harness caught a bug I had shipped.
- **Do not explain away a failure that matches what you expected.** A 5-second suite taking
  128 s was the same signal that had just correctly flagged a mutated file, and I attributed
  it to a race instead. Check the runtime before explaining a red test.
- **A stale GREEN(BAD) is worse than no result.** A batch of mutation-gate output arrived
  from harness revisions that had since been rewritten; several mutations were reported
  GREEN(BAD) that were in fact RED. Re-run the single mutation by hand before believing it —
  and restore the source afterwards, since the harness restores by string-replace and a
  concurrent edit breaks the match.
- **An empty result and a missing entry look the same in the message.** `TestDb::
  applied_migrations()` returned `[]`, which reads like "migration 0104 missing" and sent the
  investigation at migration 0111 rather than at the accessor. `--nocapture` printing the
  count distinguished them immediately. A count of 0 means the accessor is broken; a count of
  N with one entry missing means the migration is.
- **Never `git add -A <dir>` while a mutation gate is running.** The gate's mutations sit in
  the working tree between its edit and its restore, so a broad add stages whichever one is
  applied at that instant. This contaminated a commit: "Close the H9 GREEN(BAD)" also carried
  `blind_date_seed(account, today)` → `blind_date_seed(account, "")`, making every day return
  the same work. Stage explicit paths, and read `git show --stat HEAD` after committing.
- **Never run a test binary while a mutation gate is running.** The gate edits the file
  under test, so a concurrent `cargo test` compiles mutated source and reports the gate's
  intended RED as a failure. Check `git status --short <file>` before believing any failure;
  a modified source plus a red test says nothing about your code. (Observed as a clean-tree
  `1 failed` on `HEAD`; an 8-second suite taking 259 s was the tell.)
- **This project has no `_sqlx_migrations` table.** It has its own migration runner, so
  `MigrationReport::already_applied` is the authoritative record of what is applied. Reaching
  for sqlx's ledger is a dead end that compiles and fails at runtime.