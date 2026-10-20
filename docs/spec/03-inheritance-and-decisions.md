# 3. Inheritance

**Status:** reserved
**Milestone:** M0
**Source:** the corresponding section of the Darkhold specification

## 3.1 From lorehaven

- **The requirement ledger as the authority** (§24.1) — same columns, same status taxonomy, same
  "re-derive, never believe" rule, plus `verified_at` and `gates`.
- **The domain crate with no I/O** (§8.3).
- **Accounts / pseuds** (§6.1) — credentials, sessions, security and trust on the account; every
  public artefact on the pseud; `pseud.account_id` is the only link and is never serialised.
- **The three orthogonal state fields on a work** — `lifecycle` × `visibility` × `completion`. A
  single `status` column makes "complete but unlisted" unrepresentable.
- **Append-only revisions** with `document_json` as the source of truth.
- **The four instance-level policies** (§6.9) — accessibility mode, body retention, media hosting,
  generated-content posture. Each set once, by the operator, at instance level, each with a
  distinct named refusal.
- **The 404-not-403 doctrine** — for inaccessible private objects *and* for operator surfaces.
- **The demand-weight primitive** (`trust × taste × contribution`, ADR 0022) with the
  majority-integrity guard and the `floor × ceiling ≤ 1.0` startup check.
- **The browse-ordering contract** (ADR 0021) — one `sort` vocabulary, and `for-you` is a
  permutation and never a filter.
- **The metadata exchange** (§15.2) — a signal is a fact about a work, never a fact about a reader.
- **The token architecture** — one source of truth, reader scale independent of site scale. The
  *values* are replaced; the architecture survives (§21).
- **Half-life** (§11.6) — how long a work keeps being read after publication, as a ranking input
  and never a public number.

## 3.2 From ficnexus

- **The adapter contract and the 107-site list** (§9.1, §9.2). The `_from_html` family is the
  single most important idea in that codebase: it enables Wayback ingest, cookie ingest,
  browser-extension capture, paste-HTML, and fixture-based tests through one API.
- **The works ↔ sources split**, refined (§6.3).
- **Body full-text search** (§11.3) — "fics where X says Y", over cached bodies, with `<mark>`
  snippets.
- **`main_char_attr`** (§11.2) — the highest-scored character tag is X **and** the freeform
  attribute Y is present.
- **The strategy registry + RRF fusion** (§12.8), including the golden guarantee and shadow mode.
- **The 7+ export formats** and the OPDS catalog.
- **The `qa/api-walk.js` idea** — parse the router source so the route inventory can never drift.
- **Config-driven adapter families** — one file serving 19 or 26 sites is how 107-site parity
  became tractable.

## 3.3 From kindred

- **The memory discipline** (§8.2, §22.3) — a cap in MiB, a gate that boots the real binary
  against the real catalogue and reads `VmHWM` after each step, and `/healthz` returning 503 when
  the process is over its own cap.
- **The signal model** (§12.7) — a linear blend of named signals, each with a weight, each able to
  **skip** rather than score zero, and skips named in `meta.degraded[]`.
- **The arena** (§12.5) — Glicko-2 with draws, four pairing strategies, an explicit explore phase,
  and **arenas for every entity kind**, not just works.
- **The weight-agreement consensus** (§12.4) — the influence of a comparison scaled by how well
  the voter's taste agrees with the instance theme, with sub-threshold comparisons **dropped**,
  not clamped.
- **The snapshot exchange** (§15.3) — k-anonymity, salted pseudonyms, signed manifests, shard
  hashes, deltas with a retained base.
- **The block model** (§12.9) — a block is a *statement*, stored separately from an inferred
  weight, idempotent, explicitly not a toggle, always visible and reversible.
- **The "measure, don't estimate" rule.**

## 3.4 From the estate standard (GS-1…GS-5, adopted 2026-10-16)

All five, in the order the standard prescribes: **GS-3 consent → GS-1 guests → GS-2 the fold →
GS-4 trust → GS-5 governance.** §14 is the whole of it.

## 3.5 Decisions

Each names its reversal cost. These are the disagreements the conversion had to settle.

