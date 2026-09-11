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

### One work, three pages, and only one of them is the reading view

eFiction serves one work across more than one kind of page, and this is the fact
the port got wrong:

* **The work page** (`viewstory.php?sid=N&index=1`) carries the title, the
  metadata labels, the summary and the chapter list. It carries no chapter prose.
* **A chapter's reading view** (`viewstory.php?sid=N&textsize=0&chapter=K`) is
  the reader's page. It links the member's own `style.css`, runs no script on
  load, and renders the prose in `div#story`. It carries **no**
  `div.chaptertitle`, so a page of this shape cannot say which chapter it is.
* **A chapter's print view** (`viewstory.php?sid=N&chapter=K`) is the *same
  chapter* — byte-identical prose — rendered by the print template. It links
  `printable.css` and fires `window.print()` on load, in a bare
  `if (window.print)` rather than in a fallback branch. Its metadata block is
  `div.infobox`, its heading `.chaptertitle` (`TITLE by AUTHOR`), the prose is in
  `div.chapter`, and the author's notes are `.notes` / `.noteinfo`.

**A table of contents links to the print view.** That is the trap awaiting
anyone who follows the site's own links to find the chapter URL: `chapter=K`
without `textsize=0` is what a work page's chapter list points at, and it is a
request to print. On `ninelivesarchive.com` that matters concretely — its
`robots.txt` disallows `viewstory.php?action=printable&*` by name while leaving
the reading view alone. Importing a library is reading, so the adapter asks for
the reading view and reads `div#story`, falling back to `div.chapter` for a
member that answers with the print template anyway. Both recordings are here so
the two views can be compared rather than trusted.

An earlier note in this repository recorded that the ported adapter's `infobox`
selector "matched zero markup". That was wrong twice over, and the mistake is
instructive: it was checked against the *work* page, where `div.infobox` really
does not appear. `div.infobox` is exactly where a *chapter* page keeps the
summary and the labels. The selector was not dead; it was pointed at the wrong
page, and the conclusion drawn from that was wrong with it.

### The headings are not where the title is

`tgstorytime.com`'s work page carries **two** elements with `id="pagetitle"`: the
header holding `<a>TITLE</a> by <a href="viewuser.php?uid=N">AUTHOR</a>`, and a
second, beside it, holding only a `Report` link. An adapter that took the first
`#pagetitle` on the page is reading whichever of the two the skin happens to emit
first. The header is the one that contains a `viewuser.php` link, and that is how
it is found.

### Labels, values, and what is not a value

A metadata block is a run of `<span class="label">Name:</span> value` pairs, and
the value ends where the next label begins. Two things about that are worth
writing down, because both are visible in these recordings and both are easy to
get wrong:

* **A class value is a link to `browse.php`, and everything else in a block is
  furniture.** `tgstorytime.com` writes `Rated: Adult <a
  href="modules/epubversion/…">Download ePub</a>`, so a walk that harvested every
  anchor's text reads the work's rating as `Adult Download ePub`. Matching the
  browse script and disregarding other anchors is what separates a value from the
  chrome around it.
* **Where the member rendered values as links, the links are the values.** On
  `tgstorytime.com`, `Characters` is a *single* link whose text contains a comma —
  `Male to Female, Young Adult (20-26 yrs)` — while on `giantessworld.net`,
  `Categories` is one link per value. Splitting the rendered text on commas gets
  the first wrong and the second right by accident; and splitting on `/` breaks
  the family's own vocabulary, turning `Slow/Gradual Change` into `Slow` and
  `FF/m` into `FF`.

### The chapter key is on the work page, but not behind one selector

`div#chapterlist` holds the chapter list for tgstorytime — **one such element per
chapter**, all sharing the id, which is invalid HTML that every browser accepts.
It carries the ordinal, a link to `viewstory.php?sid=N&chapter=K`, the title, the
author, and a **review link carrying `chapid=NNNN`**. That `chapid` is the site's
own identifier for the chapter, and it is available at preview time without
fetching a single chapter.

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
| Chapter list container | `div#chapterlist`, one per chapter | none; links are loose in the page |
| Summary on the work page | `div.summarytext` | a `Summary:` label span |
| Where the rating lives | `div.storyinfo`, beside the block | `Rated:` inside the block |
| Date format | `08/06/21` | `January 18 2022` |
| Chapter titles | the author's own (`In the beginning - Chapter 1`) | generated (`Chapter 1`) |
| Chapter count | 19 | 13 |
| Rating | `Adult` | `X` |
| `Warnings` label | absent | one, whose text is a sentence |

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
label spans generically picks up a story whose word count is the whole archive's
— 47,323,633 words over 25,318 chapters, which is implausible rather than
obviously impossible, and so survives a glance.

