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
| The adapter crate, its trait and the registry | `crates/scrapers/src/{lib,registry,sites}.rs` | 177 in-crate |
| The safe fetcher and URL guard | `crates/scrapers/src/safety.rs` | in-crate |
| `robots.txt`: the source's own pace and path rules | `crates/scrapers/src/robots.rs` | 20 |
| Chapter sanitation | `crates/scrapers/src/sanitize.rs` | in-crate |
| The Archive-software adapter | `crates/scrapers/src/sites/ao3.rs` | 9 fixture tests |
| The Royal Road adapter | `crates/scrapers/src/sites/royalroad.rs` | 13 unit + 21 fixture tests |
| The Syosetu adapter | `crates/scrapers/src/sites/syosetu.rs` | 16 unit + 31 fixture tests |
| Recorded fixtures | `crates/scrapers/tests/fixtures/{ao3,royalroad,syosetu}/` | provenance in `fixtures/README.md` |
| The planning rules | `crates/domain/src/imports.rs` | 24 |
| Migration 0006, both dialects | `migrations/{sqlite,postgres}/0006_imports.sql` | applied by every acceptance test |
| The repositories | `crates/db/src/imports.rs`, `crates/db/src/secrets.rs` | through the acceptance tests |
| The import service | `crates/app/src/imports.rs` | through the acceptance tests |
| The routes | `crates/app/src/routes/imports.rs` | 17 acceptance tests |
| Acceptance tests | `crates/app/tests/milestone_6.rs` | 17 |

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
scribblehub, wattpad, the eFiction family, ficbook. Three — AO3, Royal Road,
Syosetu — are built. Of the rest, the eFiction family now has its reconnaissance
committed and the others do not:

* **eFiction** is the family worth doing next: one script, many archives, so one
  adapter covers nineteen hosts. Its fixtures are recorded in
  `crates/scrapers/tests/fixtures/efiction/` across five members, and
  `tests/fixtures/README.md` writes down what they establish — including that the
  ported `infobox` selector was never dead but pointed at the wrong page, that
  `div#chapterlist` is one member's skin rather than the family's, that a
  chapter's stable key is the `chapid` on its review link, and that these archives
  answer with three distinct outcomes (the story, an unvalidated-story refusal,
  and a content-warning gate) where the old design assumed two. What is left is
  the adapter, its variant layer and its fixture test.
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

## 7. Rate limits come from the source

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

## 8. Source reachability, measured

Recorded on 2026-09-10 with a plain HTTPS request and a browser `User-Agent`,
against the URL taken from the ficnexus adapter for each source:

| Source | Result | Consequence |
|---|---|---|
| Archive of Our Own | `200` | Adapter built, fixtures recorded |
| Royal Road | `200` | Adapter built, fixtures recorded |
| Syosetu | `200` | Adapter built, fixtures recorded |
| the eFiction family: tgstorytime.com, giantessworld.net, gluttonyfiction.com, narutofic.org, ninelivesarchive.com, and thirteen more listed by the port | `200` | **Reachable; reconnaissance recorded, adapter not built yet** |
| **www.fanfiction.net** | **`403`** | Cloudflare |
| **www.scribblehub.com** | **`403`** | Cloudflare |
| **www.fimfiction.net** | **`403`** | Cloudflare |
| **forums.spacebattles.com** | **`403`** | Cloudflare (and the XenForo board family generally) |

FictionPress runs the same software as FanFiction.net and is therefore behind the
same wall.

One row says *reachable* and no more. The eFiction members answer plain requests, so
an adapter for them can be verified and is ordinary work. It is still not done, and
the table says so rather than implying a delay is a difficulty — but the
reconnaissance behind it is not pending any more: fixtures from five members are
committed under `crates/scrapers/tests/fixtures/efiction/`, and
`tests/fixtures/README.md` records what they establish. Three of those five record
something other than a work page, which is itself the finding: these archives gate a
warned story behind a content-warning interstitial, and the recording shows the gate
rather than the story. The port's member list is eighteen hosts, and the four names an
earlier draft of this table carried — fanficauthors.net, lcfanfic.com, phoenixsong.net,
mediaminer.org — appear in none of them; they were invented, and the real list is now
in the fixtures README where it can be checked.

### What that means, and the decision it forces

For the blocked sources, an adapter cannot be **verified**, and a parser that
cannot be verified is the failure the plan's third pitfall names: a selector that
matches nothing returns zero chapters, the import records an empty work, and the
reader sees a success.

ficnexus carries a Cloudflare path: TLS impersonation via `primp` with a
`chromium --dump-dom` fallback. Porting it wholesale would undo part of the
security work M6 just did, and it is worth being explicit about why rather than
letting this look like an oversight:

* `SafeFetcher` resolves a hostname, checks the answers, and then **pins** them
  for the request, so a name cannot resolve to a public address for the check and
  a private one for the connection. A browser started as a subprocess cannot be
  pinned that way — it does its own resolution, follows its own redirects, and
  loads subresources.
* A `chromium --dump-dom` render also fetches images, stylesheets, fonts and
  anything a page's script asks for. Through the guard, every one of those is a
  decision we make in advance; through a browser, none of them is.

So the options are, in order of how much I would trust them:

1. **Do not support the blocked sources.** Honest, loses FanFiction.net, which is
   probably the single largest source for a fanfiction platform.
2. **A sanctioned browser path with its own guard.** Pre-validate the URL and
   every redirect target against the same IP rules, run the browser with no
   network access of its own (a proxy that enforces the allow-list, or a
   network namespace), cap the response size, and document the boundary as
   weaker than `SafeFetcher`'s. Real work, and testable — the guard's refusals
   can be asserted even when the site's page cannot be recorded.
3. **Port the ficnexus path as it stands.** Fastest, and the only one I would not
   recommend: it puts an unpinned fetcher into the one code path whose entire
   purpose is that a user-supplied URL cannot make the server read its own
   network.

Option 2 is the one I would build, and it is larger than the rest of M6's adapter
work. It needs a decision because it trades a documented security property for
coverage, which is the kind of trade the spec (§11.5, §11.7) leaves to the
operator rather than to the implementer.

---

## 9. Before this milestone can be tagged

1. The two pages, and a browser journey over them.
2. The remaining tier-1 adapters, each with recorded fixtures.
3. An M6 entry in `docs/sessions/`.
4. Whatever the journey finds — on M4 and M5 it found something both times.
