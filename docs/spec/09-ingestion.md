# 9. Ingestion

**Status:** reserved
**Milestone:** M8
**Source:** the corresponding section of the Darkhold specification

## 9.1 What ingestion is for

To bring a work in, read it, and take it out again — with the raw source bytes retained and the
canonical record curated separately. **The archive is a library, not a proxy.** A source that
becomes unreachable leaves a work in the archive, not a broken link in a feed.

## 9.2 The 107-site list

ficnexus's FanFicFare-parity list is the most expensive asset in the estate. It is **adopted as a
work list**, not as a claim of current parity — a ported adapter is a ported adapter, and each one
must be re-verified against the live site before the archive claims it works.

| family | count | shape |
|---|---:|---|
| Major archives | ~30 | AO3 + 4 OTW siblings, FFN, FictionPress, RoyalRoad, ScribbleHub, FimFiction, Literotica, Wattpad, DeviantArt, FicBook, Quotev, AsianFanFics, Syosetu, Kakuyomu, MediaMiner, ChiReads, PMD, SpiritFanfiction, Fanfictions.fr, Twisting the Hellmouth |
| Forum / XenForo | 8 | SpaceBattles, SufficientVelocity, QuestionableQuesting, theforce.net, fiction.live, AlternateHistory, Althistory, The Sietch |
| eFiction | 19 | the classic family |
| eFiction-variant (`viewstory.php?sid=`) | 26 | psychfic, wolverineandrogue, sycophanthex, walkingtheplank, themasque, ksarchive, twilighted, whofic … |
| StoriesOnline | 3 | storiesonline, scifistories, storyroom |
| WordPress-novel | 1+ | novelfull family |
| Adult-gated | ~14 | fanfics.me, fictionhunt, inkbunny, sofurry, dokuga, asexstories, aneroticstory, mcstories, hentaifoundry, bdsmlibrary, readonlymind, utopiastories |
| Bespoke | ~15 | HPFanficArchive, adultfanfiction.org, LCFanFic, FireflyFans, FicWad, FictionMania, FanFiktion.de, TouchFluffyTail, FanFicAuthors, PhoenixSong, StoriesOfArda, Fictionalley, NovelAll, MassEffect2.in … |

**Parity is a property of the adapter, not of the crate.** FanFicFare v4.60.0 is the baseline.

**Reachability is a fact, not an assumption.** From the estate's build host, AO3 returns 404 plus
a bot challenge and FFN returns 403 Cloudflare. The archive's answer is an honest one: the adapter
is present, the site is unreachable from this host, the site profile says so, and the mitigations
(user-supplied cookie ingest, `cf_clearance`, an egress proxy) are a documented workaround.

**And the mitigation for the launch corpus is already in hand (§9.8).**

## 9.3 Credentials and the adult gate

- **Operator credentials** — environment, per site, never in the config file.
- **Reader-supplied credentials** — opt-in, explicit consent, **AES-256-GCM encrypted at rest**,
  30-day expiry, passwords never returned by any endpoint, and the download path uses the
  *requesting reader's* credentials, never the operator's.
- **The adult gate** — a separate flag from the credential. A site marked adult-gated requires both
  a login and the adult confirmation.
- **SSRF is not an SSRF exception.** Credentials are scoped to one site; a credential is never a
  capability to fetch an arbitrary URL.

## 9.4 Body retention

| policy | behaviour | the reader sees |
|---|---|---|
| `cache` (default) | the body is stored, indexed, exportable, searchable by quote | full text, full-text search, every export format |
| `aggregate` | metadata only; the body is a link to the source | a link, no full-text search, a reduced export set |

A reader must be able to see, on the work page, **which policy is in force** and what it costs
them. A body that expires is *not* silently deleted: the work survives, the body is marked
unavailable, and the reader is told where the source is.

## 9.5 Deduplication and the identity model

1. **Exact** — the same `source_site` + `source_key` imported twice. A no-op, enforced by a unique
   index.
2. **Content hash** — identical normalised title + author + word count + chapter count. An exact
   duplicate within a site: merged silently.
3. **Identity proposal** — normalised title + normalised author match, word count within 5 %.
   Auto-merged above a high confidence threshold; below it, a **proposal** a curator votes on.
   Never merged on title alone. Never merged on author alone.

**The merge is reversible.** A merge records what it merged and why; an unmerge is a split proposal.

