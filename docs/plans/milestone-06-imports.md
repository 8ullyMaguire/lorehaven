# Milestone 6 — what was built, and what was not

`docs/plans/junior-implementation-plan.md` says what M6 was *meant* to be. This
file records what it *is*, where that differs, and why — the same shape as
`docs/plans/milestone-05-jobs.md`, and written at the same time as the work
rather than afterwards.

The milestone is **partly built**. Its status is not "done with a caveat": the
import machinery is finished and tested, and the pages a reader would use are
absent, which means the plan's own journey cannot be walked.

---

## 1. What exists

| Piece | Where | Tests |
|---|---|---|
| The adapter crate, its trait and the registry | `crates/scrapers/src/{lib,registry,sites}.rs` | 146 in-crate |
| The safe fetcher and URL guard | `crates/scrapers/src/safety.rs` | in-crate |
| Charset decoding for fetched pages | `crates/scrapers/src/safety.rs` (`decode_body`) | 9 in-crate |
| `robots.txt`: the source's own pace and path rules | `crates/scrapers/src/robots.rs` | 21 |
| Chapter sanitation | `crates/scrapers/src/sanitize.rs` | in-crate |
| The Archive-software adapter | `crates/scrapers/src/sites/ao3.rs` | 7 unit + 9 fixture tests |
| The Royal Road adapter | `crates/scrapers/src/sites/royalroad.rs` | 13 unit + 21 fixture tests |
| The Syosetu adapter | `crates/scrapers/src/sites/syosetu.rs` | 16 unit + 31 fixture tests |
| The eFiction family adapter | `crates/scrapers/src/sites/efiction.rs` | 18 unit + 37 fixture tests |
| Recorded fixtures | `crates/scrapers/tests/fixtures/{ao3,royalroad,syosetu,efiction}/` | provenance in `fixtures/README.md` |
| Live verification, by hand | `crates/scrapers/tests/live_verification.rs` | 6 `#[ignore]`d, network |
| The planning rules | `crates/domain/src/imports.rs` | 24 |
| Migration 0006, both dialects | `migrations/{sqlite,postgres}/0006_imports.sql` | applied by every acceptance test |
| Migration 0007, both dialects | `migrations/{sqlite,postgres}/0007_revision_cache.sql` | applied by every acceptance test |
| The repositories | `crates/db/src/imports.rs`, `crates/db/src/secrets.rs` | through the acceptance tests |
| The revision cache | `crates/app/src/revisions.rs`, `crates/db/src/revisions.rs` | 10 |
| The import service | `crates/app/src/imports.rs` | through the acceptance tests |
| The source health sweep | `crates/db/src/imports.rs`, `crates/app/src/worker.rs` | 6 acceptance tests |
| The routes | `crates/app/src/routes/imports.rs` | 28 acceptance tests |
| Acceptance tests | `crates/app/tests/milestone_6.rs` | 28 |

---

## 2. The decisions that differ from the plan, and why

**The trait takes a fetcher; it does not make one.** The plan sketched
`async fn preview(&self, url: &str, creds: Option<&Credentials>)`. That shape
lets an adapter build its own HTTP client, which means the SSRF guard is a
convention an adapter author has to follow rather than something the type system
enforces. The trait now hands the adapter a `&dyn Fetcher` and an adapter that
wants to reach the network any other way has to add a dependency to do it. This
is transport for spec §11.5's "adapters use the shared safe fetcher", and it is
the difference between a guard and a suggestion.

**Capabilities come from the adapter, limits come from the instance.**
`FetchPolicy::for_source` takes the adapter's politeness interval and nothing
else. An adapter able to ask for a ten-gigabyte body or an hour-long timeout
could undo the guard it runs behind, which would make the guard advisory.

**A credential belongs to a pseud, not an account.** The plan's migration sketch
has `source_credentials.account_id`. Spec §11.6 says "no automatic copying
across pseuds", which an account-scoped row cannot honour: one reader's login for
a source would be usable by every face they present. The column is `pseud_id`,
and imports are scoped the same way.

**Deleted a credential's secret, and left the cascade to it.** The schema
declares `source_credentials.secret_id REFERENCES secrets(id) ON DELETE CASCADE`,
so the repository deletes the ciphertext and the row that named it goes with it.
A revocation that required a caller to remember a second delete is a revocation
that eventually does not happen.

**The secret is written before the row that names it.** `secret_id` is `NOT
NULL` and a foreign key, so the other order is refused by the database. The value
is encrypted against the credential's natural key — pseud, source, label —
because a row id does not exist yet at that point, and because the natural key is
what makes rotation replace one ciphertext rather than accumulate orphans.

**A preview fetches, in the request.** Spec §11.4 puts the metadata preview
before the confirmation, so it happens while the reader waits; doing it
out-of-band would be a spinner around a plan the reader confirmed without
seeing. It runs through the same guard as the worker with a shorter clock (20 s),
and it writes nothing at all, which is the property the tests pin.

**`POST /imports` returns `202` and the queue row names only the import.** The
URL, which can carry a private token, lives in the import row. Spec §11.6 asks
for no credentials in job payloads; a URL is the same class of secret.

