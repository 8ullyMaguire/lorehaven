# What is left — the 100 ideas, re-probed 2026-10-05

**THE CANONICAL INVENTORY IS `docs/requirements.csv`** — 703 rows, one per requirement, with
a status and an evidence field. It is the thing ADR 0023 names as the feature inventory and
this file is commentary on it. Anything below that disagrees with the CSV is wrong.

**This file supersedes `docs/plans/100-ideas-scope.md` §3** and **supersedes
`docs/plans/REMAINING-2026-10-03.md`** and `docs/plans/WHAT-IS-LEFT.md`, which already
superseded each other in a chain. Three files were claiming to be the live plan; that was
itself the defect, and it is why this one existed in a stale state for three commits after
its last edit.

**UI and routing defects found by driving a live instance (2026-10-05) are not in this
file** — they live in `docs/plans/REMAINING-2026-10-05.md`, which owns that list and
defers every feature question back to this file and to `docs/requirements.csv`. The two
defects it named in this file (a stale row and a numbering gap in §2) are now fixed, and
§2 below says so where the fix is visible.

## The probe rule, which is the whole lesson of this file

**A status document rots faster than the code it describes, because nobody re-derives it.**
Five times on this project the tracker said something was unbuilt while it was shipped and
mounted: Concierge (11 component tests, no route), item 9 (the backend existed, the frontend
called nothing), item 11 (three routes, zero frontend references), and items 1 and 4. Two of
those were *this file's* fault, and both times the reason was the same:

> `grep -rl <term> frontend/src` answers "does this string exist", which is not the same
> question as "is this mounted".

The probe that answers the second question looks for the **consumer** of the data, not the
producer: grep the component inside a route file, not the type inside `api.ts`. And a row whose
`evidence` cites plan documents is the tell — a plan file cannot prove a thing renders. Every
evidence field I have written since cites **file paths in `frontend/src`**.

Item 1 is the case that makes the rule precise. It has **no page route and never should** —
it is a banner on the homepage. A route-shaped check reports it as broken; it is correct. So
"no route" is only a defect for a surface meant to be a destination.

Every row was measured against the tree with `grep -rl` over
`frontend/src crates/app/src crates/db/src`, and every "store only" claim was then
confirmed by reading the route registration and the frontend. A count is evidence
about a term; the *wired/unwired* column is the finding.

## The one-line version

| state | count | what it means |
|---|---|---|
| **shipped AND wired** | **8** | items 1, 4, 7, 9, 11, 14, 27, 33 |
| built by an earlier pass | 2 | items 8, 22 — the probe was wrong, not the code |
| **shipped from `requirements.csv`** | **2** | M45-51 subject access + erasure (`9f9995f`), M45-53 import visibility (`da3dfe2`, `ded795a`) |
| **open, Tier 1 remainder** | **5** | items 31, 41, 43, 75, 22-preset |
| next in queue | — | item 31, or M45-39 (the two external-dependency rows) |
| deliberately refused | 3 | item 2 (archive scrape), the streak trio — argued in `100-ideas-scope.md` §1 |
| not yet assessed | ~72 | see "what I did not touch" at the foot |

## The 26 `planned` rows in `requirements.csv`, and how they were sorted

The CSV is the canonical inventory (703 rows: 120 `implemented-verified-e2e`, 312
`implemented-fully-tested`, 239 `implemented-locally-tested`, 4 `unsupported`, 1
`evaluated-and-rejected`, **26 `planned`**). Grouping them by what they need:

| group | count | ids |
|---|---|---|
| new query/store | 22 | M45-25, 26, 27, 29, 30, 32, 33, 34, 36, 37, 38, 40, 41, 42, 43, 44, 45, 46, 47, 48, 50, 52, 54, 55 |
| migrations | 1 | M45-28 |
| external dependencies | 2 | M45-39 (ActivityPub), M45-53 (scraping providers) |
| **done since** | **−2** | M45-51 (`9f9995f`), M45-53 (`da3dfe2`, `ded795a`) |

**No `planned` row has render work left in it.** Item 9 was the last one where data existed
and nothing displayed it. Every remaining row is a feature, which is why this queue is going
slowly and honestly rather than quickly.