**Cross-source identity is a first-class concept**, not a column.

## 9.6 Import, library, work

Three distinct things, and conflating them is the classic archive bug:

- **library_item** — a copy in *a reader's* library. Private. Deletable without affecting anyone.
- **work_source** — the archive's record of a source copy, in the public catalogue.
- **work** — the canonical record.

A reader imports a URL → a `library_item` is created for them → the work enters the catalogue →
the work page is shared by everyone with the same work, from any source. Importing a work you
already have adds a *source*, not a second work.

## 9.7 Scrape health and self-healing

- Every failure is classified: `transient` · `blocked` · `structural` · `systemic`, with a stable
  fingerprint over `domain|kind|normalised-message`.
- Debounce: ≥3 same-domain structural failures in 15 minutes → one healing event; one heal per
  fingerprint per 24 hours.
- A **diagnose-only** agent is permitted by default; the autonomy level is configuration and its
  default is off.
- Every scrape failure is a **user-visible** state on the work page.

## 9.8 The launch corpus — the offline path

The owner requires: *"all works already on disk with more metadata than just random, author,
should have all metadata like tags, fandom, summary, etc"* and, on the fallback, *"find the 1k
works you think I would enjoy most based on my preferences and submit them to the site with all
the metadata."*

**The owner's ruling (2026-10-19): import every work for which metadata exists.** The 1,000-work
selection is not an import scope; it becomes the **evaluation set** (below). The reason is
obvious once measured — a hand-picked 1,000 thrown away 111,000 works that the recommender could
have used, and the recommender's quality is a function of the corpus it indexes, not of the
curation applied to it.

There are two corpora and they are handled differently, because **the AO3 block does not apply to
files that are already local**.

| corpus | on disk | metadata available offline | plan |
|---|---:|---|---|
| **AO3 mirror** | 1.7 GB SQLite, 112,935 works | **yes** — title, author, summary, rating, word count, kudos, bookmarks, complete, update date, and the full tag set per work | import directly. **No request to AO3 is needed**, which sidesteps the bot challenge entirely |
| **Questionable Questing** | 10,266 EPUBs, 1.5 GB | **no** — the sampled OPFs carry title, creator and description, and **no tags** | enrich from the source thread. `archive/qq/creative-writing/links.txt` (232 KB) and `questing/links.txt` (44 KB) hold the thread URLs in order, so each EPUB has a known thread. QQ is reachable from this host. A QQ adapter parses the thread's tag list into `work_tags` |
| **FicHub / FicNexus exports** | in the 44,701-file archive | partial — the filenames carry `{id} {title} - {author}` | the AO3 id resolves into the mirror for metadata where the id is an AO3 id; otherwise fetch from the source |
| **V for Vendetta** | 10 PDFs | no | out of scope for v1; a work with no tags is still importable as an original |
| **the owner's own 13 works** | local EPUBs and `.txt` | the mirror has the full tag set for all 13 | imported as the first **written** works, with `owner_pseud_id` set |

### 9.8.1 The import criterion — stated, and measured

A work is imported **iff it has a title, an author string, a summary, and at least one tag
assertion**. Measured against the mirror, that is:

| criterion | works | % of the mirror |
|---|---:|---:|
| everything in the mirror | 112,935 | 100 % |
| title + author + summary | 112,134 | 99.3 % |
| **+ at least one tag assertion — THE IMPORT SET** | **111,686** | **98.9 %** |
| …restricted to Explicit or Mature | 67,311 | 59.6 % |
| …restricted to Explicit or Mature and complete | 34,535 | 30.6 % |

**The rating band is a reader-side filter, not an import rule.** The instance's content policy
(D-9) is permissive and the archive does not decide at import time what a reader may not see
(N-16, §10.2). The rating band becomes the **default filter for anonymous visitors** and a
one-click filter for everyone else. The archive stores what the source published and labels it.

**Two findings that change the design, both measured:**

1. **The fandom namespace is unusable as a filter.** Only **51 of 112,935** works carry a
   `fandoms` tag; 44 carry `characters`; 39 carry `relationships`. The freeform namespace carries
   **3,890,504 rows over 112,481 works** — the freeform tags *are* the taxonomy, and the filed
   namespaces are nearly empty. This is the same defect the AO3 recommender already documented:
   *"AO3 files Character and Relationship tags on only 44 and 39 works of 113,995. Judged on that
   namespace alone, zero entities clear a 60-work floor."* So the fandom is **inferred from the
   freeform name's shape** (kindred's `LooksLikeFandom`), never from the `fandoms` column — a
   `tag_type` cap would exclude nothing while reporting success.