**The retry is a new job on the same import.** `POST
/imports/:id/retry-failed-chapters` queues another attempt at an import that has
finished, and the attempt re-reads only the ordinals recorded as failed — and
only where the adapter advertises that it can address a chapter alone. A source
that cannot is fetched in bulk with the skip rule doing the work, which is
honest but is not the same guarantee.

---

## 3. What is not built, with the reason

Two things, and neither is a gap in the machinery: one is deferred by the plan,
and one is reconnaissance that has been done and an adapter that has not been
written.

**Preservation batches.** Spec §14.5 and the plan's fifth pitfall both put them
in Milestone 17, behind a documented permission basis and a dry-run report. The
destination field exists and accepts only `library`. Nothing in M6 could be
mistaken for the authorisation. `M6-10`.

**The remaining tier-1 adapters.** ffnet/fictionpress, the XenForo board family,
scribblehub, wattpad, the eFiction family, ficbook. Four — AO3, Royal Road,
Syosetu, eFiction — are built. Of the rest, none has fixtures:

* ~~**eFiction**~~ — **built**, and §7 is about it. It was the family worth doing
  first: one script, many archives, so one adapter covers nineteen hosts. Its
  fixtures are recorded in `crates/scrapers/tests/fixtures/efiction/` across five
  members, and `tests/fixtures/README.md` writes down what they establish.
* **ffnet/fictionpress, the XenForo boards and scribblehub** answer `403` to a
  plain request, and `cloudflare-challenge.html` records what that looks like.
  Reading them needs TLS impersonation or a browser, and the plan's §6 weighs
  that against the SSRF guard `SafeFetcher` provides — a browser subprocess
  cannot be DNS-pinned the way the guard pins a request. This is the point at
  which a signed-in session is the honest tool, and the work is left for one.
* **wattpad and ficbook** are reachable and simply not yet recorded.

`M6-02`.

---

## 4. What the tests found

Three defects in the boundary between Milestone 5's store and Milestone 6's first
use of it, and one parser defect. They are recorded in `docs/verification.md`
with the fix for each; the summary is that a store with no caller is a store
whose foreign keys have never been exercised, and this is where they were.

---

## 5. Syosetu, and the shape that made a preview expensive

The third adapter, and the first whose source spreads one work across more than one
page. Knowing which page holds what is most of the work, and one of the three is a
trap:

| Page | Holds |
|---|---|
| `/{ncode}/` | the episode list, **100 episodes at a time**, paginated by `?p=N` |
| `/novelview/infotop/ncode/{ncode}/` | title, author, summary, dates, status, word count, tags, and the *total* episode count |
| `/{ncode}/{episode}/` | one episode's prose |

**The work page does not state how many episodes the work has.** A 795-episode work
serves 100 rows and a pager; the number 795 appears only on the info page, as
`全795エピソード`. A preview that reads the work page and stops reports 100 of 795
chapters — and because the import trusts the preview, it stores a seventh of the
work and reports success. That is the plan's third pitfall ("a selector that matches
nothing returns zero chapters") in a different costume: not a selector that matched
nothing, but a paginated list read once.

So the adapter reads the info page first, walks every page of the episode list, and
then checks the two against each other. If they disagree it returns a parse failure
naming both numbers rather than a short list. Two checks, because a partial read can
look consistent:

* **The count.** The list yielded N episodes; the info page states M. `N != M` is a
  refusal.
* **The walk.** The pager advertises P pages; P pages must have been read. This one
  is defensive and no recorded fixture can reach it — where both are wrong the
  count check fires first and says more — which is recorded in the test file rather
  than left as silence.

The cost is real and is stated rather than hidden: previewing a 795-episode work is
one info request plus eight list requests, and importing it is 795 more. At the one
request per second Syosetu publishes that is about fourteen minutes, which is
tolerable because the import is resumable.

### What the port was wrong about here, again

The same three defects as Royal Road, plus two that are worse because they are
silent:

| The ported adapter | What this adapter does instead |
|---|---|
| `updated: now()` | reads `最新掲載日` |
| `published: … else now` | a parse failure is `None`, never the current time |
| `author_local_id: story_id` | the work id goes in `source_work_key` |
| `chapter_id: chapters.len() + 1` | the site's own episode number, which is in the URL |
| `let Ok(toc) = fetch(..) else continue` | a failed list page is an error, not a skipped 100 episodes |
| `fetch_chapter_text` → `String::new()` | a failed body is an error, never a blank chapter stored as success |
| `can_handle` by substring | host **and** a work-shaped path |

The fourth row down is the one worth dwelling on. `let Ok(..) = fetch(..) else
continue` in the ported code drops up to a hundred episodes of a work when one list
page fails, and the import completes and reports success — with a chapter list that
is short in the middle. Row five is the same failure for a single chapter: a failed
fetch becomes an empty string, and an empty chapter is stored. Neither raises
anything. Both are why the count check exists at all: it is the only thing that can
notice a hole the adapter made itself.

### Two smaller decisions

**Dates are Japanese local time.** The site writes `2012年 04月20日 21時58分` and
`2012/04/20 21:58` with no offset. The adapter reads them as JST (+09:00) and carries
the offset rather than converting it away. This is an inference — the page never says
so — and it is written down because if it is wrong it is wrong for every work on the
site. Read as UTC, every imported date would be nine hours out.

