# Recorded fixtures

Pages fetched from the live sites, committed verbatim. A parser written from
memory of a site's markup is a parser written from a guess, and the guess fails
in the one place that matters: the selector that silently matches nothing.

**These are the contract.** When a source changes its markup, the fixture test
fails. That failure is the signal a person is needed — not an empty chapter list
that reads like a successful import.

## Rules

* Record the page from the live site; never hand-write one from what the parser
  expects. A fixture edited to match the parser tests nothing.
* Record the *whole* response body, not the fragment you think matters. The bug
  the AO3 adapter had — `div.chapter` matching the heading's own
  `div.chapter.preface.group` — was only visible in a real page.
* Record the site's own error page too. "Not found" and "the markup changed"
  need to be distinguishable, and they are only distinguishable if the error
  page is on file.
* Note the URL and the date in this file when you add one.

## ao3

| File | Source | Recorded |
|---|---|---|
| `work.html` | `https://archiveofourown.org/works/92356871` (redirects to chapter 1) | 2026-09-10 |
| `work-full.html` | `https://archiveofourown.org/works/92356871?view_full_work=true` | 2026-09-10 |
| `work-ongoing.html` | `https://archiveofourown.org/works/91806026` | 2026-09-10 |
| `not-found.html` | `https://archiveofourown.org/works/99999999999999` (`404`) | 2026-09-10 |

The three live pages are multi-chapter works chosen to cover: a completed work
whose chapter index lists each chapter's own id (`work.html`), the whole-work
page the chapter fetch actually reads (`work-full.html`), and an ongoing work
whose status label is `Updated:` rather than `Completed:` with ten chapters
posted of thirty planned (`work-ongoing.html`).

The `work.html` recording is a *chapter* view, which is what the site redirects
`/works/{id}` to. That is why the work page's chapter list comes from the
`select#selected_id` index rather than from the chapter elements: the work page
holds one chapter's body and the whole list of ids, and the whole-work page holds
every body.

## royalroad

| File | Source | Recorded |
|---|---|---|
| `work.html` | `https://www.royalroad.com/fiction/21220/mother-of-learning` | 2026-09-10 |
| `chapter-1.html` | `https://www.royalroad.com/fiction/21220/mother-of-learning/chapter/301778/1-good-morning-brother` | 2026-09-10 |
| `chapter-2.html` | `https://www.royalroad.com/fiction/21220/mother-of-learning/chapter/301781/2-lifes-little-problems` | 2026-09-10 |
| `not-found.html` | `https://www.royalroad.com/fiction/999999999/definitely-not-a-real-fiction` (`404`) | 2026-09-10 |

`Mother of Learning` (109 chapters, completed) is recorded because it exercises
both ends of this site's shape at once: a long chapter table, a `COMPLETED`
status label beside a title that also carries a fiction-type label, an author
with a profile URL, and a work whose last two "chapters" are announcements rather
than chapters — which the site lists as chapters and which this adapter therefore
lists as chapters too.

Two chapter pages are recorded rather than one because a chapter page *does not
state its own position in the work*. The adapter recovers the ordinal by matching
the page's title against the work's chapter list, and a single chapter would not
show that the same rule gives chapter 1 for one page and chapter 2 for another.

This is also the fixture set that disproves the earlier ported selectors: the
ported adapter read `h2.chapter-title`, `div[property='description']` and
`span[property='genre']`, and all three match nothing on the recorded pages.

### A note on what a fixture cannot show

Royal Road answers a chapter URL whose slug is wrong (`.../chapter/301781/whatever`
→ `200`) but returns `404` when the slug is absent entirely, so a chapter URL
cannot be built from the chapter id alone. That behaviour is not visible in a
fixture — it took three live requests to establish, and it is why
`fetch_chapter` re-reads the work page to resolve an id to a path instead of
constructing one. Recorded here so the next person does not have to rediscover it
by shipping the bug.

## syosetu

| File | Source | Recorded |
|---|---|---|
| `long-info.html` | `https://ncode.syosetu.com/novelview/infotop/ncode/n2267be/` | 2026-09-10 |
| `long-work-p1.html` | `https://ncode.syosetu.com/n2267be/` | 2026-09-10 |
| `long-work-p2.html` | `https://ncode.syosetu.com/n2267be/?p=2` | 2026-09-10 |
| `long-ep1.html` | `https://ncode.syosetu.com/n2267be/1/` | 2026-09-10 |
| `long-ep2.html` | `https://ncode.syosetu.com/n2267be/2/` | 2026-09-10 |
| `short-work.html` | `https://ncode.syosetu.com/n9525ii/` | 2026-09-10 |
| `short-info.html` | `https://ncode.syosetu.com/novelview/infotop/ncode/n9525ii/` | 2026-09-10 |
| `short-ep1.html` | `https://ncode.syosetu.com/n9525ii/1/` | 2026-09-10 |
| `short-ep2.html` | `https://ncode.syosetu.com/n9525ii/2/` | 2026-09-10 |
| `tanpen-info.html` | `https://ncode.syosetu.com/novelview/infotop/ncode/n2611bq/` | 2026-09-10 |
| `tanpen-work.html` | `https://ncode.syosetu.com/n2611bq/` | 2026-09-10 |
| `not-found.html` | `https://ncode.syosetu.com/n9999zz/` (`404`) | 2026-09-10 |

