# M6-02, the fifth adapter: XenForo

Status: **reconnaissance complete, fixtures recorded, adapter not written.**
Everything below was measured on 2026-09-11 against the live sites through the
guarded fetcher and the Byparr solver, and the pages it was measured from are
committed under `crates/scrapers/tests/fixtures/xenforo/`.

This file exists because the other four tier-1 adapters carried forward from
M6-02 — ffnet, wattpad, ficbook, scribblehub — are written, tested and
committed, and this one is the last thing standing between M6 and its tag.

## What a XenForo source is

Not one site. A XenForo forum is software, and the fanfiction it holds is
*threads*: a forum thread whose posts are the chapters, with the ordered chapter
list supplied by the **SV Threadmarks** add-on. The software is shared; the
walls, the `robots.txt` and the policies are not.

The three candidate hosts, and they are three separate sources by this codebase's
own rule (*a wall is a property of one host, not of the software a host runs* —
`crates/scrapers/src/safety.rs`, `Wall`):

| Host | `robots.txt` | Plain | Fingerprint | Solver |
|---|---|---|---|---|
| `forums.spacebattles.com` | 200, 3178 B | refused | refused | 1.5 MB |
| `forums.sufficientvelocity.com` | 200, 3178 B — **byte-identical** | refused | refused | — |
| `forum.questionablequesting.com` | **404, no file** | — | — | — |

SpaceBattles and SufficientVelocity are run by the same operator and publish the
same file; QuestionableQuesting publishes none at all, which by `robots.txt`
semantics means unrestricted, and by this fetcher's behaviour means the default
pace.

### What the two files say

```text
User-agent: *
Content-Signal: search=yes,ai-train=no,use=reference
Allow: /

User-agent: Amazonbot
Disallow: /
… ninety more lines, every one of them a named crawler …
```

`Allow: /` for `*`, then a long tail of AI and SEO crawlers disallowed **by
name** — GPTBot, ClaudeBot, anthropic-ai, CCBot, Bytespider, PerplexityBot,
AhrefsBot and the rest. This is the one place in this project where the
fetcher's own product token is the load-bearing detail: `RobotsRules::parse` is
called with `product_token(&self.policy.user_agent)`, and the claim this adapter
rests on is that **our token is not on that list**. A future change to the
default user agent that made it collide with one of those names would turn every
XenForo import into a refusal, and it would look like a wall rather than a
mistake.

No `Crawl-delay` is published on either host, so the fetcher's one-second floor
is the pace.

## The three documents an import needs

### 1. The threadmark list — the chapter list

`GET /threads/{slug}.{id}/threadmarks` (the add-on's route; the thread page
itself links it).

```html
<div class="threadmarkListingHeader"> …  <dt>Created</dt><dd><time datetime="2013-06-24T23:27:50-0400">…
                                          <dt>Status</dt><dd>Ongoing</dd>
                                          <dt>Watchers</dt> …

<div class="structItem structItem--threadmark js-inlineModContainer "
     data-likes="59" data-content-author="master arminas" data-content-date="1372130903">
  <div class="structItem-cell structItem-cell--main">
    <div class="structItem-title threadmark_depth0">
      <ul class="listInline listInline--bullet">
        <li><a href="/threads/{slug}.{id}/#post-11149727">Prologue; September 27, 2596</a></li>
      </ul>
    </div>
  </div>
  <div class="structItem-cell structItem-cell--meta" title="Likes: 59">
    <dl class="pairs pairs--justified"><dt>Words</dt><dd>1.7k</dd></dl>
  </div>
  <div class="structItem-cell structItem-cell--latest">
    <a href="/threads/{slug}.{id}/#post-11149727"><time class="structItem-latestDate u-dt"
       datetime="2013-06-24T23:28:23-0400" data-timestamp="1372130903">Jun 24, 2013</time></a>
  </div>
</div>
```

What this gives, and what it costs:

* **Chapter key** — the post id in the anchor's fragment (`post-11149727`). This
  is the right key: it is stable, it is what the site's own permalinks use, and
  it is what the `#post-{id}` fragment on every page resolves to.
* **Ordinal** — the item's position in the list. The list is in reading order.
  This is the same conclusion as Scribble Hub's `number`: the site's own
  numbering is not what a reader's progress is mapped onto.
* **Title** — the anchor text, which is the threadmark label the author wrote.
* **A date** — `time.u-dt[datetime]`, a real offset-bearing timestamp. The
  recorded work writes `-0400`, so it is converted rather than assumed UTC.