**The author's notes are set apart with `blockquote`.** The recorded episodes carry a
preface and an afterword around the story. Those are kept, and marked, because a
reader should be able to tell the author from the narrative. The wrapper is
`blockquote` rather than a `<div class="notes">` for a concrete reason: the crate's
sanitiser allows a fixed list of tags and attributes in which `div` and `class` do not
appear, so a classed wrapper would be stripped and the note would merge into the
prose. That is a rendering convention, not a claim about the site's markup, and it is
the kind of choice worth revisiting deliberately rather than inheriting.

---

## 6. The port from `ficnexus`, and what it was wrong about

M6's adapters are ported from `~/code/rust/ficnexus`, which already holds a Rust
translation of FanFicFare's per-site knowledge. The decision was made explicitly
and is worth restating with its limit: **the selector knowledge is what is worth
lifting, and it is not trustworthy on its own.** For Royal Road every one of these
was true of the ported adapter, and each is checked against the recorded pages
rather than against the ported code:

| The ported adapter | The recorded page | What this adapter does instead |
|---|---|---|
| `published: 0` | the page states `datePublished` | reads it from the page's structured block |
| `updated: Utc::now()…` | the page states `dateModified` | reads it from the page's structured block |
| `status: "ongoing"`, hardcoded | the page labels the work `COMPLETED` | parses the label, and reports `Unknown` for a label it does not recognise |
| `author_url: String::new()` | the structured block carries the profile URL | carries it |
| `author_local_id: fiction_id` | — | the fiction id goes in `source_work_key`, where it belongs |
| `chapter_id: (i + 1) as i32` | each row carries the site's chapter id | the site's own id is the chapter key |
| `h2.chapter-title`, and the `h1` selectors for the *author*, both fall back | `h2.chapter-title` matches **nothing** | the chapter title comes from `h1.font-white` |
| the body is `inner_html()`, unsanitised | — | sanitised on the way out of the crate |

The `h2.chapter-title` row is the one that matters, because the ported adapter
does not fail on it: it falls back to `format!("Chapter {}", i + 1)`, so every
chapter of every Royal Road work is titled `Chapter 1`, `Chapter 2`, … and the
author's own titles are discarded with nothing raised. That is exactly the failure
this milestone's fixtures exist to catch, and it is why the fixture test asserts
the title of the first chapter *and* the second rather than only that a title
exists.

`published: 0` is worth its own note. It is not a missing value; it is the epoch,
so a reader would see the work as published in 1970 — worse than the absent
timestamp the type already models.

### What porting did *not* mean

The other 51 ficnexus adapters still carry these defects, so the remaining sources
are **not** a copy job. Each needs the same four corrections before its fixture
test can pass: real dates or none, the source's own chapter id rather than a loop
index, a title selector that actually matches, and sanitisation. The ticket for
the bulk pass is that list, not "port 51 files".

Two defects in the ported code are structural rather than per-site and do not
carry over at all: it has no SSRF guard (its `fetch` is a bare `client.get(url)`,
where this crate's adapters receive a `&dyn Fetcher` and cannot construct a client
— §2), and it has no fixtures, so none of the above was ever checkable there.

---

## 7. The eFiction family, and the five things it taught

One module, `crates/scrapers/src/sites/efiction.rs`, covering nineteen archives
across eighteen hosts — the largest single piece of M6's adapter work, and the one
where recording pages first paid for itself twice over: the port's table was wrong
about the markup *and* about the dates, and neither error was visible without a
page to check against.

### The port's date table cannot read either recorded member

`parse_common_date` tries `%B %d, %Y` and `%d %b %Y`. The two members that were
recorded write `08/06/21` (tgstorytime) and `January 18 2022` — the second has no
comma, and `%B %d, %Y` requires one. So **both** worked members returned no date at
all from the port's parser, and the failure is silent: `Option<NaiveDate>` becomes
`None`, `None` becomes the epoch or the fetch time, and the work arrives looking
like it was posted today. A table of expected formats is a table of the formats
someone remembered.

Dates are parsed by shape instead, and the one genuinely ambiguous shape is
handled by naming the ambiguity rather than hiding it. `08/06/21` is 8 June or
6 August and the page cannot say which; the adapter reads it day-first — eFiction's
own default, and both members that use the numeric form are British archives — and
disambiguates against the future where the two readings straddle it, because a
publication date cannot be in the future. A date in a shape that is not recognised
becomes `None` rather than a guess.

### A chapter has two URLs, and the site's own link points at the print view

`viewstory.php?sid=N&chapter=K` is what a work page's chapter list links to, and it
is the **print** view: it links `printable.css` and runs `window.print()` on load,
in a bare `if (window.print)` rather than a fallback. `&textsize=0&chapter=K` is the
reading view. Both render byte-identical prose, in different containers —
`div.chapter` against `div#story` — and both members recorded behave the same way.

Following the site's own links is normally how a parser finds the right URL, and
here it finds the wrong one. The consequence is not theoretical:
`ninelivesarchive.com` disallows `viewstory.php?action=printable&*` in its
`robots.txt` by name. **Importing a library is reading**, so the adapter asks for
the reading view and reads `div#story`, with `div.chapter` as the fallback for a
member that answers with the print template anyway. The fixtures assert the prose
is identical either way, so the choice is about what is being asked of the archive
and not about what can be parsed.