The two are told apart **structurally, not by name**: archive statistics sit in
`div#infoblock` and story metadata sits in a `div.content` that also carries
`Completed:`. Every one of the three gate recordings confirms it — none of them
contains a `div.content` at all. A block that has `Chapters:` and `Word count:`
but no `Completed:` is the archive, not the work.

### One member is behind a challenge

`cloudflare-challenge.html` is a Cloudflare interstitial — `Just a moment...`,
`Enable JavaScript and cookies to continue`. It is recorded because it is the
reason a class of these archives cannot be read by a plain request, and because a
fetched page that looks like this must be recognised as a block rather than
parsed as a story with no title.

### What the members actually answer

These recordings say what the *markup* looks like. They do not say which members
will serve it, and that is a separate question with a separate answer — asked on
2026-09-11 by reading each member's `robots.txt` and requesting its work page, and
recorded here because the adapter's own allow-list claims eighteen hosts and a
reader deserves to know how many of them are real.

| Members | Answer |
|---|---|
| `giantessworld.net`, `gluttonyfiction.com`, `narutofic.org`, `ncisfiction.com`, `spikeluver.com`, `starslibrary.net`, `thedelphicexpanse.com`, `thehookupzone.net`, `valentchamber.com` | reachable; work page and chapters open |
| `ninelivesarchive.com` | reachable; `Crawl-Delay: 10`, work page open, **chapter views disallowed** (`viewstory.php?sid=*&chapter=*`) |
| `tgstorytime.com`, `sinfuldreams.com` | `User-agent: * / Disallow: /` — the whole archive |
| `dark-solace.org`, `sunnydaleafterdark.com` | Cloudflare 403 to a plain request |
| `libraryofmoria.com`, `mttjustonce.net`, `mugglenetfanfiction.com`, `naiceanilme.net` | the domain no longer resolves |
| one unspecified member | a Cloudflare JS challenge (`cloudflare-challenge.html`) |

Three consequences, and none of them is the adapter's to fix:

* **`tgstorytime.com` is unimportable**, and it is one of the two members whose
  markup this directory records. The parser reads it; the archive's own
  instruction is `Disallow: /`; the import refuses. That refusal is asserted in
  `tests/live_verification.rs` rather than merely noted here, because an adapter
  that quietly worked around it would be ignoring the only instruction the
  archive gave.
* **`ninelivesarchive.com` permits a work page and forbids every chapter.** The
  import is therefore allowed to see the metadata and forbidden to read the
  prose, which is the archive's choice and produces a failed import rather than a
  partial one.
* **Four hosts in the adapter's allow-list are dead.** They are kept because the
  list is what the adapter *claims* rather than what currently answers, and a
  member that comes back should not need an adapter change — but an operator
  counting the family's reach should count nine, not eighteen.

`valentchamber.com` is the one member with a considered position rather than a
blanket rule: `Allow: /` for `User-agent: *`, with `Content-Signal: search=yes,
ai-train=no, use=reference` and explicit `Disallow` for the named AI-training
crawlers. Nothing there forbids an import that stores a work for readers to read
from this instance; it does forbid training a model on it, which this import
does not do and which the operator of an instance is the one who has to keep
true.

### The bytes are not what the pages say they are

Every one of these recordings declares `charset=ISO-8859-1` in its
`Content-Type` and then emits Windows-1252: byte `0x92` where the author typed a
right single quote, `0xA3` for a pound sign. Decoded as true Latin-1 the first is
a C1 control character; decoded as UTF-8 it is a replacement character; either
way the chapter title a reader sees is corrupted and nothing fails. These files
are therefore read through `lorehaven_scrapers::safety::decode_body`, which
follows the WHATWG alias table and reads the label `ISO-8859-1` the way every
browser does — as windows-1252. A fixture read with `std::fs::read_to_string`
does not decode at all, and one read lossily loses the apostrophes that
`tests/efiction_fixtures.rs` asserts on.