* **A word count** — but **abbreviated**: `1.7k`, not `1700`. It cannot be
  summed into a work's `word_count`, and the fixture suite should assert that it
  is *not* read as a number. `SourceWork::word_count` for this source is
  therefore either absent or summed from the posts actually fetched.
* **A header** — `Created`, `Status` (`Ongoing` / presumably `Completed`) and
  `Watchers`. The status is a **visible word**, unlike Scribble Hub's machine
  string, and has not yet been recorded in more than one value.

**Pagination is real and must be handled**: the recorded list holds 26 items and
links `?per_page=25&page=2`. `per_page` is a query parameter the site honours, so
the list can be read in one request per 25 chapters — but the *chapter count* is
not stated anywhere in the header, so there is **no cross-check** of the kind
that made Scribble Hub safe. The adapter's completeness check has to come from
somewhere else, and the honest options are: page until a page comes back short
(the add-on's own end condition), and record that the count is not checkable
against a stated total.

### 2. The thread page — the posts

`GET /threads/{slug}.{id}/` (and `/page-N`), or
`GET /threads/{slug}.{id}/post-{postid}` which **serves the whole thread page**
with that post anchored.

```html
<article class="message message--post hasThreadmark threadmark-category-1 js-post"
         data-author="master arminas" data-content="post-11149727"
         id="js-post-11149727"
         itemid="https://forums.spacebattles.com/threads/{slug}.{id}/#post-11149727">
  <meta itemprop="name" content="Post #2">
  <span class="u-anchorTarget" id="post-11149727"></span>
  <div class="message-inner"> … <div class="bbWrapper"> … the prose … </div> …
</article>
```

* **The prose** is `.bbWrapper` inside `article[data-content="post-{id}"]`. The
  page carries 26 of them, so the parse is per-post by id, not "the first
  article".
* **`itemid` is the post's canonical URL**, which is the self-identification the
  chapter parser needs — the same role `rel=canonical` plays for ficbook and
  Scribble Hub. A page that does not carry the post asked for must be refused
  rather than stored under the wrong ordinal.
* **A post page is 1.8 MB** because it is the whole thread page, sponsors and
  scripts included. Fetching 26 chapters therefore costs 26 × 1.8 MB of traffic
  for perhaps 1% of it being prose. This is the adapter's real cost, it is not
  avoidable (the add-on has no per-post endpoint that returns just the post), and
  it should be said in the module documentation rather than discovered by an
  operator watching their bandwidth. `FetchPolicy::max_bytes` is the guard.
* **`data-author`** is on the article, so a work's author is readable from the
  post even though the threadmark list does not name one.

### 3. The thread — for work-level metadata

The thread page's `<title>` is `By The Horns (Story only Thread) | SpaceBattles`,
and the threadmark list header carries `Created` and `Status`. Everything else a
`SourceWork` wants — summary, tags — is on the thread's *first* post or in the
forum listing, and **has not been recorded yet**. The first post of a story
thread is usually the author's index/description post, which is a reasonable
source for a summary and is what `thread.html` holds.

## What is still to do

1. Record a threadmark list **and** a thread page for a second work, so the
   status mapping and the pagination end-condition have more than one shape
   behind them — the lesson of ficbook's two size lines.
2. Decide the work key. The thread id is the natural key, and the slug is
   cosmetic and changes when a thread is renamed; the address should therefore be
   normalised to `/threads/{id}/` where the site accepts it, or the slug rebuilt
   from the current one.
3. Write the adapter: `Wall::Solver` for SpaceBattles, and confirm
   SufficientVelocity and QuestionableQuesting separately rather than inheriting
   SpaceBattles' answer.
4. Register each host as its own source, with its own `robots.txt` facts — the
   named-crawler list above is the reason.
5. Fixture tests over the five recorded files, plus the abbreviated-word-count
   assertion.
6. A live test through the solver, asserting the plain client is refused (which
   is what makes the solver half meaningful — the pattern the other four use).

## Fixtures recorded

| File | Source | Recorded |
|---|---|---|
| `thread.html` | `forums.spacebattles.com/threads/by-the-horns-story-only-thread.262832/` | 2026-09-11 |
| `threadmarks.html` | the same thread's `/threadmarks` | 2026-09-11 |
| `post-11149727.html` | the same thread's `/post-11149727` | 2026-09-11 |
| `robots-spacebattles.txt` | `forums.spacebattles.com/robots.txt` | 2026-09-11 |
| `robots-sufficientvelocity.txt` | `forums.sufficientvelocity.com/robots.txt` | 2026-09-11 |

The three HTML files were recorded through the solver; the two `robots.txt`
files plainly, because neither host challenges them.
