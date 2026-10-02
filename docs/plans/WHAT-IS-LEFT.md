# Lorehaven — what's left

Updated at the start of each turn. Last commit: `51b0295` (gap C step 4).

## Done this project

| Gap | Feature | Where | Tests |
|---|---|---|---|
| G | §20.3 author payout multipliers | `crates/domain/src/payouts.rs` | 12 |
| G | §20.3 payout store (signals → ledger) | `crates/db/src/payout_store.rs` | 14 |
| G | §20.3 weekly recalculation job | `crates/app/src/payout_recalc.rs` | 12 |
| D | Series-aware recs (first *unfinished* entry) | `crates/db/src/series_recs.rs` | 17 |
| E | Earned-bookmark ratio + §53.6 definition | `crates/domain/src/earned_bookmark.rs` | 13 |
| F | Hidden-classics rec strategy | `crates/db/src/rec_strategy.rs` | 11 |
| B | Blind Date daily surface | `crates/db/src/discovery.rs` | 8 |
| C | §23.7 provider interface | `crates/domain/src/ai.rs` | 7 |
| C | Per-dimension pre-read report | `crates/domain/src/preread.rs` | 8 |
| C | OpenAI-compatible adapter | `crates/app/src/ai/` | 18 |
| C | Report persistence (migration 0111) | `crates/db/src/preread_store.rs` | 12 |

Plus, as incidental fixes found by the above: migration 0110 FK divergence,
`hit_rate.rs` migrating the shared postgres database, a `kind_index` collision,
a race in the test-support schema cache, and a `CostQuote::is_priced()` that
reported an unpriced local-model quote as priced.

## Remaining ranked gaps (from docs/plans/100-ideas-audit.md)

All six of the audit's "real gaps" are now closed. Gaps A, B, D, E, F, G are
complete, and **gap C is complete except for one route** — see below.

### Gap C step 5 — the author-facing route (the last piece)

§32.6: a pre-read report is shown to the *author*, never on the public work page, and
never as a composite number. `PreReadReport` has no `score()` and the `preread_reports`
table has no `score` column, so a composite would require adding one — a visible change
rather than an accidental one. A test asserts the column does not exist on either engine.

What is left:

1. `GET /works/:id/preread` in `crates/app/src/routes/` — author-only. A reader gets 404
   or 403, not an empty report, because a 404 that meant "no report" and a 404 that meant
   "not yours" are the same response and would leak the existence of the report.
2. The per-dimension breakdown, plus the `missing` dimensions with their reasons. A report
   where everything came back and one where half the provider's output was unparseable look
   identical without the `missing` list, and the difference is what tells an author whether
   to trust the score.
3. A `DELETE /works/:id/preread/:provider` withdrawal route, calling `forget_provider` —
   per provider, not per work, per §23.7.
4. Frontend: an author-only panel on the *editor* route. Never the public work page.

## Also outstanding

- 31 rows in the M45 tracker still marked `planned`.
- `docs/plans/100-ideas-audit.md` needs its gap F section marked CLOSED.
- The §0.3 "three TEXT/uuid columns" note should become six now: `rec_strategy.rs` and
  `discovery.rs` added two, `preread_reports.work_id` is the sixth.

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