| # | decision | from | why | reversal cost |
|---|---|---|---|---|
| **D-1** | **No progression economy.** No XP, levels, points, reputation, or streak that withholds something. Episodic recognition (badges, weekly boards) is kept. | ficnexus (100 levels / 10 ranks / 12 XP event types) and its forum `exp` ladder | lorehaven ADR 0013 and 0022 forbid a *new unit* of standing: it compounds, demands a source for every action, and collides with trust. ficnexus documents the cost itself — 3 XP ledgers and 2 ladders on one `users` row | Low. The tables are additive. The **real** cost of getting it wrong now is permanent: a site that ships levels can never un-ship them |
| **D-2** | **One ladder, not two.** Trust is the only authority. No separate progression rank, no `role` column, no `is_admin` flag. | ficnexus (8 authority columns on `users`) | 8 parallel columns means every authorisation decision has a coin-flip reading. **One column, one ladder, one table of what it unlocks** | Low |
| **D-3** | **Community moderation is a jury, not a points economy.** Daily moderator action budget, fixed reason→delta table, randomly drawn jury for anything consequential. | ficnexus's Slashdot-derived points system (already replaced there by a trust ladder) | A points currency is a spendable influence token with the same laundering properties as XP. A jury is drawn, not bought | Medium — a jury is a new subsystem |
| **D-4** | **Search stays in SQL.** FTS5 on SQLite, native FTS + `pg_trgm` on PostgreSQL, plus **optional** pgvector. No Elasticsearch, no Vespa, no Meilisearch. | the 2026-10-17 draft, which proposed Meilisearch | lorehaven ADR 0026: one engine, one query language, one dialect-pair to keep in parity. A third engine is a third dialect to keep in parity, for a feature pgvector already provides inside the same database | Low. The `Plan` boundary is where a future engine would enter |
| **D-5** | **One process for the app, one for taste, and the split is optional.** | both lorehaven (ADR 0001) and kindred | A 124 MB in-memory CSR graph and a database pool do not belong in one RSS. But a small instance must not need two processes. The split is a *configuration* (§8.2) | Low — the domain crate is shared either way |
| **D-6** | **Guests are real accounts with an empty credential and a flag.** | concord (the reference) | A parallel guests table means either a second owner column on every authorship column or a nullable one — two sources of truth for "who did this". That cost has already been paid once in this estate | Low |
| **D-7** | **The catalogue is a read-only mirror; the taste service never writes the archive's database.** | ficnexus (taste and archive share one Postgres) | The taste service's read pattern has a completely different concurrency and memory profile from the forum's. Sharing one database means the recommender's scans throttle the forum | Low |
| **D-8** | **The instance theme is public.** The house taste is a *published editorial policy* — named works, named authors, a stated front-page identity — and it pulls every reader's recommendations. | lorehaven N-2 ("no operator taste visible, inferable or hinted at") | The lorehaven rule protects a *private* preference from becoming a public standing. It does not forbid an instance from *stating* its editorial position. A public theme is what makes the instance useful to a stranger on arrival, and it is the difference between a personal taste and an editorial identity. **What stays private is the individual reader's taste** (§12.2) | Low. The theme is data, not policy: hiding it is a config flag |
| **D-9** | **The instance permits content involving minors, age-gated and labelled.** | the 2026-10-17 draft and lorehaven §15.7 ("disallows content involving minors absolutely") | It is the owner's instance and it hosts what he reads — 12 of his 54 works carry `underage sex`, and his own writing does too. The archive's job is the *mechanism*: a mandatory age gate, first-class archive warnings, and per-reader filters that hide rather than delete. The archive never generates such content, and DMCA and abuse escalation still apply. §25 R-9 records the operational exposure this creates | Low technically; the legal exposure is a deployment decision, not a code one |
| **D-10** | **A paid tier buys compute, never rank.** The Supporter tier (§18) unlocks the expensive recommendation tier and nothing else. | the 2026-10-17 draft, which deferred monetisation entirely | The Pi 5B frame makes the expensive tier genuinely expensive. Charging for compute is a cost-recovery mechanism, not a ranking mechanism. A paid work gains no ranking advantage; a paying subscriber gains no rank, no influence, no moderation, and no better *position* in any list — only a better *algorithm* | Low |
| **D-11** | **The theme is chosen from works, authors and seed tags — and the works are the strongest signal.** The tag vector is *derived* from all three, weighted **works ≫ authors > tags**. The theme also carries a derived set of **opposing tags**, and a work carrying one is shown less. | the 2026-10-17 draft, which proposed choosing by tag; the owner's ruling of 2026-10-19 | The owner: *"I prefer to choose my favorite fics/authors over tags to choose the instance theme, the content the algorithm pulls users towards."* — and then: *"the theme shouldn't be exclusively seed works and authors, seed tags and concurrent tags should also have some weight and works with opposing tags should be shown less. it just shouldn't be as important because works indicate theme better than authors or tags."* **A work is the most direct statement of what an instance is about**; an author is a statement about a body of work; a tag is a statement about a *category*, and a category is the weakest of the three because it is the most widely shared. The opposition is the same idea read negatively: a tag that marks a *different* register is evidence against | Low — two more tables and one derived set |
| **D-14** | **Import every work that has metadata, and measure the recommender against a held-out set instead of curating a subset.** | the 2026-10-19 draft and v1 of this note, which proposed "the 1k works you think I would enjoy most" with no criterion | The owner's ruling: *"import all works for wich we have metadata rather than 1k without any criterion."* A hand-picked 1,000 discards 110,686 works the recommender could have used, and recommender quality is a function of the corpus it indexes, not of the curation applied to it. The 1,000 becomes the **evaluation set**: the owner's 41 bookmarked works are held out of the profile and the ranking is scored against them (§9.8.2) | Low — it is a larger import and a smaller wish |
| **D-13** | **A theme's tag weights are voted; its membership and publication are not.** The community re-weights inside a cluster; the owner decides what is in it and whether it is published. | the 2026-10-19 draft, which had no community governance over taste at all | A self-governed instance whose six themes are frozen configuration is a contradiction the community notices. The owner's structural power is the seed floor (20 %) and the publication switch; the community's is the numbers. Neither can take the other's (§12.12.1) | Low — it is four tables and six endpoints, and it is reversible by disabling voting |
| **D-12** | **Moods are user-named and learned online. No shipped model, no pretrained weights.** | ficnexus's `mood` config preset (`adventurous|comfort|intellectual|emotional`) and the 2026-10-17 draft | The owner: *"moods to be named by me in app … trained as I use the site, not pretrained."* A preset list is a product decision pretending to be a feature. A mood is a named vector the reader grows from their own in-mood behaviour | Low |