## ffnet

Two hosts, one script. FanFiction.net and FictionPress run the same software and
serve the same shapes, and these recordings exist because assuming that is the
same as knowing it. They were recorded on 2026-09-11, and **the recording needed
the unblock path**: a plain client is refused with `403` by both.

| File | Source | Recorded |
|---|---|---|
| `ffnet-work.html` | `https://www.fanfiction.net/s/12345678/1/` (2 chapters) | 2026-09-11 |
| `ffnet-work-complete.html` | `https://www.fanfiction.net/s/5782108/1/` (122 chapters, `Status: Complete`) | 2026-09-11 |
| `ffnet-chapter-2.html` | `https://www.fanfiction.net/s/12345678/2/Jillian-Holtzmann-Ace-Attorney` | 2026-09-11 |
| `ffnet-not-found.html` | `https://www.fanfiction.net/s/99999999999999/1/` (an id that does not exist) | 2026-09-11 |
| `fictionpress-work.html` | `https://www.fictionpress.com/s/3280165/1/Unrelenting` (17 chapters) | 2026-09-11 |
| `fictionpress-work-second.html` | `https://www.fictionpress.com/s/2171761/1/Vampiric-Desires` (4 chapters) | 2026-09-11 |
| `fictionpress-not-found.html` | `https://www.fictionpress.com/s/99999999999999/1/` | 2026-09-11 |

`FanFiction.net` was recorded through the browser fingerprint (`primp`) and
`FictionPress` through the solver service, which is not a convenience: measured on
the same day, FanFiction.net accepts a fingerprint and FictionPress refuses it.
The two hosts are the clearest example in this repository of a wall being a
property of one host rather than of the software it runs, and recording them
needed two different mechanisms to say so.

`ffnet-work` and `ffnet-work-complete` are the pair that matters. The two-chapter
work **has no `Status:` field at all**, and the completed 122-chapter work has
`Status: Complete` — so the field is present or absent, and a work that does not
carry it must be recorded as `unknown` rather than as ongoing. The ported code
hardcoded "ongoing" for every work; on this site that is the wrong answer half the
time with nothing to indicate it.

### The chapter list is `#chap_select`, and it is complete

Every chapter of a work is an `<option>` in `#chap_select`, all of them: 122 for
the long work, 17 and 4 for the two FictionPress ones. There is no pagination to
walk and no separate table of contents. The option's `value` is the ordinal and
its text is `{ordinal}. {title}`.

So on this source **the ordinal is the source's own chapter key**, because the
site has nothing better and says so — there is no per-chapter id in the list. That
makes the port's index-as-key defect harmless *here*, which is worth stating
explicitly rather than being quietly grateful for: the rule is to use the source's
own key, and on this source the ordinal is it. The chapter's own address is
`/s/{work id}/{ordinal}/{slug}`, and the slug is decorative — the same chapter is
served with or without it, so it must not be part of the key.

The list is rendered **twice** per page, in the top and bottom navigation, and the
two are identical. Reading the first is what the adapter does; the recording is
here so that a page where they disagree can be recognised rather than guessed at.

### What the two siblings disagree about

Same script, different answers. Each of these is a defect waiting for a parser
that assumes the first host it saw was the shape of the source.

| | FanFiction.net | FictionPress |
|---|---|---|
| Attribute quoting | unquoted (`id=chap_select`, `value=1 selected`) | quoted (`id="chap_select"`, `value="1" selected=""`) |
| Visible dates | `3/14/2015` (numeric) | `Jun 20, 2016` (abbreviated month) |
| `<title>` on chapter 2 | `… Chapter 2, a ghostbusters fanfic` | `Vampiric Desires Vampiric Desires Chapter 1, a fantasy fiction` |
| Untitled chapters render as | `2. Chapter 2` | `2. Vampiric Desires Chapter 2` |
| Characters field | present | **absent on both recordings** |

The quoting difference is invisible to a real HTML parser and lethal to a
hand-rolled regex — during reconnaissance it produced a false finding that
FictionPress omits its first chapter, because `selected=""` did not match a
pattern written for `selected>`. The chapter list was complete all along. This is
the argument for writing the parser against a parsed document rather than against
the bytes, and for recording fixtures rather than reasoning about them.