**None of the 27 has a section in `docs/spec.md`** — checked, 0 hits for each id. So every one
of them needs spec + plan written *before* code, per the standing rule. M45-51 was picked
first because its correctness stake per unit of work was the highest: both halves are the class
where a silent bug is a real harm, and the measurement showed the erasure half was already 99%
present as schema.

**M45-53 is closed** (`da3dfe2`, `ded795a`), and its closing is the pattern to follow for the
rest: **measure before building.** It was flagged "biggest reputational and copyright risk" and
turned out to be already satisfied — `body_for` filters `account_id` inside the query, its
signature leaves a route nowhere else to put an account, and the injection-verified test predated
the requirement. `library_items` has no visibility column at all, so there was no default to get
wrong. What shipped was the *gate* (`scripts/check-library-visibility.py`) so the property stops
depending on every route happening to take the session account, plus runtime proof on the
metadata routes.

**M45-39 is next**, and it is the first of the two rows that need an external protocol
(ActivityPub for account `Move`, a dead-man's-switch export). Neither M45-28 (migrations) nor
the 22 new-query rows have a spec section either, so each still needs spec + plan first.

**The "no UI" and "never rendered" rows are gone because those items are now rendered.**
Every row above was re-probed against the tree on 2026-10-05 rather than carried forward —
this table had been stale for three commits and was claiming items 1, 4, 9 and 11 were
unfinished when all four were shipped and mounted. That is the third time on this project that
a status document outlived the thing it described:

| item | where it actually renders | commit |
|---|---|---|
| 1 Continue Reading | `ContinueReadingBanner` mounted in `Home.svelte`; no page route needed, it is an enhancement to the homepage rather than a destination of its own | `7537035`, `f124a67` |
| 4 reading time | `word_count` on every card | `2868b25`, `f124a67` |
| 9 Why this? | `WhyRecommended` mounted in `Discover.svelte`, reading `slot_id` off every feed item | `1f5ce1a`, `5759616` |
| 11 DNF | `DnfPanel` mounted; API routes under `routes/dnf.rs` | `2901bc0` |
| 7 Surprise Me | `SurpriseMe.svelte` at `/surprise-me`, in the Read menu | `5759616` |

**The lesson, because it has now cost three commits.** "Implemented" and "reachable" came
apart on items 7, 9 and 11 — Concierge was the fourth. A probe with `grep -rl` answers
"does the string exist" and cannot answer "is it mounted". The check that answers the second
question is to look for the *consumer* of the data, not the producer: `grep` the component
inside a route file, not the type inside `api.ts`. Item 1 is the case that makes this precise —
it has no page route and never should, because it is a banner on the homepage, so "no route"
is the correct answer and a route-shaped check would report it as broken.

`5759616` also landed item 9's missing `api.ts` half: `1f5ce1a` committed
`WhyRecommended.svelte` but not the `fetchSlotExplanation` it imports, so the frontend build
was failing at HEAD with *"fetchSlotExplanation is not exported"*. A commit that leaves the
tree unbuildable is worse than one that leaves a feature half-done, because the next thing that
touches the frontend trips over it.

## 1. The four corrections to `100-ideas-scope.md` §3

The plan is a good document and its §1 refusal of the AO3 scrape is correct and
should stand. §3's build table is where it went wrong, and the errors all point the
same way: **it read the spec's promises instead of the tree.**

### Item 11 (DNF) is not "the only new state" — it shipped in M45-21

The plan's Step 1 creates `reader_work_status`. **That table would have been a
second implementation of a feature that exists**, with a weaker design. What is
already there:

- `did_not_finish` (`migrations/{sqlite,postgres}/0073_dnf_reasons.sql`) with **six
  structured reasons** (`not_my_taste`, `triggering`, `slow_pacing`,
  `abandoned_by_author`, `dropped_other`, `other`), a `note`, `is_public`, and a
  partial unique index on `(pseud_id, work_id) WHERE deleted_at IS NULL`.
- `works.allow_dnf_feedback` — the author's consent switch, which the plan's table
  has no equivalent of and which §57.4's privacy argument actually needs.
- `crates/db/src/dnf.rs`: `upsert_dnf`, `delete_dnf`, `read_dnf`,
  `list_dnf_for_work`, `aggregate_dnf_counts`, `work_allows_dnf_feedback`,
  `list_dnf_by_pseud`.
- Three registered routes: `PUT /works/{id}/dnf`, `GET /works/{id}/dnf/reasons`,
  `GET /me/dnf`.
- `crates/db/tests/m45_21_dnf.rs`.

The plan's own design note argues `reader_work_status` is better *"because a private
reading note per chapter (item 20) wants the same privacy shape"*. True, and the
answer is to add that column to `did_not_finish` when item 20 is scheduled — not to
stand up a parallel table for one status while a richer one sits beside it. A
migration number is not an entitlement.

**The real gap is not the table. It is that `grep -rl dnf frontend/src` returns
nothing.** Seven tests, three routes, no way for a reader to reach any of it.

### Item 8 (completion badges) is built, per the same misreading as last pass

`COMPLETION_LABEL` is in `frontend/src/lib/labels.ts` and rendered by
`WorkCard.svelte` through `MetadataChip`. Already rejected as a Concord complaint on
2026-10-04 for the same reason. The plan re-lists it as unbuilt.

### Items 1, 4 and 9 are render work, not new state

| item | data that exists | what is missing |
|---|---|---|
| 1 Continue Reading | `reading_progress.position_permille` (10 files) | nothing fetches and renders it |
| 4 word count + minutes | `chapter_revisions.word_count` (40 files), 3 files already touch `reading_time` | no card shows it |
| 9 recommendation reason | `reason` stored (138 files), `reason_tag` in 5 | never rendered |

The plan calls item 9 "§16.1 already stores them; nothing renders them", which is
**right**. It then filed it alongside two items it wrongly believed were unbuilt.

### Item 7 (Surprise Me) has a route id and nothing else

`frontend/src/lib/router.test.ts:18` mentions `/surprise` as a `RouteId`. There is no
`/surprise` route on the server, no handler, and no store query. A router id is not a
feature; this is the "implemented-but-unreachable" failure mode the pass-10 note
already found seven instances of.

## 2. Next, in this order

| order | item | why this order |
|---|---|---|
| 1 | ~~**11 — DNF UI**~~ | DONE — `2901bc0` |
| 2 | ~~**1 — Continue Reading**~~ | DONE — store, route, banner, 8 E2E journeys |
| 3 | ~~**4 — word count + minutes**~~ | DONE — store `2868b25`, rendering `f124a67` |
| 4 | ~~**9 — reason tags**~~ | DONE — the backend already existed; this was pure UI |
| 5 | ~~**7 — Surprise Me**~~ | DONE — `5759616`; the detail is in §2a below |
| 6 | 31, 41, 43, 75, 22-preset | the Tier 1 remainder; all render work over existing columns |

**Two fixes to this table, both from `REMAINING-2026-10-05.md` §7.**

Row "7 — Surprise Me" was still listed as open while §1's own table said "shipped
AND wired" and §2a carried three commits of notes about it. The tree agrees with §1:
`SurpriseMe.svelte:50` calls `fetchSurpriseMe()`, `/surprise-me` is in `router.ts`, and
`frontend/e2e/surprise-me.spec.ts` exists. Struck, because a plan that lists a shipped
item as next is the exact failure this file's own §1 is about.

And the numbering jumped 3 → 5, because row 4 was removed when it closed and nothing
renumbered behind it. Harmless, but a reader assumes a gap is a lost row — which is
precisely the wrong assumption to plant in a document whose subject is documents that
lie. Renumbered. **This is the only row that is actually open.**

Items 1, 4 and 9 are one `reader_surface`-style module: no migrations, no new state,
one store file, one route set, three components. They should ship as **one** change,
not three, because the module boundary is the work.

## 2a. Progress log

### DONE — item 11, DNF (commit `2901bc0`)

Store, route, component, unit tests, E2E journey. A reader can mark a work DNF with one
of six reasons and a private note. All 8 E2E journeys green after the fix.

### DONE — item 1, Continue Reading (store + route in `7537035`, banner after it)

`crates/db/src/continue_reading.rs` (12 tests, both engines), the route
`GET /api/v1/continue-reading`, `ContinueReadingBanner.svelte` (8 tests) mounted on the
homepage, 2 wiring tests in `Home.test.ts`, and `e2e/continue-reading.spec.ts`.

The one design point worth writing down, because the first version got it wrong and a
test caught it rather than a reader: **progress is per-DEVICE, and there are two of this
reader's rows.** `0004_reading` declares two partial unique indexes — one for
`device_id IS NOT NULL`, one for `IS NULL` — so a reader with a laptop and a phone has
two rows per work. The first query aggregated with `MAX(position_permille)`, which is the
**furthest** position, not the **last** one, and aggregating `MAX(updated_at)` and
`MAX(position_permille)` independently let them disagree about which row they came from:
the banner would say "90% through" while quoting chapter 1 from a different device. The
fix is no aggregate at all — group by every selected column and order by `updated_at`, so
`LIMIT 1` returns one whole row.

The route is `RequireSession`, not `MaybeSession`: "where did **you** stop" has no
anonymous answer, and an empty banner to a stranger reads as "you have read nothing".

Three more defects the mutation gate found, all of the same family — a green build is
not a working feature:

- **A comment line starting with `#` inside a raw SQL string.** `#` is not a SQL comment
  introducer. PostgreSQL lexed it as the prefix of the `#>` operator and raised
  `syntax error at or near "::"` — an error about a type cast, raised from a comment.
  SQLite ignored it, so the entire SQLite suite was green throughout. Swept the repo: 0
  more.
- **`is_public = 1` on PostgreSQL**, where the column is BOOLEAN: `operator does not
  exist: boolean = integer`. Correct on SQLite, invisible by default.
- **My own mutation runner produced 14 false survivors** because it ran no tests at all
  and scored empty output as a pass. It now refuses to score a run that tested nothing,
  and it asserts that a compile failure is distinguishable from a pass before it is
  trusted with a result.

And one in the wiring, which is the one that matters:

- **`Home.svelte` wrapped the banner in `{#if session.isSignedIn}` while the comment
  directly above it said the guard lived in the component.** That contradiction made the
  signed-out case untestable: with the wrapper the node is not in the DOM when signed out,
  so "no request was made" had nothing to assert about, and deleting the whole mount point
  left the test green. Passing `signedIn` as a prop keeps the node mounted and hidden,
  which is a state a test can pin. All four wiring mutations now go red.

### DONE — item 4, word count + reading time (store `2868b25`, rendering `f124a67`)

`SUM(chapter_revisions.word_count)` through `chapters.current_revision_id` — the same join
`events::work_word_count` already uses, so this is a second READER of a correct aggregate,
not a second definition of it. All six query arms (three queries, two engines).
`readingLength()` in one shared module, imported by all three rails.

The audit did not mention the real shape of this item: **`WorkCard` already had a
`wordCount?: number` prop that no caller ever passed.** The component was written, the
column was in the database, and nothing connected them. Unreachable, not missing — which is
why "just render it" was half the work and why `wordCount?` being optional had been hiding
in plain sight.

`word_count` is `i64`, NOT `Option<i64>`. The aggregate is COALESCEd to 0 in SQL, so "this
work has no chapters" is 0 words, a true answer. An Option would put `undefined` in front
of the client for every work without prose, and a card that hides a missing count would
then also hide a zero.

**The two-engine run paid for itself twice.** Both mutations PASS on SQLite and FAIL only
on PostgreSQL:

- the `::bigint` cast moved outside the coalesce. `SUM` over INTEGER is `bigint` on SQLite
  and `numeric` on PostgreSQL, and `COALESCE(x, 0)` over a numeric is still numeric, so the
  cast belongs on the aggregate;
- the revisions reached by `cr.chapter_id = c.id` instead of
  `cr.id = c.current_revision_id` — summing every draft the author ever saved. A heavily
  revised chapter reports several times its real length and it looks plausible, so nothing
  downstream would notice.

All three PostgreSQL arms verified separately, since a mutant in one arm is invisible to a
test that only exercises another. 22 store tests green on both engines.

**Two of my own test fixtures were wrong before the code was.** 300 words / 250 is 1.2 and
`ceil(1.2)` is 2, so a 300-word work correctly says "~2 min" — I had asserted "about a
minute" against it. And `12,000` is not `< 10,000`, so the formatter drops the decimal and
"12k words" is right, not "12.0k". Both were failures in the test, not the formatter, and
each is now the thing the test documents.

**`#[serde(skip)]` on the store field is the mutation that proves the journey earns its
place.** Computed, tested on both engines, never serialised: all 22 store tests and all 520
frontend tests stay GREEN, and the number is simply absent from every page. One E2E journey
goes red. Nothing else does.

And a false lead worth recording: my first mutation renamed the SQL alias to
`word_count_hidden` and the journey still passed. That was CORRECT — serde reads the struct
field name, not the column name — so an alias rename is not a mutation of the wire format
at all, and I nearly recorded "the journey does not verify the field" from it.

### DONE — item 9, recommendation reasons ("Why this?")

**Item 9 was not a feature. It was a missing `if`.** Before writing anything I probed the
running server by hand:

```
GET /api/v1/discovery           -> {"items":[{"title":"Explaining The Feed",
                                    "slot_id":"f1f7a2c1-…"}], …}
GET /api/v1/discovery/slots/f1f7a2c1-…/explanation
                                -> {"reasons":["taste_tags","popular"], …}
```

So the route existed, the store wrote a `slot_id` onto every item it served, the
explanation answered with a real vocabulary — and no element in the frontend called
either. Every layer was correct and the reader could see none of it. The tracker had it
as "the data is already stored and the label is free", which was right and understated the
second half: the label was not free either, it had to be a sentence.

Decisions:

- **`slot_id` is OPTIONAL on `DiscoveryItem`, unlike `word_count` from item 4.** The
  server warns-and-continues when `record_response` fails, so a slot id is genuinely
  absent on a healthy server. An `Option` here is a fact about the server, not laziness.
- **A disclosure, not a tooltip.** A tooltip is invisible until hovered, so "why am I
  seeing this" stays a question only the reader who thought to ask it will. The reader who
  most wants to know WHY is the one being recommended most.
- **Mounted OUTSIDE the `<a>`.** The feed row is one link, so a button inside it is a
  button inside an anchor: invalid HTML, and in practice the click navigates and the
  disclosure never opens.
- **Fetches on click only.** N items × N routes per paint for information most readers
  never ask for, and it would put a slots-table read on the feed's critical path.
- **Four states render NOTHING rather than a disabled control**: no slot id, a 404 (§3.3
  makes "no such slot" and "not yours" indistinguishable, so the client cannot tell them
  apart either), a signed-out reader, and a request in flight.
- **404 and 500 are different.** 404 retires the trigger — there is no explanation. A
  network error does NOT, and offers "try again", because the slot probably exists. All 8
  mutations confirm the two are kept apart.

**One false lead in the verification itself, and it is the oldest trap on this project.**
My first E2E mutation pass reported BOTH wiring mutations as survivors. They were not: I
edited `Discover.svelte`, ran Playwright, and it tested the OLD assets — the release binary
embeds `frontend/dist` at COMPILE time, so a source edit changes nothing that is served.
`src` mtime 21:54, `dist` 21:47, binary 21:51: both stale. Every mutation on this project
costs a `vite build` AND a `cargo build --release`, which is why it runs as a shell script
rather than an inline loop (which hit the 300s cell limit mid-mutation and left the mount
tag stripped from the file, which then read as a component defect on the next look).

And then the sweep's BASELINE came back red with the source restored — which is a harness
fault, not a finding, and had to stop everything. The cause was my own manual Playwright run
holding port 8173 alongside the script's. The runner now aborts if anything holds the port,
and reads its verdict from **stdout only**: Node writes
`ExperimentalWarning: localStorage is not available` to stderr on every vitest run, and a
scorer that greps `Error:` across both streams called all eight mutations BUILD-ERROR while
the baseline read PASS. That is the same "scored nothing as a pass" failure this project has
already paid for twice, and it arrived in a third disguise.

Two test failures that were my fault, not the code's, and both are worth the space:

- **`vi.doMock` in `beforeEach` cannot mock a module the component already imported.** It
  applies only to imports made AFTER it, so every click reached the real client and four
  tests failed with "cannot find why-reasons" — which read like component bugs. `vi.mock`
  is hoisted and is the right tool.
- **Mocking the wrong layer.** I threw an `ApiError(404)` from the mocked client to test
  the 404 branch. But the client CATCHES 404s itself and returns `null` — so I had been
  testing the client's own mapping, and the component's catch-all correctly read my throw
  as a network failure. Mock the layer the component actually consumes. That client's
  mapping is real behaviour, so it now has its own file
  (`SlotExplanationClient.test.ts`): the component test mocks it wholesale, so nothing
  else in the suite runs its body, and if it stopped swallowing 404s every component test
  would stay green while the feature rotted.

### Item 7 (Surprise Me) — store, route and UI, with two bugs only a browser found

**THE UI EXISTS NOW, and the two bugs below were found by the Playwright journey after 28 unit
tests were already green.** Both are the same shape: a claim about the reader stated confidently
when the data did not support it.

**Bug A — a brand-new reader was told their profile covered the whole catalogue.**
`surprise_me_work` returned `Option<SurpriseCandidate>`, and an `Option` cannot carry
`profile_empty` when there is no candidate. So the route filled in `false`, and a reader with
no profile on an instance with nothing published got *"Everything here shares a tag with your
profile. Surprise Me deliberately steps outside it."* False twice over: they have no profile,
and there is nothing here. The flag is a property of the **reader**, so it has to be
answerable with no works in the catalogue — the return type is now `SurprisePick { candidate,
profile_empty }`, computed by a separate `profile_is_empty` query when there is no candidate,
and reusing the candidate's own value when there is one (a second query could only disagree
with it). Two store tests cover it, and the mutation (hardcoding `false` again) turns exactly
one red.

