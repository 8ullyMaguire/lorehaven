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