| **D-17** | **An author is an identity, not a name string.** The instance groups an author's names across sources and across their pseuds into one author identity, and one author page lists every work by that author from every source. **The author may opt out.** | the owner's ruling, 2026-10-19: *"Same way same work from different sources are clustered into a single one in my site the same should happen to authors, so knowing a single author profile should allow you to find all his/her works, this should also be the case for the different pseuds unless author opts-out"* | **A name string is not a person.** The same author is `-Roulette-` on one site and `Roulette_CV` on another, and is three pseuds on one account; the work identity model (T-M10-01) already solves the same problem for works, and an author without the same treatment is the one gap in a catalogue that claims cross-source identity | Low — four tables and a curator queue, and the raw names are never overwritten, so it back-fills over the existing corpus |
| **D-18** | **An instance-level, published, versioned list of de-prioritised tags.** The default is a **weight** of zero, which demotes the tag everywhere it would be promoted and removes nothing. A **block** is available as an explicit operator opt-in, and is published so the sort contract can exempt exactly that set. | the owner's ruling, 2026-10-19: *"tags I don't want to see as much … the feature is either blocking those tags instance wide or reducing visibility with a weight"* | The list is a **taste** statement, not a content policy — the owner's own measured taste is 39 f/m of 54 works, and the list is the same statement made about the tags that do not fit it. Making it **published** (D-8's logic) keeps it an editorial position rather than a private preference, and making the default a **weight** rather than a block keeps N-7 and N-16 intact | Low — one table, one config key, and the applications are three call sites |
| **D-19** | **The corpus expansion is a workflow, not a script.** Seeds → frontier → expansion → metadata extraction → catalogue, with a checkpoint, a budget, a dry run that prints exact request counts, and a determinism guarantee. | the owner's ruling, 2026-10-19: *"doing bfs on this author's works and bookmarks … should be a deterministic workflow and generally useful not just for this usecase, for example selecting which databases to expand, how much and such"* | The BFS, the collectors, the crawl progress with cursors, the parse cache and the offline `plan` **already exist** in `~/code-local/ao3/baseline` — 2,140 tests, 25 working verbs, and a `plan` command that answers "which seeds, how many requests, how long" without making a request. What is missing is not machinery but a **contract**: what the workflow must guarantee to be reusable by something other than one author's bookmarks | Low — the machinery is not rewritten; the contract is written down and Darkhold's side of it is built |

## 3.6 Things that are deliberately dropped

- ficnexus's **100-level progression, ability tree, 26 rank-gated features, daily quests, login
  streaks, badge definitions, and its weekly/monthly/category leaderboards** — the episodic half
  (badges, weekly boards) is kept, the cumulative half is dropped (D-1).
- ficnexus's **`reputation` and bounty currency** — dropped with D-1 and D-3.
- ficnexus's **preset mood list** — dropped with D-12.
- ficnexus's **Ollama dependency for the core path** — the archive's core reading, writing,
  importing and community MUST work with no model runtime installed.
- lorehaven's **economy layer** (credits, payouts, Pool A/B redistribution) — replaced by the much
  smaller Supporter tier (§18).
- kindred's **`owner_taste` as a *public* leaderboard notion** — the instance theme is public, but
  the leaderboard says which notion produced each ranking, and anti-recommendations and
  hidden-gems stay unweighted, because weighting them by the house theme makes the page fight
  itself.
- **Every project's "we might add this later" list.** The non-goals are closed (§4).

---