**Bug B — a failed request printed a confident empty state under the error banner.** The
component had `{#if error}` as a *sibling* of `{#if loading}…{:else if work}…{:else}`, so on a
404 the chain fell through and rendered *"everything here shares a tag with your profile"*
underneath the error summary. `error` is now the first and exclusive branch.

**No Rust test caught Bug A, and that is the point.** Every store test either had a candidate
to read the flag off or never examined the empty case — the shape the `Option` erased. The
journey that caught it asserts the *sentence*, not the presence of text, because a page with
one generic empty message would have passed anything weaker.

**Three ways my own journeys lied, all from the same cause — the scratch database is SHARED
across journeys, not reset per test:**

| what I asserted | why it was wrong |
|---|---|
| "a reader with no profile sees the catalogue-empty message" | an earlier journey in the file had published work, so the catalogue was not empty |
| "the served pick is the work this test published" | the query picks from every published work; three existed by then |
| `getByRole('link', { name: 'Surprise Me' })` | matched a work titled *"Surprise Me Eligible Work"* from another journey — strict mode caught it as two elements |

A journey that depends on its position in the file is worse than no journey. What is left is
what is actually the frontend's claim: the two empty states are worded differently, the served
card links to the work it names, the nav entry exists, and a failure is visible rather than
converted into a quiet empty state.

