# M6-02, the fifth adapter: XenForo

**Landed.** `crates/scrapers/src/sites/xenforo.rs` is written and tested: three
forums, three registry keys, 13 module tests and 13 fixture tests, plus a live
test over all three hosts. `v0.07-imports` is unblocked by it.

This file is kept as the design record, because two of the things it says first
were wrong in ways worth remembering. Corrections are marked.

Everything below was measured on 2026-09-11 against the live sites, and the pages
it was measured from are committed under
`crates/scrapers/tests/fixtures/xenforo/`. The module documentation in
`xenforo.rs` and the `## xenforo` section of `tests/fixtures/README.md` are the
authoritative record; this is the reconnaissance that led there.

## Two corrections to the first draft of this file

**1. "The three hosts need a solver."** Wrong for all three, and wrong because of
the probe rather than the reading. The reconnaissance sent a **browser**
`User-Agent` from a non-browser client, and Cloudflare challenged exactly that
incoherence. Re-measured with each client's own honest agent:

| Client | Result |
|---|---|
| `Lorehaven/{version} (+import)` — the fetcher's own agent | **200**, the real 128 KB page |
| `curl/8.0` | **200**, the real page |
| a Chrome `User-Agent` over plain TLS | **403**, *Just a moment* |

Two of the three hosts were also probed only through the solver, so their refusal
to a plain request was never established at all — it was inherited from
SpaceBattles, which is the exact error this codebase's `Wall` documentation warns
about. All three declare `Wall::None`.

**2. "The chapter count is not checkable against a stated total."** Wrong. The
list header carries **`Threadmarks: 42`** beside `Created`, `Status` and
`Watchers`. The first pass read the header for the first three and missed the
fourth, and then wrote a paragraph about the gap it had not actually looked for.
The recorded work arrives as 25 + 17 = 42, the adapter refuses a list that does
not add up, and that check is what makes a partial import impossible.

## What a XenForo source is

Not one site. A XenForo forum is software, and the fanfiction it holds is
*threads*: a forum thread whose posts are the chapters, with the ordered chapter
list supplied by the **SV Threadmarks** add-on. The software and the add-on are
shared across all three hosts; the `robots.txt` files and the client policies are
not.

| Host | Registry key | `robots.txt` | Plain request |
|---|---|---|---|
| `forums.spacebattles.com` | `spacebattles` | 200, 3178 B | 200, real page |
| `forums.sufficientvelocity.com` | `sufficientvelocity` | byte-identical | 200, real page |
| `forum.questionablequesting.com` | `questionablequesting` | **404, no file** | 200, real page |

### What the two published files say

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
AhrefsBot and the rest. This is the one place in this project where the fetcher's
own product token is load-bearing: `RobotsRules::parse` is called with
`product_token(&self.policy.user_agent)`, and the claim the adapter rests on is
that **our token is not on that list**. The fixture suite asserts it both ways —
our token is allowed these paths, and `GPTBot` parsed from the same recorded file
is not — so a file that allowed everything to everybody could not pass.

A future change to the default user agent that collided with one of those names
would turn every XenForo import into a refusal, and it would look like a wall
rather than a mistake.

No `Crawl-delay` is published on either host, so the fetcher's one-second floor is
the pace.

## The three documents an import needs

All three are addressed by the thread id alone — `/threads/{id}/...` resolves
directly, measured rather than assumed, which is why a stored address never goes
stale when an author renames a thread.

### 1. `/threads/{id}/threadmarks` — the chapter list

```html
<div class="threadmarkListingHeader"> …  <dt>Created</dt><dd><time datetime="2013-06-24T23:27:50-0400">…
                                          <dt>Status</dt><dd>Ongoing</dd>
                                          <dt>Watchers</dt><dd>209</dd>
                                          <dt>Threadmarks</dt><dd>42</dd> …

<div class="structItem structItem--threadmark js-inlineModContainer "
     data-likes="59" data-content-author="master arminas" data-content-date="1372130903">
  <div class="structItem-cell structItem-cell--main">
    <div class="structItem-title threadmark_depth0">
      <ul class="listInline listInline--bullet">
        <li><a href="/threads/by-the-horns-story-only-thread.262832/#post-11149727">Prologue; September 27, 2596</a></li>
      </ul>
    </div>
  </div>
  <div class="structItem-cell structItem-cell--meta" title="Likes: 59">
    <dl class="pairs pairs--justified"><dt>Words</dt><dd>1.7k</dd></dl>
  </div>
  <div class="structItem-cell structItem-cell--latest">
    <a href="/threads/by-the-horns-story-only-thread.262832/#post-11149727"><time … datetime="2013-06-24T23:28:23-0400">…</time></a>
  </div>
</div>
```

* **Chapter key** — the post id, from the anchor's `#post-{id}` fragment on
  SpaceBattles and SufficientVelocity and from the id as a **path segment** on
  QuestionableQuesting. Both are read; the fragment form alone returns zero
  chapters on the third host.
* **Ordinal** — the item's position. The list is in reading order.
* **Title** — the threadmark label the author wrote.
* **A date** — `time.u-dt[datetime]`, an offset-bearing timestamp written as
  `-0400`, which RFC 3339 does not allow.
* **A word count** — **abbreviated** (`1.7k`). It cannot be summed into a work's
  total, so `word_count` is not reported at all.
* **The total** — `Threadmarks: 42`, which is the completeness check.
* **A trap: a thread page carries a widget list** of the most recent few
  threadmarks. It parses as a valid five-item list with `Threadmarks: 42` beside
  it. Nothing reads a thread page as a chapter list, and the count check is what
  would catch it if anything ever did.
* **`per_page` is silently clamped.** `50` and `100` are honoured; `1000` returns
  **25**, the site's default, with no error. The adapter asks for 100 and checks
  the total regardless, so a clamp in either direction is caught.

### 2. `/threads/{id}/` — what the work is

`h1.p-title-value` for the title, `div.p-description a.username` for the author
and their profile link, `meta[property='og:description']` for the summary, and
`a.tagItem` for the tags — whose anchors carry a category icon whose `<title>` is
a label, so `Setting battletech` is one tag called `battletech`.

The site's own summary is used rather than the first post, because on this work
the first post is the author's index post while on many threads it is the first
chapter.

### 3. `/threads/{id}/post-{post}` — the prose

The add-on has **no per-post endpoint**: this returns the whole thread page with
the post anchored, which is ~1.8 MB for a few kilobytes of prose. The parse is
scoped to `article[data-content="post-{id}"] .bbWrapper`; the recorded page
carries 26 articles, so an unscoped selector returns the first post's prose for
every chapter.

A **post-only address** (`/posts/{id}/`, the site's "copy link to post") redirects
to the containing thread page, which states `data-content-key="thread-{id}"`. That
is what lets a pasted post link be resolved and imported.

## The status vocabulary

The site enumerates it in its forum filters: `incomplete`, `complete`, `hiatus`,
`dropped`. The header renders the **visible label**, which is not the machine
value, and the hosts do not agree on the words:

* SpaceBattles and SufficientVelocity write `Ongoing`.
* QuestionableQuesting writes `Incomplete` for the same state.

`Dropped` is the site's word for what the domain calls `Cancelled`. An unseen
label is `Unknown` rather than a guess.

## What is deliberately not read

* The per-chapter word count, because it is abbreviated.
* The first post as a summary, because it is a chapter on many threads.
* Tags as the theme renders them, because the category is inside the anchor.
* Any work-level language or content rating: the forum states neither.
