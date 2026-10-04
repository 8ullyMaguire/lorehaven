# What is left — the 100 ideas, measured 2026-10-04

**This file supersedes the build list in `docs/plans/100-ideas-scope.md` §3.** That
section lists seven items as "building now", and four of the seven are wrong about
what exists. Read the probe table below before scheduling anything from that plan.

Every row was measured against the tree with `grep -rl` over
`frontend/src crates/app/src crates/db/src`, and every "store only" claim was then
confirmed by reading the route registration and the frontend. A count is evidence
about a term; the *wired/unwired* column is the finding.

## The one-line version

| state | count | what it means |
|---|---|---|
| shipped and wired | 3 | items 14, 27, 33 — `d81444c` |
| shipped, **no UI** | 1 | **item 11 (DNF)** — 3 routes, store, 8 acceptance tests, zero frontend references |
| built, **never rendered** | 3 | items 1, 4, 9 — data exists, nothing displays it |
| specified, not built | 1 | item 7 (Surprise Me) — `/surprise` is a *router id*, no route, no handler |
| built by an earlier pass | 2 | items 8, 22 — the probe was wrong, not the code |
| open, Tier 1 remainder | 5 | items 31, 41, 43, 75, 22-preset |
| deliberately refused | 3 | item 2 (archive scrape), the streak trio — argued in `100-ideas-scope.md` §1 |
| not yet assessed | ~72 | see "what I did not touch" at the foot |

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
| 1 | **11 — DNF UI** | largest gap between built and reachable; a reader who abandons a work has nowhere to record it |
| 2 | **1 — Continue Reading** | the single highest-impact retention item on the list; three queries over a table that exists |
| 3 | **4 — word count + minutes** | one calculation, one component; makes every card more clickable |
| 4 | **9 — reason tags** | the data is already stored and the label is free |
| 5 | **7 — Surprise Me** | genuinely new query, but small and it serves the diversity budget |
| 6 | 31, 41, 43, 75, 22-preset | the Tier 1 remainder; all render work over existing columns |

Items 1, 4 and 9 are one `reader_surface`-style module: no migrations, no new state,
one store file, one route set, three components. They should ship as **one** change,
not three, because the module boundary is the work.

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