**A PostgreSQL run aborted on 6 failures that were not failures.** All six were in
`milestone_39`, all six had the same cause — `could not resize shared memory segment
"/PostgreSQL.…": No space left on device` while applying migration 0026 — and all six passed
on a re-run. `/dev/shm` had 15G free by the time I looked, because the exhaustion was
transient: two threads replaying the full migration catalogue at once allocate parallel-query
shared memory faster than the kernel releases it.

So the suite **needs `--test-threads=1` on PostgreSQL, not 2**, and needs `--no-fail-fast` or
one shm hiccup hides the other 100 suites behind an abort. Both were already known for SQLite;
the same arithmetic applies to PG and the `--test-threads=2` advice in this file is wrong for
a full-workspace run.

### Earlier — item 7, Surprise Me

`surprise_me_work` in `crates/db/src/discovery.rs`, the route
`GET /api/v1/discovery/surprise-me`, and `crates/app/tests/surprise_me.rs` — 13 tests
green on SQLite **and** PostgreSQL, four consecutive runs. Not yet committed with UI.

**The inversion is an EXCLUSION, not a negative score.** §16.10 says "the system inverts
usual weighting", which could be read as scoring candidates `-affinity` — and that would
rank the *least* interesting work in the catalogue first, so a reader who clicks learns
nothing and concludes the button is broken. So: exclude what the profile already loves,
then order the rest stably. The profile narrows the set; it never ranks within it.