2. **97 % of the freeform tags are singletons.** 634,190 distinct freeform names; **587,258
   (97 %) are on exactly one work**. Only 56,197 clear a 5-work floor, 30,694 a 10-work floor,
   16,975 a 20-work floor. A taxonomy of 634,231 browsable nodes is not a taxonomy, it is a
   list of one-off user phrases. So **canonical entity creation is gated on a work-count floor**
   (§6.4), and everything below the floor stays a raw assertion — browsable, searchable, and
   countable, but not a node in the tree.

### 9.8.2 The evaluation set replaces the 1,000-work selection

The 1,000 works are no longer an import scope. They become the **relevance fixture**, and the
criterion is a measurement rather than an opinion:

- **Held out:** the owner's **41 bookmarked works**, excluded from the profile used to rank.
  All 41 are in the mirror, all 41 have a summary, all 41 have ≥1 tag, 39 have ≥1 freeform tag.
- **Measured:** precision@k of the ranking against those 41, plus the per-fandom cap, which is the
  number that actually matters — the engine's documented failure is fandom monocrop: *"offline
  output is fandom-monocropped (26/30 Harry Potter) even with `--max-per-fandom`, because the
  offline cap is a known no-op and the seed set is HP-dominated."*
- **Tooling:** `ao3_recommender/evaluation/` already has the harness and the metrics
  (precision, recall, NDCG). The same fixture becomes the **golden corpus** the Rust engine
  (§12.8) is asserted against in M13, which is how the ordering problem between M9 and M13
  becomes a test rather than a contradiction.

### 9.8.3 The import path

The imported works are submitted to the instance as **metadata exchange signals** — the spec's
§15.2 mechanism, using the existing tooling
(`~/code-local/ao3/baseline/build_lorehaven_signals.py`, which reads the contract from
`lore-metadata` rather than assuming it, and asserts its payload against the allowed field set).
This is the first end-to-end exercise of the exchange in the estate, and it needs no AO3 request.