The date difference is the same lesson as the eFiction family's, in a smaller
space: the two hosts of one script write the same field two ways, so a date parser
that reads one shape returns `None` for the other host and the work arrives with no
published date at all. **The visible text is not the value to read.** Both hosts
carry the real timestamp in a `data-xutime` attribute — epoch seconds — and that
is what the adapter reads:

```
Updated: <span data-xutime='1426348782'>3/14/2015</span>      (FanFiction.net)
Updated: <span data-xutime="1466437099">Jun 20, 2016</span>   (FictionPress)
```

The characters field is absent from both FictionPress recordings and present on
both FanFiction.net ones, which settles a question the shape of the line raises:
the metadata is one ` - `-delimited run with **three unlabeled fields** in it —
language, genres, characters — and they cannot be read by position, because a work
with no listed characters has two where another has three. The adapter identifies
the language against the site's own short list of languages and tells genres from
characters by shape, and the recordings are here to hold that to.

### The prose, and the id that is not the one you want

The chapter body is `div#storytext`, and it is nested inside `div#storytextp` —
one character apart, so a prefix match on `storytext` selects the container *and*
the prose, and a body read that way arrives wrapped in its own wrapper. The exact
id is the selector.

### Not found, and a phrase that is not a moderation hold

Both hosts answer a missing work with **HTTP `200`**, and the page carries no
`profile_top`, no `storytext` and no `chap_select`. Status cannot be used; the
document has to be read.

The page's heading is `Story Not Found`. It *also* contains the sentence **"Story
is unavailable for reading."** — boilerplate that appears on this page for a work
that simply does not exist. Nothing may therefore be inferred from that sentence
alone, and in particular it must not be read as the moderation hold the eFiction
adapter distinguishes: doing so would send an operator to look for a takedown that
never happened.

**A genuinely withheld FanFiction.net work has not been recorded**, so this
adapter claims `NotFound` for this page and claims nothing about a hold. That is
the honest limit of these recordings, and the place to extend them is here rather
than in a guess.

### One thing an operator should know

Both hosts serve every one of these pages — the missing-work page included — with
`NOARCHIVE` in a `robots` meta tag, FanFiction.net capitalised (`<META NAME='ROBOTS'
CONTENT='NOARCHIVE'>`) and FictionPress lowercase (`<meta name="ROBOTS"
content="NOARCHIVE">`), which is the sibling difference again in one more place.
That is a directive about search-engine caches rather than about a reader keeping a
copy of a work they are reading, and it is recorded here rather than resolved,
because an instance's operator is the one who decides what their instance stores.

## wattpad

| File | Source | Recorded |
|---|---|---|
| `story.json` | `https://www.wattpad.com/api/v3/stories/410445604` | 2026-09-11 |
| `part-text.html` | `https://www.wattpad.com/apiv2/?m=storytext&id=1623966332&page=` | 2026-09-11 |
| `part-text.html.gz` | the same response, **as the site sent it** (`content-encoding: gzip`) | 2026-09-11 |
| `part-empty.html` | `https://www.wattpad.com/apiv2/?m=storytext&id=1623782492&page=` (the `Photo Gallery` part) | 2026-09-11 |
| `story-page.html` | `https://www.wattpad.com/story/410445604-the-older-swan-paul-lahote` | 2026-09-11 |
| `part-page.html` | `https://www.wattpad.com/1623966332-…-chapter-one` | 2026-09-11 |
| `robots.txt` | `https://www.wattpad.com/robots.txt` | 2026-09-11 |
| `missing-story.json` | `https://www.wattpad.com/api/v3/stories/999999999999` (`400`) | 2026-09-11 |
| `missing-story.html` | `https://www.wattpad.com/story/999999999999-definitely-not-real` (`404`) | 2026-09-11 |

`The Older Swan | Paul Lahote` — 31 parts, 13 tags, ongoing — is recorded because
it is the shape that made this adapter's design questions concrete: parts that are
not prose, a word count that does not exist, and a metadata path and a prose path
that the site's own `robots.txt` treats differently.

### The two halves are on two paths, and only one is permitted