And it is emphatically **not Blind Date with a different seed**. Blind Date leaves the
profile by *ignoring* it (no taste clause at all); Surprise Me has to go specifically away
from it, which is the whole feature and is why it needs the tests Blind Date does not.

**The reader's own works are NOT excluded** — the opposite of Blind Date, which excludes
authors the reader has finished. Surprise Me is about taste distance, and on a small
instance excluding self-authored work returns nothing at all.

A real bug the tests caught, and the kind that is invisible on one engine:

- **`profile_empty` counted ROWS, not SIGNALS.** A reader whose profile row exists with
  `signals = []` has a row, so the flag said "you have a profile" while the exclusion
  correctly treated it as empty. The flag and the exclusion disagreed about the same
  reader, and the route would have told someone with no taste profile that they had one.
  Now it counts array elements.

  Three separate failures kept this alive, and the third is the one worth keeping:

  1. **`cargo fmt` silently reverted half the fix.** I corrected the SQLite arm, the tests
     went green on PostgreSQL, and a later formatting pass reflowed the string literal so my
     next replacement no longer matched. The arm was left counting rows again with no error
     anywhere. When a mutation "has no effect", suspect the artefact pipeline and the
     formatter before suspecting the test.
  2. **The static checker I wrote to catch this class could not catch it.** I built
     `scripts/check-query-arm-symmetry.py` to compare the two engine arms of every query,
     iterated on it eight times, and it exited 0 with the row-count bug present in every
     state. Its arm-splitting kept landing on the wrong `Backend::` mention — first on a
     dialect fragment (`=> "::text"`) rather than the query, then on the execute dispatch,
     so a 12-line SQLite arm and a 13-line PostgreSQL arm never lined up. **I deleted it.**
     A linter that cannot fail on the defect it was written for is worse than no linter,
     because the next person cites it as evidence. What actually caught it was running both
     engines, and what will catch it next time is `crates/app/tests/profile_empty_arms_agree.rs`.
  3. **My first version of that test duplicated the SQL it was testing.** It re-implemented
     both arms inline and asserted its own copies agreed — so it passed 4/4 against the
     broken code. Reverting the arm to counting rows changed nothing it could see. It now
     calls `surprise_me_work` and reads `profile_empty` off the returned candidate, and
     reverting the arm turns **2 of 4 red on SQLite**. Verified both directions, then
     restored.

  **Scope of that test, stated because it looks larger than it is:** the mutation fails 2 of
  4 on SQLite and passes 4 of 4 on PostgreSQL. That asymmetry is correct — the PostgreSQL arm
  was always right, so the bug is a SQLite-only defect — but it means a green PostgreSQL run
  proves *nothing* about it. The header says so, so nobody later reads "4 passed" as
  coverage it does not provide.

