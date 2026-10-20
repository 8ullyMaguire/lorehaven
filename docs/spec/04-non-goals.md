# 4. Non-goals — the hard refusals

**Status:** reserved
**Milestone:** M0
**Source:** the corresponding section of the Darkhold specification

**Refusals, not deferrals.** Each names the failure it prevents.

| # | refusal | why |
|---|---|---|
| **N-1** | **No purchased trust, ranking, moderation authority, or position.** Credits never buy a level, a slot, a boost, or a place in a queue. A Supporter subscription buys *compute* and nothing else | lorehaven §0.3, ADR 0022; D-10 |
| **N-2** | **No individual reader's taste is visible, inferable, or hinted at** — no label, no multiplier name, no credit breakdown, no leaderboard that reveals another reader's preferences | the *instance theme* is public by design (D-8); a **person's** taste is not, and the difference is the whole point of the distinction |
| **N-3** | **No bulk text endpoints.** No route returns the full body of many works at once | ADR 0018. AI-training crawlers disallowed by default via a *generated* `robots.txt` |
| **N-4** | **No automatic publication of AI output.** Generated content is published only under a declared instance posture, and the author's declaration is shown as a *statement*, never as "protected" | ADR 0018. **The archive indexes and recommends AI-generated works** (8 of the owner's 54 carry `ai-generated text`; he writes them), labelled as a declared attribute and filterable — never hidden from him |
| **N-5** | **No multi-tenancy.** One instance per deployment | a shared instance is a shared moderation queue and a shared ban list |
| **N-6** | **No reading history, ratings, notes, library membership, credentials, session identifiers, IP addresses or file paths in any outbound signal** | the exchange schema has no such fields, so no configuration can widen what leaves the instance (§15.2) |
| **N-7** | **No ranking that removes.** `for-you` is a permutation; the reachable set is identical under every sort and under every mood | ADR 0021. A taste-skewed default that removed results would be a shadowban applied by ranking instead of by moderation |
| **N-8** | **No "the second engine is optional".** SQLite and PostgreSQL are both first-class and both gated on every schema change | ADR 0004: "the second engine is not only a deployment target, it is the cheapest available check on the first one" |
| **N-9** | **No unreviewed bulk scrape of a source site.** Every adapter has a rate posture, a robots posture, and a per-source credential path | a source that blocks the operator's host is blocked, and the honest answer is "not reachable from here" |
| **N-10** | **No re-hosting third-party media.** The instance stores metadata and a reference | lorehaven §30.2 |
| **N-11** | **No "a 500 is not an empty state".** A not-enough-data condition has a named answer | lorehaven's open defect `/arena` → 500 for every new account |
| **N-12** | **No unmoderated guest commenting on a work's text.** Guests get the functionality; a specific surface can require an account | the estate standard's firewalls are per-site, and the default is a quarantine |
| **N-13** | **No anonymous read of a restricted work.** The 404-not-403 doctrine applies to every surface | |
| **N-14** | **No feature that exists only in a document.** A requirement is done when it has code that runs, tests that drive the real path, and evidence recorded in the ledger | lorehaven `docs/goal.md` |
| **N-15** | **No scraper that treats a failed fetch as a link.** A degraded source is a degraded source | "a caching instance that downgrades a failed fetch into a link" is a non-goal |
| **N-16** | **No mood, theme or channel that filters.** Every one of them reweights; none of them can make a work unreachable | the same doctrine as N-7, applied to the taste layer |

---
