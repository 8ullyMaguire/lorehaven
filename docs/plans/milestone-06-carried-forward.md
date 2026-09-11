# Milestone 6 — carried forward

Milestone 6 was stopped on 2026-09-11 with **`M6-02` unfinished**, and it is
therefore **not tagged**. Nothing else in the milestone is open: `M6-01` and
`M6-03`–`M6-12` are implemented and tested, and `M6-10` is deferred to Milestone 17
by the plan rather than by circumstance.

This file exists so the remaining work does not have to be reconstructed from a
session log. `docs/plans/milestone-06-imports.md` is the record of what M6 *is*;
this is the record of what it *owes*.

---

## 0. Progress since this file was written

| Adapter | State |
|---|---|
| `ffnet` — FanFiction.net and FictionPress | **written, tested, committed** |
| `wattpad` | **written, tested, committed** |
| `ficbook` | **written, tested, committed** |
| `scribblehub` | **written, tested, committed** |
| `xenforo` | **written, tested, committed** |

All five are done, each against pages recorded from the live site, each with an
offline fixture suite and a live test. `docs/plans/milestone-06-xenforo-recon.md`
keeps the reconnaissance as the design record, because two of its first readings
were wrong and the corrections are the useful part: the wall it recorded had been
caused by the probe itself (a browser `User-Agent` sent over non-browser TLS, which
is precisely what Cloudflare challenges), and the chapter total it said was missing
is stated in the list header and is now the adapter's completeness check.

Two decisions the adapters settled, which the fifth follows rather than
rediscovering:

* **A robots override exists.** `imports.honour_robots` is an operator setting
  (spec §11.5): `Disallow` is honoured by default, and an instance may override
  it, in which case every bypass is counted and the override is visible in the
  catalogue. Crawl-delay is never affected. This is why `wattpad` can read
  metadata under the default policy while its prose is refused — and why the
  refusal names the rule and the setting that would lift it.
* **A fetch layer that does not undo the source's compression corrupts bodies
  silently.** `safety::decode_response_body` handles it; `wattpad` is the source
  that exposed it, and any source behind an edge that gzips unconditionally would
  have been storing mojibake. Checked for a new source rather than assumed.

`M6-02` stays `partially-implemented`, and `v0.07-imports` **still cannot be
cut** — the milestone is not complete until XenForo lands.

---

## 1. Why it stopped here

The access half of importing from the bot-walled sources was the piece that looked
blocked, and it is done and live-verified: a browser fingerprint, a solver service
speaking the FlareSolverr v1 protocol, and an Internet Archive snapshot, declared
per source and switched on per instance (see §9 of the milestone plan). What is
left is **five parsers**, and each one is ordinary work of the kind four adapters
have already been through — record pages, write the parser against them, add a
fixture test.

They were not started rather than half-started, because a parser written from a
guess is worse than no parser: it matches nothing in one place and reports a work
with no chapters as a success.

## 2. What `M6-02` owes

Four adapters exist — Archive of Our Own (`ao3`, which also covers the other
Archive-software installs), Royal Road, Syosetu, and the eFiction family (nineteen
archives across eighteen hosts). **Five are missing.** Each entry below states what
is already known, so the next session starts from measurement rather than from a
fresh investigation.

### 2.1 `ffnet` — FanFiction.net and FictionPress (do this first)

**Fixtures: recorded.** Seven pages for the two hosts: `ffnet-work.html`,
`ffnet-work-complete.html` (122 chapters, `Status: Complete`), `ffnet-chapter-2.html`,
`ffnet-not-found.html`, `fictionpress-work.html` (17 chapters),
`fictionpress-work-second.html` (4 chapters), `fictionpress-not-found.html`. The
parser's whole contract is written up in `crates/scrapers/tests/fixtures/README.md`
§ffnet — read that before writing a line.

**Wall:** FanFiction.net is `Wall::Fingerprint`; FictionPress is `Wall::Solver`.
This is the pair that proves a wall belongs to a host rather than to the software
it runs, and each adapter must declare its own.

The findings that matter, all from the recordings:

| Fact | Consequence for the parser |
|---|---|
| A 2-chapter work carries **no `Status:` field**; the completed one carries `Status: Complete` | Absent means `Unknown`, never "ongoing". The ported code hardcoded ongoing. |
| The date is in `data-xutime` (epoch seconds); the visible text is `3/14/2015` on one host and `Jun 20, 2016` on the other | Read the attribute. A text parser needs two formats and returns `None` for the host it did not anticipate. |
| The metadata is one ` - `-delimited run with **three unlabeled fields** (language, genres, characters), and FictionPress omits characters | Cannot be read by position. Identify the language against the site's own list; classify the rest by shape. |
| The chapter list is `#chap_select`, **complete** (122 options for the 122-chapter work), rendered twice and identical both times | No pagination to walk, no separate table of contents. |
| The option's `value` is the ordinal and there is no per-chapter id | The ordinal **is** the source's own chapter key here. Include the slug in the URL, never in the key. |
| A missing work is **HTTP 200** with no `profile_top`, no `storytext`, no `chap_select` | Detect from the document, not the status. |
| That page also contains "Story is unavailable for reading" as boilerplate | Must **not** be read as the moderation hold the eFiction adapter distinguishes. |
| Prose is `div#storytext`, nested inside `div#storytextp` | A prefix match on `storytext` selects the wrapper as well as the body. |

**Unrecorded, and therefore unclaimed:** a genuinely *withheld* FanFiction.net
work (a takedown or moderation hold) has no fixture, so the adapter claims
`NotFound` for the missing-work page and must claim nothing about a hold. Extend
the recordings there before writing that branch.

### 2.2 `wattpad`

No wall: reachable with a plain client, and never recorded. This is the cheapest of
the five — record a work page, a chapter, and the site's own not-found page, then
write the parser. Nothing is known about its markup beyond that it is a different
shape from everything already built.

### 2.3 `ficbook`

No wall: reachable with a plain client, never recorded. Same shape of work as
wattpad. It is a Russian-language source, so the recording will exercise the
charset path and the sanitizer's `ruby` allowance.

### 2.4 `scribblehub`

**Wall: `Wall::Solver`.** A plain client and a browser fingerprint are both refused;
33,742 bytes of a real series page were read through Byparr on 2026-09-11, so the
access path is proven and only the parser is missing. Its `robots.txt` is itself
behind the challenge, so **its rules are unknown** — the adapter runs at the default
pace with that condition recorded rather than claiming a refusal or a permission
the file never stated.

### 2.5 The XenForo boards

**Wall: `Wall::Solver`.** A plain client and a fingerprint are both refused;
1,526,890 bytes of a real SpaceBattles thread were read through Byparr. The shape is
the different one of the five: a forum thread rather than a work with a table of
contents, so the adapter must decide what a chapter is — a threadmark, most likely
— and that decision belongs in the plan section for this adapter rather than in the
code alone.

The board set is not yet chosen. It is one adapter for the family, on the eFiction
pattern: pick the boards by robot's-rule survey first, as that family did.

## 3. Deliberately not built

**`M6-10` — approved preservation batches.** These stay in **Milestone 17**,
behind a documented permission basis, the operator role and a dry-run report
(spec §14.5). M6 shipped the machinery they will use and nothing that could be
mistaken for the authorisation: the destination field exists and accepts only the
reader's own library. `M6-10` is `unsupported` in `docs/requirements.csv` and
should stay there until M17 does that work.

**`M6-15` — instance work body retention.** The `cache` half is what M6 built and what
ships; the setting and the `aggregate` half do not, and are `M6-15` in
`docs/requirements.csv` (spec §11.15, added 2026-09-11). An operator who wants their
instance to be a catalogue of links — metadata, attribution and a canonical URL, no
stored text — currently has no way to say so, and nothing refuses a body on their
behalf. This is a real gap rather than a rounding error, but it is a bounded one: the
default is unchanged, so nothing shipped contradicts it, and the work is a setting, a
refusal at each path that could deliver a body, and the honest "not held here" states
on the reader and export paths. It stays `unsupported` until somebody builds it.

**`M6-13` — import result quality.** Added to the spec as §11.14 on 2026-09-11,
after M6 closed, by folding in the one idea worth taking from FicNexus's scraper
layer. §11.8 grades the *source* and this grades the *result*; M6 built the first
and not the second. What is missing is a classifier over fetched metadata —
accepted, rejected with a reason, or held for a person — a zero word count being
held rather than rejected, and the reason recorded on the job report. Nothing in
M6's code contradicts it, and the fixture seam (`preview_from_html`) is where its
tests belong, so it is a small piece of work rather than a reshaping. It stays
`unsupported` until somebody builds it, and it is written down here so that the
gap is a decision rather than an oversight.