**Four dialect mistakes, all of them mine, and three were opposite guesses about the same
table.** Read off the migrations rather than assumed:

| column | SQLite | PostgreSQL | what I first wrote |
|---|---|---|---|
| `works.id` | TEXT | UUID | cast to text |
| `bookmarks.subject_id` | TEXT | UUID | cast to **text** → `uuid = text` |
| `bookmarks.account_id` | TEXT | UUID | cast to uuid (right) |
| `work_tags.work_id` | TEXT | UUID | cast to text → `uuid = text` |
| `taxonomy_nodes.id` | TEXT | TEXT | text (right) |
| `taste_profiles.account` | TEXT | TEXT | cast to **uuid** → `text = uuid` |

So the account id is a uuid in one subquery and text in another, and both are correct — the
rule is per COLUMN, not per value. `taste_profiles.signals` is a JSON **array**
(`json_group_array`/`json_agg` in `recompute_taste_profile`), not an object; I assumed a map
and wrote `jsonb_each_keys`, which PostgreSQL rejected as `cannot call jsonb_each on a
non-object` before the shape question even arose.

And the fixture bug this project has now hit three times: `bookmarks.is_public` is INTEGER
on SQLite and BOOLEAN on PostgreSQL. The literal `1` gives 42804, the bound string `'true'`
gives the same code with "text", and SQLite accepts both. The suite's `exec_with` gained a
`#b` marker, and the PostgreSQL bind now follows the MARKER rather than the argument value —
keying on the value would bind a boolean to `work_tags.weight`, whose argument is `"0"`.