Three works, because this site has three shapes and one of them is a trap:

* **`n2267be`** (795 episodes) is the trap. Its episode list serves **100 episodes
  per page** and paginates with `?p=N`, and the number 795 appears *only* on the
  info page as `全795エピソード`. An adapter that reads the work page and stops
  reports 100 chapters; one that reads two pages reports 200. Both import a
  fraction of the work and report success. Pages 1 and 2 are recorded *and the
  work is deliberately left incomplete on disk*, because the test that matters is
  the one asserting the adapter refuses this input.
* **`n9525ii`** (6 episodes on one page) is the ordinary case: short enough that
  every page describing it could be recorded, so a complete assembly is testable.
* **`n2611bq`** is a 短編 — a one-shot. Its work page holds the prose directly and
  has no episode list at all, and its info page has no episode-count element. The
  two absences are the point: they are what distinguishes this shape from a
  serialized work with one episode.

Two of these episodes (`short-ep1`, `short-ep2`) carry a preface *and* an
afterword around the story, which is what the body-extraction test uses to check
that the author's notes are kept and set apart rather than silently merged into
the prose.

### Facts a fixture cannot carry

Recorded here because they were established by live requests and would otherwise
have to be rediscovered by shipping the bug:

* An episode's URL segment **is** its position in the work — `/{ncode}/1/` states
  `1/795`, `/{ncode}/2/` states `2/795`. That is what lets a single chapter be
  re-read without the work page, and it is asserted in the fixture test.
* A long work paginates its **episode list**, not an individual episode. Checked
  rather than assumed, because the alternative would have needed joining logic,
  and importing the first page of every long chapter is exactly the silent failure
  this directory exists to prevent.
* Dates are Japanese local time with no offset written anywhere on the page. The
  adapter reads them as JST (+09:00); read as UTC they would all be nine hours out.
* `novel18.syosetu.com` is a separate host behind an age gate and is deliberately
  out of scope. No page from behind that gate could be recorded, so no parser
  could be written against it honestly — which is why the adapter neither claims
  its URLs nor lists it in `hosts()`.

## efiction

One family, many members. eFiction is not a site but a script that hundreds of
archives run, each with its own skin. These recordings are from five of them, and
they disagree with each other in ways that matter — which is the whole reason the
family needs a variant layer rather than one parser.

| File | Source | Recorded |
|---|---|---|
| `tgstorytime-work.html` | `https://www.tgstorytime.com/viewstory.php?sid=6369&index=1` | 2026-09-10 |
| `tgstorytime-chapter-1.html` | `https://www.tgstorytime.com/viewstory.php?sid=6369&chapter=1` | 2026-09-10 |
| `tgstorytime-chapter-2.html` | `https://www.tgstorytime.com/viewstory.php?sid=6369&chapter=2` | 2026-09-10 |
| `tgstorytime-story-1.html` | `https://www.tgstorytime.com/viewstory.php?sid=6369&textsize=0&chapter=1` | 2026-09-10 |
| `tgstorytime-access-denied.html` | a story id that is not validated | 2026-09-10 |
| `giantessworld-work.html` | `https://www.giantessworld.net/viewstory.php?sid=11369&index=1` | 2026-09-10 |
| `giantessworld-chapter-1.html` | `https://www.giantessworld.net/viewstory.php?sid=11369&chapter=1` | 2026-09-10 |
| `giantessworld-chapter-2.html` | `https://www.giantessworld.net/viewstory.php?sid=11369&chapter=2` | 2026-09-10 |
| `giantessworld-story-1.html` | `https://www.giantessworld.net/viewstory.php?sid=11369&textsize=0&chapter=1` | 2026-09-10 |
| `giantessworld-access-denied.html` | a story id that is not validated | 2026-09-10 |
| `gluttony-content-warning.html` | `https://www.gluttonyfiction.com/viewstory.php?sid=313` | 2026-09-10 |
| `narutofic-content-warning.html` | `https://www.narutofic.org/viewstory.php?sid=11545` | 2026-09-10 |
| `ninelives-content-warning.html` | `https://www.ninelivesarchive.com/viewstory.php?sid=3205` | 2026-09-10 |
| `cloudflare-challenge.html` | a member behind a JS challenge | 2026-09-10 |