`Disallow: /apiv2/*` is the line that matters. The story document at
`/api/v3/stories/{id}` is on none of the disallowed prefixes and answers a plain
client, so **a preview of a Wattpad work needs no allowance from anybody**. The
prose at `/apiv2/?m=storytext&id={part}&page=` is disallowed by name, so **every
chapter body is one the site has asked us not to read**.

That is not a defect in the adapter and it is not worked around here. Under the
default configuration the shared fetcher refuses the chapter fetch with a message
naming the rule, which is what spec §11.5 asks for; under `imports.honour_robots
= false` — an operator's own decision, on their own instance — the same code reads
it with no change to the adapter. `tests/live_verification.rs` asserts both
outcomes against the live site in one test, because a pair is what makes either
half mean anything.

The stories themselves are not behind a challenge: a plain client was served all
of these, so this source needs no solver and no fingerprint. Its wall is its
`robots.txt` and nothing else.

### One request is a whole chapter

The prose endpoint takes a `page` parameter and the reader app uses it to split a
part across screens. Measured on 2026-09-11 against two parts: **`page=` empty is
the whole chapter, byte-identical to the numbered pages joined** — 13,680 bytes
against `page=1` + `page=2`, and 18,822 against `page=1` through `page=3`. So the
adapter asks for the whole thing once. An adapter that walked `page=1..n` would
spend five requests to get the same bytes, and one that read only `page=1` would
import a fifth of every chapter and report success.

### The response is compressed whether or not you asked

`part-text.html.gz` and `part-text.html` are the **same response recorded twice**
— 12,514 compressed bytes and 25,434 plain — and both are here because the site
answered `content-encoding: gzip` to an explicit `Accept-Encoding: identity`,
then answered a later request to the same URL in plain text. The behaviour varies
by edge, so it is not something an adapter can predict either way.

This is the case where the failure would have been silent: the compressed bytes
would be stored as a chapter body, decoded as text, and the import would report
success with a chapter full of mojibake in the database. It is handled in the
shared fetcher (`safety::decode_response_body`), not here, and the pair of
recordings is what makes it testable against real bytes rather than against a
gzip this repository made itself.

### What the site does not publish

* **A word count.** `length` is a *character* count: the recorded work's Chapter
  One is `length: 18380` against a `wordCount: 3676` on the same chapter's own
  page — a ratio of five, which is what characters look like. Per-part word
  counts exist and are not in the story document, so summing them would cost one
  request per part at preview time. The adapter reports **no word count**, which
  is the honest answer; passing 291,689 off as one would be wrong by a factor a
  reader would not check.
* **Category names.** `categories: [6, 0]` is integers with no published table
  behind them. Dropped rather than guessed at. The author's own tags are carried,
  because those are text.
* **The author's link.** The document names the author and does not link them.
  `/user/{name}` is read off the site's own structured block on the work page and
  built from it.

### Parts that are not prose, and parts that are not published

The recorded work's first three parts are `Photo Gallery`, `Playlist` and `Cast`
— 54, 865 and 998 characters of the same `<p>` markup, one holding an `<img>`.
They are parts the reader is shown, so they are chapters here, for the same reason
Royal Road's announcement posts are: the site lists them, and an adapter that
silently dropped them would report a work as shorter than its author published it.
The image is removed by the sanitiser rather than pointed at, which leaves the
text they contain.

A part carries `draft`. An unauthenticated read only sees published parts, so this
should never be `true` — but a part the author has not published is not part of
the public work, and including one would publish it further. Drafts are excluded
and the count they remove is logged rather than absorbed.

### Not found arrives as `400`, and as HTML