**A test I wrote was flaky by construction and I only found it by running three times.**
`different_readers_see_different_works` asserted `seen.len() == 2` over 20 works. Measured:
two independent FNV-1a picks over 20 candidates agree **5.15%** of the time — one failure
per twenty runs. The store promises the SEED differs per reader, not that the ANSWER does;
a collision is correct behaviour. It now runs twelve readers and asserts they do not ALL
get the same work, which a constant seed cannot survive by luck (20·(1/20)¹²).

## 3. Gates every one of them must clear

The existing pattern is `crates/db/tests/reader_surface_t1.rs` plus a Playwright
journey, and it is the bar:

- **Both engines.** 18 store tests green on SQLite and PostgreSQL. PG needs
  `--test-threads=2`: 18 schemas each replaying the migration catalogue exhausts
  `max_locks_per_transaction`, which is a lock-table limit and not a code defect.
- **The privacy mutation, run twice.** Every query reading a reader-private row gets
  `is_public` removed and must turn **exactly one** test red at store level and
  **exactly one** Playwright journey red after `cargo build --release --bin lorehaven`.
  Restored byte-identical both times (`diff`).
- **Windows belong at the store layer.** A route computes its window from
  `Utc::now()`, so a literal-date fixture sits outside it and the query correctly
  returns nothing. Three route tests failed on exactly this.