The URLs for the two worked examples are the ones those pages link to themselves
— a work page's own "Table of Contents" link, and a text view's own "Next Page"
link. The three content-warning recordings are named for the id each page's own
acknowledgement link carries, which is how the requested story is recoverable
from an interstitial that does not otherwise name it.

### The two-page shape

eFiction serves one work across more than one kind of page, and this is the fact
the port got wrong:

* **The work page** (`viewstory.php?sid=N&index=1`) carries the title, the
  metadata labels, the summary and the chapter list. It carries no chapter prose.
* **A chapter page** (`viewstory.php?sid=N&chapter=K`) carries one chapter. Its
  metadata — including the summary — is in `div.infobox`, its heading is
  `.chaptertitle`, and the author's notes are `.notes` / `.noteinfo`.
* **A chapter's text view** (`...&textsize=0&chapter=K`) is the same chapter
  under a different skin, and it is the one that renders the prose in `div#story`
  with the story block in `div.storyinfo`. `div#story` does **not** exist on the
  plain chapter page.

An earlier note in this repository recorded that the ported adapter's `infobox`
selector "matched zero markup". That was wrong twice over, and the mistake is
instructive: it was checked against the *work* page, where `div.infobox` really
does not appear. `div.infobox` is exactly where a *chapter* page keeps the
summary and the labels. The selector was not dead; it was pointed at the wrong
page, and the conclusion drawn from that was wrong with it.

### The chapter key is on the work page, but not behind one selector

`div#chapterlist` holds one line per chapter for tgstorytime: its ordinal, a link
to `viewstory.php?sid=N&chapter=K`, the chapter's title, the author, and a
**review link carrying `chapid=NNNN`**. That `chapid` is the site's own
identifier for the chapter, and it is available at preview time without fetching
a single chapter.

**But `div#chapterlist` belongs to that member's skin, not to the family.**
giantessworld's work page has no such container, and still carries thirteen
`chapter=K` links and thirteen `chapid` values. A parser that looks for
`#chapterlist` reads tgstorytime and imports nothing from giantessworld while
reporting success — which is the failure this directory exists to prevent, in the
form it actually takes on this family.

The port keyed chapters by its loop index instead of by any of this. An index is
a position, not an identity, and it changes the moment a chapter is inserted.

### What the two worked members disagree about

| | tgstorytime | giantessworld |
|---|---|---|
| Chapter list container | `div#chapterlist` | none; links are loose in the page |
| Summary on the work page | `div.summarytext` | a `Summary:` label span |
| Chapter titles | the author's own (`In the beginning - Chapter 1`) | generated (`Chapter 1`) |
| Chapter count | 19 | 13 |

Both carry `Completed:`, `Word count:`, `Read:`, `Published:` and `Updated:` as
label spans, with the value in the markup that follows the span — and they
disagree about the date format inside those values, which is what makes the date
parser a variant rather than a function.

### Three outcomes, not two

A story id that is not validated answers with the site's own error block:
`Access denied. This story has not been validated by the administrators.` The
page still carries the site's title and chrome. It is not a `404`, and it is not
the markup having changed — it is a third outcome, and an adapter that folds it
into either of the other two will report a moderation state as a missing story
or as a parse failure. Both recordings are here so the distinction can be
asserted rather than assumed.

### The content-warning interstitial

Three of these members answer a request for a warned story with a gate page
instead of the story: *"This content is for people aged 12 years and over"*,
*"you must be of legal age in your region"*, *"Age Consent Required"*. Each
carries the id in its own acknowledgement link, with the member's own parameter
name — `warning=2`, `warning=6`, `ageconsent=ok&warning=5`. This is why those
three recordings are interstitials and not work pages: the probe that recorded
them asked each member for a story, and this is what three of them answered. An
adapter may not treat such a page as a story, and a preview that did would report
a title and no chapters.

`narutofic-content-warning.html` also carries `Members:`, `Series:`, `Stories:`,
`Chapters:`, `Word count:` and `Reviewers:` as label spans. Those are that site's
*archive statistics*, not the requested story's metadata. A parser that reads
label spans generically picks up a story whose word count is the whole archive's.

### One member is behind a challenge

`cloudflare-challenge.html` is a Cloudflare interstitial — `Just a moment...`,
`Enable JavaScript and cookies to continue`. It is recorded because it is the
reason a class of these archives cannot be read by a plain request, and because a
fetched page that looks like this must be recognised as a block rather than
parsed as a story with no title.