### A value is what the member rendered as a value, and not the furniture around it

The metadata block is a run of `<span class="label">Name:</span> value` pairs, and
two things follow that are not obvious from that description:

* **Every class value is a link to `browse.php`, and everything else is chrome.**
  tgstorytime writes `Rated: Adult <a href="modules/epubversion/…">Download
  ePub</a>`, so a walk that harvested every anchor's text read the rating as
  `Adult Download ePub` — a rating that is not a rating, from a rule that looked
  reasonable. Distinguishing by the script a link points at is what separates a
  value from the furniture, without a list of label names to maintain.
* **Where values were rendered as links, the links are the values.** tgstorytime's
  `Characters` is one link whose text contains a comma —
  `Male to Female, Young Adult (20-26 yrs)` — and giantessworld's `Categories` is
  one link per value. Splitting rendered text on commas gets the first wrong and
  the second right by accident, and splitting on `/` breaks the family's own
  vocabulary, turning `Slow/Gradual Change` into `Slow` and `FF/m` into `FF`. The
  markup already says where the boundaries are; the text does not.

### Archive statistics and story metadata look identical, and are told apart structurally

Two of the three recorded content gates carry the archive's own totals —
`Members:`, `Series:`, `Stories:`, `Chapters:`, `Word count:`, `Reviewers:` — and
`Chapters:` and `Word count:` are also story fields. A parser that reads label
spans generically reports narutofic's 25,318 chapters and 47,323,633 words as one
work's. That is implausible rather than impossible, so it survives a glance, and it
would have survived into a reader's library.

The distinction is structural, not a name list: archive statistics sit in
`div#infoblock`, story metadata sits in a `div.content` that also carries
`Completed:`, and none of the three gates contains a `div.content` at all. The
adapter requires the block and the label together, and the fixture test asserts the
archive's totals are never read as the work's.

### Three outcomes that are not a story, and a fourth

An archive that will not serve a work answers **HTTP 200** with a page of its
ordinary furniture, so there is no status code to read. Four distinct outcomes were
being folded into two, and each now has its own error because each sends an
operator somewhere different:

| What the page is | Error | Why it is not one of the others |
|---|---|---|
| A moderation hold (`Access denied. This story has not been validated…`) | `Withheld` — **new in this milestone** | The work exists and the archive will not serve it. `NotFound` would send an operator looking for a typo in a URL |
| A content gate (age acknowledgement) | `AuthRequired` | The job pauses and asks the reader; a credential with `adult_allowed` satisfies it by following the page's own link, once |
| A challenge wall | `Blocked` | Detected by the interstitial's own markers, not by the absence of content — identification by absence turns the next new gate into "no such work" |
| Nothing at all | `NotFound` | |