The import is a **bulk job**, not a per-work request: 111,686 works, 634,231 tags and
3,891,300 tag assertions, in batches of ≤ 200 signals (§15.2's `MAX_BATCH_SIGNALS`). It is
idempotent by content hash, so a re-import costs a submitter nothing and creates no second
record.

**Bodies are a separate matter, and the distinction is a first-class badge.** The import brings
**metadata**, not text. The owner's 54 works and the 10,266 QQ EPUBs have bodies; the other
~111,000 have a source URL and a summary. The instance is therefore a **catalogue with a library in
it**, not a mirror.

Every work therefore carries a **provenance badge**, and it is visible everywhere a work is listed
— not only on the work page:

| badge | meaning | what the reader gets |
|---|---|---|
| **hosted** | the instance holds the body | full text, full-text search, every export format, offline |
| **referenced** | the instance holds the metadata and a source link | summary, tags, the source, and a reduced export set |
| **known** | the instance holds only a signal (from the exchange) | title, author, tags, and a source link; no summary |

**The rules:**

1. **The badge is in the list, not just the page.** A browse list that mixes hosted and referenced
   works without saying so is a list of lies about what the reader can actually read.
2. **The recommendation's reason string names it** — *"matches Gamer / System · referenced"* —
   so a reader is never surprised by a recommendation they cannot read.
3. **The badge follows the body-retention policy** (§15.7's `cache` / `aggregate`), and a policy
   change re-derives the badges.
4. **A reader's own library is always `hosted`**, whatever the policy, because the reader put it
   there.

## 9.9 Legal posture

- Robots posture is resolved per site and recorded; the default posture is honoured.
- Rate posture is per site and configurable; the default is conservative.
- AI-training crawlers are disallowed by default via a **generated** `robots.txt`.
- The archive's own `robots.txt` disallows bulk body access, and there are no bulk body endpoints
  to disallow (N-3).
- A DMCA notice removes the material and its local mirrors on request, and does not delete the
  `MediaReference`.
- An author's `ai_training` declaration is shown as a statement and is exported.

## 9.10 The expansion workflow (D-19)

**The corpus grows by a workflow, not by a script.** The owner's ruling, 2026-10-19: *"doing bfs
on this author's works and bookmarks … should be a deterministic workflow and generally useful not
just for this usecase, for example selecting which databases to expand, how much and such."*

**Where the machinery already is.** The BFS (`recommenders/bfs.py` — `_run_seed_bfs`,
`collect_from_user_bookmarks`, `parse_bookmarkers_from_work`), the collectors
(`scrapers/collectors.py`), the work parser, the crawl progress with **cursors and checkpoints**
(`database/crawl_progress.py`), the parse cache and the backfill (`database/backfill.py`) and the
**offline `plan`** (`planner.py`) all exist in `~/code-local/ao3/baseline` — 2,140 tests, 25
working verbs, and a `plan` command that answers *"which seeds, how many requests, how long"* with
**exact counts where they are exact** and a `not counted` section where they are not.

**So this is a contract, not a rewrite.** §8.3's rule applies: one implementation per concept.
The workflow is not reimplemented in Rust; the workflow's **guarantees** are written down, and
Darkhold's side of it is built.

**The workflow, in five stages:**

```
seeds  →  frontier  →  expansion  →  metadata extraction  →  catalogue
```

| stage | what it does | the guarantee |
|---|---|---|
| **seeds** | a named set of starting points — an author, a work, a tag, a fandom, a bookmark list | the seed set is **declared**, not discovered, and is recorded with the run |
| **frontier** | the seeds' immediate neighbours — an author's works and bookmarks, a work's authors, a tag's works | the frontier is a **queue with a total order**, so the visit order is deterministic |
| **expansion** | the frontier's neighbours, under the depth, cap and budget | **the same seed set and the same configuration visit the same works in the same order** |
| **extraction** | the metadata for each visited work is fetched, parsed and stored | extraction is **idempotent** — a re-run re-fetches nothing it already has |
| **catalogue** | the works, authors and tags land in the archive's catalogue | the catalogue write is **idempotent by content hash** (T-M9-04) |

**The five guarantees, and they are what makes it reusable:**

1. **Deterministic.** The same seeds and the same configuration produce the same visit order and
   the same result set. *"A random holdout would make two runs of the same config produce
   different metrics, and a metric that moves when nothing changed cannot be used to decide
   whether a change helped."* The same rule, and for the same reason.
2. **Configurable — which databases, and how much.** The configuration names the **sources** to
   expand (the local mirror, the live sites, the QQ archive), the **depth**, the **per-source
   caps**, the **wall-clock budget**, and the **channels** to use. The `plan` dry run prints the
   result before a single request is made.
3. **Resumable.** The crawl progress carries a cursor per container. A crash resumes; a re-run
   does not re-fetch what it has.
4. **Honest about its own cost.** The plan prints exact request counts where they are exact, an
   ETA computed from the named crawl delay, and a `not counted` section for the uncertainty it
   cannot see. *"An omitted uncertainty reads as certainty."*
5. **Rate- and posture-respecting.** The robots posture, the rate posture and the credential
   scope of §9.3–§9.4 apply to the expansion exactly as they apply to any other fetch. **A
   degraded source is a degraded source** (N-15).

**The relationship to Darkhold.** The expansion's output lands in the archive through the
**metadata exchange** (§15.2) — the same mechanism the launch corpus uses, and the same
`build_lorehaven_signals.py` builder, which **reads the contract from the `metadata` crate rather
than assuming it**. So an expansion is:

- a **bulk job** against the exchange, in batches of ≤ 200 signals;
- **idempotent by content hash**, so a re-run costs a submitter nothing and creates no second
  record;
- and **resumable**, because the exchange's batch endpoint is idempotent and the workflow's
  cursor survives the restart.

**The 24-hour run is an operational instance of the workflow, not a mode of it.** That is the
whole point of the guarantees: a run that lasts 24 hours and a run that lasts 24 minutes are the
same code with a different budget, and the one that lasts 24 hours is not a special program that
nobody can point at twice.

**One honest constraint.** From the estate's build host, **AO3 returns 404 plus a bot challenge
and FFN returns 403 Cloudflare** (R-4). The expansion's reachability against AO3 therefore
depends on a **credential or an egress proxy**, and the workflow's plan must report the
unreachable sources as unreachable rather than assuming them. The **local mirror is not
affected** — it is a file, and the AO3 block does not apply to files that are already local.

---