## 4. Verification gaps to close, in the order they matter

1. **PostgreSQL has never been executed.** Every migration and every statement is
   written twice and only the SQLite half has ever run. This is the largest
   untested surface in the project and it is not specific to M6 — it has been the
   top open risk since Milestone 5.
2. **An archived read has never parsed a real snapshot.** The archive tier's URL
   construction, provenance marking, per-hop redirect check and error handling are
   verified against the real archive; whether a real Internet Archive snapshot of a
   real source page parses is not, because no page of the recorded works has one
   and the tier needs an adapter to reach.
3. **The solver tier is verified against one service.** Byparr 3.0.4, live, reading
   all five walled sources — but Byparr has no session API, so the session-reuse
   path is exercised only against a stub. FlareSolverr proper has one and is
   archived; if an operator runs it, the session path is what to watch.
4. **No browser test suite.** `svelte-check` now runs clean and Vitest covers the
   store, router, API client and four pages, but spec §23's automated browser
   journeys do not exist. Every frontend defect found in M3, M4 and M6 was found by
   hand.
5. **`M6-08` classifies why a source is unhealthy in the import's error mapping but
   does not write it back** onto the source row, so a source can report that it is
   degraded without reporting why.

## 5. The rules a successor must not break

These were each decided with a reason, and several were measured rather than
assumed. They are repeated here because they are the parts most likely to be
"simplified" by someone who was not there.

* **An adapter declares a wall; the instance decides the chain.** `SourceAdapter::wall`
  is a claim about **one host**, measured. `unblock()` is derived from it. Neither
  half may grant the other's: an adapter asking for a fingerprint gets one only if
  the build carries the feature, and an instance with a solver offers it only to a
  source that declared a wall.
* **A wall this instance cannot pass is refused before anything is queued**, naming
  the fix. Never discovered one page at a time.
* **Escalate only on a detected bot challenge** — never on a `404`, a `robots.txt`
  refusal, or a rejected credential. No different client changes those answers, and
  escalating turns one refusal into three requests to a site that already said no.
* **The transport is a seam inside `SafeFetcher`, not a second fetcher.** Pinning,
  hand-rolled redirects, the bounded read, the robots gate and the pacing live in
  one place; a second fetcher would drift from the first within a milestone.
* **A fingerprint is only coherent if the headers agree with it** — do not override
  the `User-Agent` on an impersonating client, and do not set `no_gzip()`. Both were
  measured. The consequence is that a fingerprinted request does not identify itself
  as Lorehaven, which is the real cost of the technique.
* **Do not match on `challenge-platform`.** It is the URL of Cloudflare's script and
  appears on ordinary served pages. Gate every body marker on a refusing status.
* **The source's own `crawl-delay` is the floor for pacing, not the crate's default**,
  and a solver solve costs about twelve seconds per page.
* **Pacing and path rules come from the source.** A `robots.txt` that cannot be read
  means the rules are unknown: proceed at the default pace and record the condition.
* **Fixtures are the contract, recorded from the live site and committed verbatim.**
  Never hand-write one to match a parser.
* **Record through the unblock path when the source refuses a plain request**, and
  note which mechanism each host needed.
* **Nothing claims a page it did not get from the source.** `Fetched::provenance`
  carries whether a read was archived, and an adapter storing a canonical URL stores
  the work's address rather than the archive's.
* **The live suite runs with `--test-threads=1`.** Parallel runs inflate it by an
  order of magnitude and the victim varies.

## 6. Definition of done for M6

1. The five adapters in §2, each with fixtures recorded from the live site, a
   fixture test, a `wall()` declaration that matches what was measured, and an
   entry in `tests/fixtures/README.md`.
2. `M6-02` leaves `partially-implemented` in `docs/requirements.csv` for
   `implemented-locally-tested`, with the four existing adapters' evidence intact.
3. The five items in §4 that belong to M6 addressed or explicitly re-deferred —
   item 1 in particular, which wants a CI job with a `postgres` service.
4. `docs/plans/milestone-06-imports.md` §3 rewritten to describe what was built
   rather than what was not.
5. Then, and only then, `git tag v0.07-imports`.

---

*Written 2026-09-11. Repository state at the time of writing: `master`, clean, the
last M6 commit being the ffnet fixtures. Milestone 7 (`v0.08-exports`) is in
progress; this file is not superseded by that work and should be worked through on
its own terms.*