`Withheld` is the milestone's only new error variant, and it is `is_transient() ==
false` and `needs_the_reader() == false`, which is the point: no retry lifts a
moderation hold and there is nothing for a reader to fix.

### The find that was not about eFiction at all

Every page in this family declares `charset=ISO-8859-1` in its `Content-Type` and
then emits Windows-1252 — byte `0x92` where the author typed a right single quote.
`SafeFetcher` decoded bodies with `String::from_utf8_lossy`, so every such byte
became `U+FFFD` and **a chapter title a reader would see was corrupted with nothing
failing anywhere**. Titles like `This week\u{fffd}s shows` pass every assertion a
test would think to make.

Bodies are now decoded by the declared charset through `encoding_rs`, which
implements the WHATWG alias table — the rule every browser applies, under which the
label `ISO-8859-1` *means* windows-1252 — with the document's own `<meta>` as the
fallback and a lossy decode only when there is nothing to go on. Valid UTF-8 wins
over a wrong declaration, because that is the case that cannot be wrong. The fix is
in the fetcher rather than in this adapter: it applies to every source, and it was
found by reading a recording as bytes instead of as a string.

### What is deliberately not done

* **Author's notes** (`div.notes` / `div.noteinfo`) are parsed and dropped. Folding
  them into `content_html` would mean inventing markup to delimit them inside prose
  the author wrote, and a chapter body here is the chapter's prose. The selector is
  recorded so the follow-up is a change to one function.
* **No bibliography**, so `bibliography` is false: a member's author page has not
  been recorded, and an adapter written against a page nobody has looked at is a
  guess.
* **Chapter-list pagination is refused rather than followed.** If a member's
  chapter list is paginated, the stated `Chapters:` count and the number of links
  found disagree and the adapter refuses loudly, naming both numbers, rather than
  importing the first page of a long work. Neither recorded member paginates, so
  the refusal is asserted by damaging a fixture rather than observed.
* **One source key for nineteen archives** means the fetcher's allow-list for
  `efiction` is eighteen hosts wide. That is inherent in treating the family as one
  source, and it is bounded by the list being a compile-time constant rather than
  anything a page can influence — but the widening is real and worth saying out
  loud.

---

## 8. Rate limits come from the source

Decided during this milestone, and written into the spec rather than left in the
code (spec §11.5, "Rate limits come from the source, and the floor is one request
a second").

The first version of the Royal Road adapter declared its own interval — 1,500 ms,
a number invented here. Syosetu then arrived needing a different number, and the
obvious next step was a third guess per adapter. That is the wrong shape: the
operator of a server knows what it can take, and a repository full of invented
numbers is stale the moment a site changes. So the fetcher reads each host's
`robots.txt`:

* **`Crawl-delay` becomes the gap.** Syosetu publishes `Crawl-delay: 1`, and that
  is now the number, taken from the site rather than chosen for it.
* **`Disallow` is enforced as a refusal.** A path the source forbids is not
  fetched. This is new behaviour beyond what was asked for, and it is the half of
  `robots.txt` that matters more: honouring only the delay would respect a site's
  slowness while ignoring its wishes about what may be read.
* **One second when nothing is published.** "No information" is not "no limit",
  and it is also the floor when a published delay is shorter or unreadable, so a
  malformed directive can never become a faster pace than the default.
* **A `404` is a site with no restrictions; any other failure is rules unknown.**
  That distinction is deliberate: refusing to import because a site's `robots.txt`
  is temporarily returning a 5xx would break a reader's import over a file that has
  nothing to do with their work, so the import proceeds at the default pace and the
  condition is logged for an operator.
* **Enforced in the fetcher, not the adapter.** An adapter that could opt out of
  pacing would make the rule advisory, which is the same reasoning that put the
  address checks there.

Two implementation notes worth keeping:

The robots *parser* is a pure function over a string (`crates/scrapers/src/robots.rs`),
so the policy is testable with no network at all — 20 tests cover wildcards,
anchors, most-specific-wins, `Allow`-breaks-ties, named groups, comments, case,
fractional delays and malformed values. The *fetching* is separate and covers
pacing, the floor, cache freshness and the refusal path.

The cache lock is not held across the fetch, because it would deadlock: fetching
reads the pacing, and the pacing reads the robots cache. Two concurrent reads of a
cold host can therefore both fetch `robots.txt` — one extra request to a
one-request-per-second host, which is a better trade than a lock that can hang an
import. That said, it means the cache is a **cache and not a mutex**: the design
guarantees at most a small number of duplicate reads, not exactly one.

### The bug this found

Reviewing the match rule before writing it turned up something worth recording. The
de-facto rule for a `User-agent` group is a case-insensitive **substring**: a group
named `Googlebot` applies to `Googlebot-Image`. Our product token is `Lorehaven`,
which contains `a`, `e`, `n`, `o`, `r`, `l` and `h` — so a `robots.txt` with a
stray `User-agent: a` would have captured us and applied that group's rules
silently, with the site having meant nothing by it. Group names shorter than three
characters are therefore held to an exact match. That is a deliberate deviation
from the letter of the standard, in the safe direction, and it has its own test.

---

## 9. Source reachability, measured

Recorded on 2026-09-10 with a plain HTTPS request and a browser `User-Agent`,
against the URL taken from the ficnexus adapter for each source:

| Source | Result | Consequence |
|---|---|---|
| Archive of Our Own | `200` | Adapter built, fixtures recorded |
| Royal Road | `200` | Adapter built, fixtures recorded |
| Syosetu | `200` | Adapter built, fixtures recorded |
| the eFiction family: nine of the eighteen hosts the port lists | `200` | Adapter built; the other nine refuse or no longer resolve (below) |
| **www.fanfiction.net** | **`403`**, served to a browser fingerprint | §9 below: readable through the declared escalation |
| **www.scribblehub.com** | **`403`** | Cloudflare, and its `robots.txt` is challenged too — rules unknown |
| **www.fimfiction.net** | **`403`**, served to a browser fingerprint | a solver is needed; the fingerprint alone does not pass it |
| **forums.spacebattles.com** | **`403`**, served to a browser fingerprint | a solver is needed (and the XenForo family generally) |

FictionPress runs the same software as FanFiction.net and is behind the same wall —
but as of 2026-09-11 it is a *stricter* configuration: the fingerprint that opens
FanFiction.net does not open FictionPress. That was measured, not assumed
(`is_bot_challenge`'s fixtures and the live test below).

**This table was recorded with a plain request, and a plain request is the one
thing these four refuse.** What each actually does about a browser fingerprint, and
what the escalation built for it can and cannot read, is measured in §9.

One row says *reachable* and no more, and *reachable* turned out not to be the
same question as *importable*. Answered for the whole family on 2026-09-11, by
reading each member's `robots.txt` and then requesting its work page:

| Members | Answer | Consequence |
|---|---|---|
| giantessworld.net, gluttonyfiction.com, narutofic.org, ncisfiction.com, spikeluver.com, starslibrary.net, thedelphicexpanse.com, thehookupzone.net, valentchamber.com | reachable; rules and pages both open | importable; giantessworld is read live by `live_verification.rs` |
| ninelivesarchive.com | `Crawl-Delay: 10`; work page open, **`viewstory.php?sid=*&chapter=*` disallowed** | a work page may be listed and no chapter may be read: the archive permits browsing and forbids reading |
| tgstorytime.com, sinfuldreams.com | `User-agent: *` / `Disallow: /` | **unimportable**, by the archive's own instruction |
| dark-solace.org, sunnydaleafterdark.com | `403` (Cloudflare) | readable through the declared escalation, as the four largest sources are |
| libraryofmoria.com, mttjustonce.net, mugglenetfanfiction.com, naiceanilme.net | the domain does not resolve | dead hosts sitting in a compile-time allow-list |

**Nine of eighteen are importable**, which is a smaller claim than eighteen and the
honest one. Two findings matter more than the count:

* **`tgstorytime.com` is one of the two members whose markup is recorded in full,
  and it is unimportable.** Its `robots.txt` is a single `Disallow: /`. The parser
  reads that site's pages correctly and the import still has to refuse them, and
  the refusal is asserted in `live_verification.rs` rather than assumed — an
  archive's own instructions are the one thing an importer does not get to
  override, and a parser that "worked" here would be a parser that ignored them.
* **A chapter has two URLs, and the obvious one is the wrong one.** `&chapter=K`
  is the *print* view — `printable.css`, `window.print()` on load — and it is what
  a table of contents links to; `&textsize=0&chapter=K` is the reading view.
  `ninelivesarchive.com` disallows `viewstory.php?action=printable&*` by name while
  leaving the reading view alone, so following the site's own links to find the
  chapter URL arrives at the one address the site has asked not to be fetched.
  §7 has the rest of it.

Fixtures from five members are committed under
`crates/scrapers/tests/fixtures/efiction/`, and `tests/fixtures/README.md` records
what they establish. Three of those five record something other than a work page,
which is itself the finding: these archives gate a warned story behind a
content-warning interstitial, and the recording shows the gate rather than the
story. The port's member list is eighteen hosts, and the four names an earlier
draft of this table carried — fanficauthors.net, lcfanfic.com, phoenixsong.net,
mediaminer.org — appear in none of them; they were invented, and the real list is
now in the fixtures README where it can be checked.

### What that means, and the decision it forces

An adapter for a blocked source cannot be **verified**, and a parser that cannot be
verified is the failure the plan's third pitfall names: a selector that matches
nothing returns zero chapters, the import records an empty work, and the reader
sees a success. So the question is whether to go through the wall at all.

#### First: what the wall is, and what it is not

Cloudflare's "Just a moment" is bound to the **TLS and HTTP/2 fingerprint** of the
connecting client. `reqwest`'s rustls fingerprint is one it rejects. That is why
changing the `User-Agent` changes nothing, why the refusal is not an IP block, and
why an ordinary browser walking through the same door is not challenged. It is a
bot-mitigation wall, and it is applied by a managed rule rather than by a person
reading a request.

**The sources' own rules are a different question, and they have now been read.**
Recorded 2026-09-11:

| Source | Its own `robots.txt` says |
|---|---|
| `www.fanfiction.net`, `www.fictionpress.com` | `User-agent: *` → **`Allow: /`**, `crawl-delay: 5`, disallowing `/secure/`, `/rs/`, `/ru/`, `/eye/`, `/m/` and `/*.php`. Story pages (`/s/…`) are allowed. Also `Content-Signal: search=yes, ai-train=no, use=reference`, and explicit `Disallow: /` for GPTBot, ClaudeBot, CCBot, Bytespider and friends |
| `www.fimfiction.net` | `User-agent: *` → `Allow: /` |
| `forums.spacebattles.com` | `User-agent: *` → `Allow: /`, plus named AI-training crawlers disallowed |
| `www.scribblehub.com` | the `robots.txt` request is itself challenged (`403`), so the rules are **unknown** — which by this crate's own policy means default pacing and a recorded condition, not a refusal |

**This inverts the framing.** FanFiction.net — the source that matters most and the
one this whole section has been about — does not merely fail to forbid an import:
it states `Allow: /` for any crawler, names its required `crawl-delay` of five
seconds, and reserves its `Disallow` for the AI-training crawlers. Its AI clause
distinguishes training from reference use, and a library import that stores a work
for people to read is on the crawling-and-reference side of that line rather than
the training side. (The `Content-Signal` block governs AI consumption specifically
— the preamble says so — so it is not itself a licence for an archive copy. The
licence is `Allow: /` with `crawl-delay: 5`, which is exactly a statement about
crawling.) So this is not a case of circumventing an archive's wishes, in the way
`tgstorytime.com`'s `Disallow: /` was. The wall says *not from that client*; the
site says *yes, at five seconds*.

#### Second: the two ways through are not the same kind of thing

ficnexus uses both, `primp` first and a browser as fallback, and an earlier draft
of this section treated them as one option. They are not.

**TLS impersonation (`primp`) does not cost the guard.** `primp` is a fork of
`reqwest` — it re-exports it and wraps its `ClientBuilder` — and it exposes
`resolve_to_addrs`, which is the exact API `SafeFetcher` already uses to pin a
host's addresses. It also exposes `redirect(Policy::none())`, `local_address`,
`https_only`, `cookie_store` and `no_proxy`. So the impersonating engine is built
from the *same* address list as the plain one, and keeps every property the guard
provides:

* we still resolve the hostname ourselves, filter to public addresses, and pin
  those exact addresses, so DNS rebinding remains closed;
* we still follow redirects by hand with `Policy::none()` and re-pin every hop, so
  a redirect to a private address is still refused;
* we still bound the body while it is read, so an oversized response is refused
  rather than buffered;
* and it is one request for one response — no subresources, no JavaScript.

The cost is not the boundary. It is **a forked HTTP/TLS stack**: `primp-reqwest`,
`primp-hyper`, `primp-h2`, `primp-rustls`, `primp-hyper-rustls` and
`primp-tokio-rustls`, plus `aws-lc-rs` underneath them. Security fixes to `rustls`
or `hyper` do not reach a fork on their own, so the cost is one of maintenance and
supply chain, paid every time those crates are patched — which is why the feature
is off by default in the crate and switched on once, for the binary.

**A headless browser is the option that actually spends the guard** — with one
qualification this section originally missed. Driving `chromium --dump-dom` *as a
subprocess* does its own DNS resolution, follows its own redirects, and loads
subresources, and `SafeFetcher` cannot pin a process. But a browser behind an
**HTTP service** — which is what FlareSolverr, Byparr and obscura-solverr all are
— is a process boundary instead: the importer sends one request to an
operator-configured address and the browser runs on the far side of it. The
importer's guard is intact; what changes is that the *solver's* request to the
source is not one the guard made. That is stated plainly in
`crates/scrapers/src/solver.rs`, and it is why the solver is only ever handed a
URL whose host the source itself declared.

#### Third: the pacing, which is not a detail

ficnexus records that `primp` passing the wall is not sufficient on its own —
back-to-back chapter fetches are refused even with the fingerprint, and the working
interval is enforced at eight seconds in its code, against FanFiction.net's own
stated `crawl-delay: 5`. Whatever is built has to be slower than the crate's
one-second default, and the source's own number is the floor. The escalation path
adds no pacing of its own: every step runs *through* the same `robots.txt` gate, so
`crawl-delay: 5` is still what paces an import from FanFiction.net.

#### What was built

Four tiers, in `crates/scrapers/src/`:

| Tier | Where | What it does |
|---|---|---|
| The plain transport | `safety.rs` | `reqwest`, pinned, unchanged |
| A browser fingerprint | `engine.rs` | the same request through `primp` with a coherent Chrome/Edge/Firefox/Safari fingerprint, still pinned |
| A solver service | `solver.rs` | the FlareSolverr v1 HTTP contract, which Byparr and obscura-solverr also speak — so the tool is a configuration value rather than an implementation |
| An archived copy | `archive.rs` | the Internet Archive's snapshot of a page the source will not serve, marked as an archived read via `Fetched::provenance` |

The transport is a seam inside `SafeFetcher` rather than a second fetcher: an
`Engine` is either `Plain` or `Impersonating`, and `attempt()` is the only place a
request is made, so pinning, hand-rolled redirects, bounded reads, `robots.txt`
gating and pacing live in one place regardless of which stack opens the socket. A
second fetcher written beside the first would have drifted from it within a
milestone.

Escalation is a chain, not a fallback that always runs: the configured transport
first, then the solver, then the archive, and each step runs **only** when the
previous one was answered with a detected bot challenge — never on a `404`, a
`robots.txt` refusal, or a rejected credential, because no different client changes
those answers. `is_bot_challenge` is written to be specific rather than eager, and
the test next to it exists because of a real mistake: `challenge-platform` is the
URL of Cloudflare's script and appears in the `<script>` tag of ordinary served
pages, including every `fanfiction.net` chapter and every `royalroad.com` page.
Matching on it would have marked successful reads as failures.

Nothing is escalated to that was not declared. `SourceAdapter::unblock()` defaults
to nothing, so every adapter that serves a plain request stays on the plain path;
an instance's `[imports]` section supplies the solver URL and the archive switch.
Neither half can grant the other's: an adapter asking for a fingerprint gets one
only if the binary carries the feature, and an instance with a solver offers it
only to a source that declared a wall.

#### What it actually does, measured

Verified live on 2026-09-11 against `https://www.fanfiction.net/s/12345678/1/`
through the real fetcher, with `crates/scrapers/tests/live_verification.rs`
asserting all three:

```
plain client: refused, as expected
browser fingerprint: 46808 bytes of the real chapter page, from https://www.fanfiction.net/s/12345678/1/
```

The refusal is asserted first and on purpose. A test that only asserted the success
would pass on a day the wall was switched off, and would go on passing after the
impersonation code had stopped working.

One trap worth recording, because reasoning alone got it wrong: **a fingerprint is
only coherent if the headers agree with it.** The first implementation set our own
`User-Agent` (`Lorehaven/0.1.0 (+import)`) on every request, and the wall refused
it — a Chrome ClientHello announcing a fanfiction importer agrees with nothing. The
agent is now left to the fingerprint, which has the consequence that *a
fingerprinted request does not identify itself as Lorehaven*. That is the real cost
of the technique, and it is the reason impersonation is declared per source rather
than applied to anything that challenges us.

The same reasoning applies to compression: `no_gzip()` is **not** set on the
impersonating engine, because a browser sends `Accept-Encoding: gzip, br, zstd` and
a fingerprint that omits them is not a browser's. The bound is unaffected — the
ceiling is applied to the bytes as they arrive *after* decompression, which is the
figure that matters for a decompression bomb.

#### The solver, run for real

Both tiers below were verified against a stub on 2026-09-11 and that was recorded
as the honest limit. It is no longer a limit: **Byparr 3.0.4 was installed and
run**, and the live suite passes against it. `live_verification.rs` gained
`a_solver_passes_a_wall_a_fingerprint_does_not`, which asserts four things in
order:

```
plain client: refused, as expected
browser fingerprint: also refused, which is why the solver tier exists
solver: 241315 bytes of the real story page via http://127.0.0.1:8191
guard: an undeclared host is refused before any solve is attempted
```

The second line justifies the third tier's existence, and it contradicts what this
plan earlier claimed. On **FimFiction a browser fingerprint is not enough**: three
browsers (Chrome, Firefox, Safari) were all refused there, against a
FanFiction.net that accepts one. A fingerprint is a per-host fact, not a
per-family one, and the chain exists because the sources do not agree.

Byparr's own log is the clearest statement of what these walls are:

```
INFO:     Challenge detected, waiting for it to clear...
INFO:     Clicked the challenge checkbox (attempt 1).
INFO:     Clicked the challenge checkbox (attempt 2).
INFO:     Done https://www.fimfiction.net/story/594215/... in 12.26s
```

An interactive checkbox, twice. That is not a header a client can set, which is
why tier two is a browser behind a process boundary and not more fingerprint work.
It also sets the cost: **one solve per page, about twelve seconds** — which is why
the pacing rule in §8 matters more here than anywhere else in the crate.

The same fetcher, against the other walled sources, reads all of them — including
FictionPress, which the fingerprint could not touch at all:

| Source | Plain | Fingerprint | Solver |
|---|---|---|---|
| FanFiction.net | refused | 46,808 bytes | 48,743 bytes |
| FictionPress | refused | refused | 35,740 bytes |
| FimFiction | refused | refused | 241,315 bytes |
| ScribbleHub | refused | refused | 33,742 bytes |
| SpaceBattles | refused | refused | 1,526,890 bytes |

#### Three bugs only a live service finds

**The endpoint was posted to as configured.** `imports.solver_url` is naturally
`http://127.0.0.1:8191`, and the client posted there — answering
`405 Method Not Allowed`, because that address serves the service's documentation
page and the contract's endpoint is `/v1`. Every stub test passed while this was
broken. A stub asserts on the *body* it is sent and never cared which path the body
arrived at. The client now accepts either spelling: a path the operator supplied is
kept, and a bare address gets the versioned one appended.

**Byparr has no session API at all.** The FlareSolverr v1 contract has one; the
maintained successor dropped it. `POST /v1` accepts a single body —
`LinkRequest{cmd, url, maxTimeout, blockMedia, returnOnlyCookies}` — and its source
mentions `sessions`, `session` and `cmd` zero times, over a browser it shares
internally. The proactive `sessions.create` was therefore parsed as a request for
the empty URL and answered
`502 Could not reach the target: ... Invalid url: "https://"`. The client had
been treating a failed creation as "carry on statelessly", so reads worked and
only the *log* was wrong — but an operator's solver log filled with errors about a
service that is working, and every page paid for a wasted navigation.

What replaced it is smaller and better than what was there. A session is now
**proven, not assumed**, and a one-page fetch never pays for one: the first request
goes stateless, because a session exists to make *later* requests cheaper, and
support is attempted only once a second request makes it worth having. A service
that answers "no" is remembered as stateless rather than asked again for every
page. On FlareSolverr proper the session is still used. A test over a real socket
asserts that a service answering `502` to a session command still serves all three
pages, is asked exactly once, and is not asked on the first request.

**The archive client followed redirects it had not checked.** The real archive
answers its entry URL with a `302` to the snapshot's own address, and the client
let `reqwest` follow that silently — a hop that could leave the archive, in a
crate whose premise is that an answer is not authority to request wherever it
points. Redirects are now followed by hand with every hop checked against the
archive's host, as the source fetcher does, and provenance records the address
actually read rather than the entry point that pointed at it.

#### What the archive tier is now known to do

archive.org is reachable again — it answered *"temporarily offline"* for the whole
of 2026-09-11 and now answers this address with a `429` rate limit — which allowed
the URL construction to be checked against the real service even where no snapshot
of a given FFN page exists:

- A missing snapshot is a **`404`**; a present one is a **`302`** to the resolved
  address. Entry URL and resolution cannot be told apart by shape: the entry URL
  already carries the `id_` modifier, and still redirects.
- The timestamp fallback is correct as written. The entry form with no timestamp
  resolves to the **newest** snapshot, verified landing on `20260826131158`.
- `id_` is the difference between a page and a wrapper around it, measured rather
  than asserted: the wrapped form of a page returned **636 kB** against **93 kB**
  for its `id_` form — the archive's toolbar and the rewritten URLs inside it.

Whether a real snapshot of a real FFN chapter parses is still unverified: no page
of the recorded work has a snapshot, and the tier needs an adapter to reach. What
is verified is everything up to the body.

---

## 10. Before this milestone can be tagged

1. The two pages, and a browser journey over them.
2. The remaining tier-1 adapters, each with recorded fixtures.
3. An M6 entry in `docs/sessions/`.
4. Whatever the journey finds — on M4 and M5 it found something both times.
