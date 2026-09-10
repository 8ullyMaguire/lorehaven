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
| The adapter crate, its trait and the registry | `crates/scrapers/src/{lib,registry,sites}.rs` | 101 in-crate |
| The safe fetcher and URL guard | `crates/scrapers/src/safety.rs` | in-crate |
| Chapter sanitation | `crates/scrapers/src/sanitize.rs` | in-crate |
| The Archive-software adapter | `crates/scrapers/src/sites/ao3.rs` | 9 fixture tests |
| The Royal Road adapter | `crates/scrapers/src/sites/royalroad.rs` | 13 unit + 21 fixture tests |
| Recorded fixtures | `crates/scrapers/tests/fixtures/{ao3,royalroad}/` | provenance in `fixtures/README.md` |
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

## 3. Deliberately not built, with the reason

**Preservation batches.** Spec §14.5 and the plan's fifth pitfall both put them
in Milestone 17, behind the operator role, a documented permission basis and a
dry-run report. The destination field exists and accepts only `library`. Nothing
in M6 could be mistaken for the authorisation.

**The source revision cache.** `source_revision_cache_entries` has existed since
migration 0005 and nothing has ever written to it; M6 did not either. An import
re-reads the source's page rather than trusting a cached revision. Making the
cache real needs an adapter that declares a conditional-request capability, and
none does. Tracked as `M6-12`.

**Automatic source health.** An operator pause is honoured — a paused source
costs zero requests, which is asserted by counting them — and nothing moves a
source's health on its own. The spec §11.8 failure classes are classified in the
message a reader sees and are not written back to the catalogue. `M6-08`.

**Nine of the ten tier-1 sources.** ffnet/fictionpress, the XenForo board
family, royalroad, scribblehub, wattpad, the eFiction family, syosetu and
ficbook. The trait is shaped for them and the fixture discipline is written down
in `crates/scrapers/src/sites/mod.rs`; none is ported. `M6-02`.

**The pages.** `/import` and the imported-items list. Everything they need
exists on the API. `M6-09`.

---

## 4. What the tests found

Three defects in the boundary between Milestone 5's store and Milestone 6's first
use of it, and one parser defect. They are recorded in `docs/verification.md`
with the fix for each; the summary is that a store with no caller is a store
whose foreign keys have never been exercised, and this is where they were.

---

## 5. The port from `ficnexus`, and what it was wrong about

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

## 6. Source reachability, measured

Recorded on 2026-09-10 with a plain HTTPS request and a browser `User-Agent`,
against the URL taken from the ficnexus adapter for each source:

| Source | Result | Consequence |
|---|---|---|
| Archive of Our Own | `200` | Adapter built, fixtures recorded |
| Royal Road | `200` | Adapter built, fixtures recorded |
| Syosetu | `200` | **Reachable, adapter not built yet** |
| www.fanficauthors.net, www.lcfanfic.com, www.phoenixsong.net, www.mediaminer.org | `200` | **Reachable, adapter not built yet** (the eFiction family) |
| **www.fanfiction.net** | **`403`** | Cloudflare |
| **www.scribblehub.com** | **`403`** | Cloudflare |
| **www.fimfiction.net** | **`403`** | Cloudflare |
| **forums.spacebattles.com** | **`403`** | Cloudflare (and the XenForo board family generally) |

FictionPress runs the same software as FanFiction.net and is therefore behind the
same wall.

Two rows say *reachable* and no more. Syosetu and the eFiction family answer plain
requests, so an adapter for either can be verified and is ordinary work; it is not
done yet, and the table says so rather than implying a delay is a difficulty.

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

## 7. Before this milestone can be tagged

1. The two pages, and a browser journey over them.
2. The remaining tier-1 adapters, each with recorded fixtures.
3. An M6 entry in `docs/sessions/`.
4. Whatever the journey finds — on M4 and M5 it found something both times.