- **`{"works": []}`, never null.** A client must never branch on null-ness.
- **An empty rail renders nothing.** Not an empty rail under a heading: an empty
  "Similar works" says the site thinks nothing relates, which is a different and wrong
  claim from "no suggestions yet".
- **Journies through `scripts/fe.sh e2e`, never `playwright test` directly.** The
  release binary embeds `frontend/dist` at compile time; `fe.sh` catches a stale
  binary **by name**, and bypassing it hides mutations.
- **A 0-grep is not an absence.** Two of the five items closed last pass were
  implemented all along. Probe the component, the route and the column before opening
  a ticket — and when a probe finds the feature already there, **reject** the
  complaint rather than validating it.

## 4. What I did not touch, and why

**Item 2 (the AO3 archive scrape) stays refused.** `100-ideas-scope.md` §1 argues it
better than I would: a salted hash of a username is re-identifiable, discarding the
salt deletes the key to your own copy rather than the copy, and a 23-day crawl of a
volunteer nonprofit is a cost somebody else pays. §11.5 already forbids it in the
spec. The local co-bookmark alternative (item 33's scorer at batch scale) and
opt-in import of a reader's *own* bookmarks (item 19) are the versions worth
building, and both are ordinary work.

**P2P stays LAN/direct-share.** A content distribution network needs a threat model
before it needs code.

**~72 items unassessed.** Several are good (bookmark collections, chapter-level
bookmarks, a reading heatmap, an abandoned shelf, provenance badges) and several are
multi-month (generalized media, the seeding protocol, Calibre sync). They go through
the same measure-then-schedule loop, in impact order, after the six above.

## Provenance

- `d81444c` — items 14, 27, 33 shipped; 18 store + 8 route + 3 inventory tests on both
  engines; 6 Playwright journeys; the `is_public` mutation red at both levels.
- `d04c225` — Tier 1 remainder named.
- Concord project 41: 28 validated, 2 rejected, 19 open. The 2 rejected are the
  false-positive probes, and a rejected complaint is a *finding*, not a closure.