The API reports a missing story as a JSON error body — `{"error_code":1017,
"error_type":"NotFound"}` — with **HTTP 400**, so the status code is not where
this is decided. The site's *page* for the same missing work is a 37 KB HTML page
with status 404 and a different message entirely (*"This page got lost in a good
story and never came back."*). Both are recorded, because the adapter must
distinguish the site's own not-found from a shape that changed: the first is
`NotFound`, the second is a parse failure, and folding either into the other
reports a deleted work as a bug or a bug as a deleted work.

## ficbook

| File | Source | Recorded |
|---|---|---|
| `work.html` | `https://ficbook.net/readfic/01899919-f575-76ed-8476-cec2348b02bf` | 2026-09-11 |
| `work-finished.html` | `https://ficbook.net/readfic/01a05297-2f11-74e0-a894-8ed36d62a32a` | 2026-09-11 |
| `chapter-1.html` | `https://ficbook.net/readfic/01899919-f575-76ed-8476-cec2348b02bf/35183469` | 2026-09-11 |
| `chapter-2.html` | `https://ficbook.net/readfic/01899919-f575-76ed-8476-cec2348b02bf/35203286` | 2026-09-11 |
| `not-found.html` | `https://ficbook.net/readfic/does-not-exist` | 2026-09-11 |
| `robots.txt` | `https://ficbook.net/robots.txt` | 2026-09-11 |

Both works are Harry Potter fandom. They were chosen for what differs between
them, not for their contents: one is unfinished and one is finished, which is
the only way the status mapping and the second shape of the size line could be
pinned down without guessing, and they are in Cyrillic, which is what the
footnote escapes and the tagged field have to survive.

### What the pages establish

* **A work page has no prose.** `#content` is on a part page only, which is what
  lets the chapter parser refuse a work page instead of returning an empty
  chapter for every part.
* **Two size-line shapes.** `планируется Макси, написано 158 страниц, 77 507
  слов, 24 части` and `24 страницы, 8 122 слова, 4 части`. The plan prefix is
  present on one and absent on the other, and the word forms differ, so the
  fields are found by their unit. The thousands separator is a **non-breaking
  space** — `77\xa0507` — which is why the digits are collected rather than the
  string parsed.
* **Status and rating are class names.** `ds-label-status-finished`,
  `ds-label-status-in-progress`, `ds-label-rating-NC-17`, `ds-label-rating-R`.
  The visible text is Russian; the classes are not language-dependent, and they
  are what the adapter reads.
* **The language field is the site's, not the work's.** `itemprop="inLanguage"`
  is `ru-Latn` on both recorded works — identical across two works in the same
  fandom but different languages of description — so it is the site's own
  interface locale written into a machine-readable field. Reported as the
  work's language it would claim every ficbook work is Russian-in-Latin-script.
  `work-finished.html` is recorded mainly to hold that comparison.
* **The part list and the "next chapter" links are the same shape.** Both are
  `a.part-link`, so the list is scoped to `ul.list-of-fanfic-parts`: collecting
  every match puts a duplicate of one part in the list, and on a one-part work
  puts the work's only chapter in twice.
* **A part page states its own address.** `<link rel="canonical">` carries the
  part id, which is how the page identifies itself; `chapter-1.html` and
  `chapter-2.html` are recorded as a pair so that a page answering with the
  wrong part can be detected rather than stored under the wrong ordinal.
* **The footnotes are not in the prose.** The references are empty placeholders
  — `<span class="footnote" id="fn_35183469_0"></span>` — and the text is in a
  `textFootnotes` object in a script at the bottom of the page, with a `\u`
  escape for every Cyrillic character. Read as text the escapes would be stored
  literally. `chapter-1.html` has nine notes and `chapter-2.html` has two, which
  is what the reference-count assertion is anchored to.
* **A missing work is a real `404`.** Unlike FanFiction.net, which answers a
  missing story with `200` and a notice, this source answers `404`, so the
  adapter needs no structural check for it. The recorded page is kept as the
  evidence that it does not.

### What the site's robots.txt says

```text
Disallow: /*?*     # every query string
Disallow: *printfic*
Disallow: *download*
Allow: /fanfiction/*?p=*
```

`/*?*` is the rule that shapes the adapter. The site writes decorative queries
on its own links — `/readfic/{uuid}?source=premium&premiumVisit=1` on listing
pages, `?from_promo=1` on the work page — and answers the same work identically
without them, so the adapter **strips the query and the fragment** rather than
asking for an address its own front door would be refused. The fragment matters
too and is easier to miss: the site's own chapter links end `#part_content`, and
a fragment is never sent to the server.

`*printfic*` and `*download*` are deliberately **not** worked around. They are
the site's own views of the same content, and reading them would be reading a
path the site has asked crawlers to leave alone rather than the page a reader
sees. `/readfic/{work}/printfic` is therefore not an address this adapter
claims.

No `Crawl-delay` is published, so the fetcher's one-second floor is the pace: a
site that asked for nothing gets nothing slower than the default.
