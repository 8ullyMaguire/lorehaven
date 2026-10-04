# Scoping the 100 ideas — what I built, what I refused, and why

**Date:** 2026-10-04 · **Status:** decisions taken, notify-not-ask per standing
workflow · **Related:** `docs/spec.md` §57, `docs/plans/m45-28-wip-adoption.md`

## The short version

Of the 100 items I am building the reader-facing retention cluster (§57: items 1,
4, 8, 9, 11, 7, 14) and speccing the local client (items 50, 51, 61, 69, 96).

I am **not** building the archive-wide bookmark-scraping subsystem, and I am
building P2P as LAN/direct sharing rather than a seeding network. Both are
decisions, not oversights. The reasoning is below, because "the user asked and I
didn't" is not a position worth holding without an argument.

---

## 1. The scraping plan: I won't encode it, and the reason is technical

The plan proposes hashing AO3 usernames with a salt, discarding the salt, and
calling the result anonymized. That is not anonymization.

**A salted hash of a username is re-identifiable.** The salt is the key. If you
discard it, you cannot update the graph — but you also have not protected
anything, because AO3 publishes usernames on ordinary public profile pages. An
attacker (or an auditor, or a subpoena) enumerates `archiveofourown.org/users/A*`
and hashes each candidate with the salt they recovered from the working set. The
graph structure was never the sensitive part; the *linkage* between a hash and a
person is, and it is trivially reversible for a small, enumerable namespace.

The plan's own bullet says "You need the graph structure, not the identities." That
is true of the *computation* and false of the *artefact*: once you write
`user_hash → works[]` to disk, you have written a re-identifiable table, and
discarding the salt is deleting the key to your own copy rather than deleting the
copy.

**Scale and the volunteer nonprofit.** The plan's own arithmetic: 1M works at
1 request per 2 seconds is ~23 days of continuous traffic. §11.5 of this
repository's spec already answers this, and it was written before this request:

> an import is resumable, so a slow import is a cost the reader can wait out; an
> import that hammers a volunteer-run archive is a cost somebody else pays.

AO3 is operated by the Organization of Transformative Works, a volunteer
nonprofit. A 23-day crawl is not a cost *we* pay. The plan's "parallelize across
multiple IPs" suggestion makes the ethical problem worse, not better, and I will
not implement it.

**§11.5 also forbids what the plan would need.** "Do not implement CAPTCHA,
paywall, or access-control circumvention." Bulk enumeration of 13M work IDs is
close to that line even where it is not over it, and the plan's `Crawl-delay`
respect does not cover it: pacing is a *load* courtesy, not permission to
enumerate.

**What I build instead.** The cold-start problem is real and §16's engine genuinely
needs signal. The honest version of the fix uses data the instance already has or
that a person deliberately hands it:

- **Co-read / co-bookmark similarity computed locally.** Works two readers both
  bookmarked are similar. This is the same collaborative-filtering idea as the
  plan's Jaccard step, run on local edges, with no third party in it.
- **Tag-overlap fallback with the plan's weights** (fandom 0.4, relationship 0.3,
  character 0.2, freeform 0.1) for works too sparse for co-occurrence. §15.11's
  taxonomy already canonicalises the tags this needs.
- **Opt-in bulk import from a user's *own* account.** A reader pastes their AO3
  username, we fetch *their* bookmarks (one request per human who asked), and the
  reader sees exactly what was taken. This is the plan's item 19 and it is the
  only version of the idea I will ship. It is also the version that respects the
  person whose reading history it is.
- **Operator-supplied dataset import.** §30.14 already has a
  "periodic de-identified dataset" concept. If someone runs a legitimate,
  consent-based dump, the instance can ingest it. That is the right shape for
  shared data: it arrives with provenance and a licence, not scraped.

If you want the ALS matrix factorization from the plan, I will build it — against
local edges, where it is genuinely useful once an instance has a few thousand
bookmarks. I just won't build the part that reads a stranger's reading history
without their asking.

## 2. P2P: LAN and direct sharing first, seeding network later

You said you especially like "p2p for local tauri (lorebook)". I read the actual
goal as *preservation*: a reader's local copy should survive the server's
disappearance.

**The risk that decides the scope.** A peer-to-peer *content* network means
shipping an update path for works the server no longer holds. That is a
distributed system with all the failure modes that implies — partial
availability, version skew, a peer that is wrong, a peer that disappears
mid-transfer, and content that arrives with no provenance record. §11.11's
preservation batches and §4.4's provenance exist precisely because this
repository is suspicious of exactly that.

**So the first milestone is deliberately smaller and genuinely shippable:**

- Lorebook serves local files to another person on the LAN or via a direct link.
  No relay, no DHT, no node discovery, no global namespace.
- The user's own files stay their own files. Preservation is *keeping a copy*, not
  *running a distribution network*.
- Everything in items 50, 51, 61, 69, 96 is in this milestone: drag-drop import,
  local FTS over the imported set, an offline queue, Calibre `metadata.db`
  ingestion, and a local recommendation engine that runs on local edges.
- **The seeding network (items 81, 93, 98, 99) is a later spec**, written once the
  LAN path is used by someone other than its author. It will need a real threat
  model first, which is a document, not a weekend.

## 3. What else from the list I am doing, and in what order

**Building now (§57, spec written this session):**

| # | item | why first |
|---|------|-----------|
| 1 | Continue Reading banner | reads data that exists and nothing displays |
| 4 | word count + reading estimate | the click-decision number |
| 8 | completion badges | prevents the abandoned-work surprise |
| 9 | recommendation reasons | §16.1 already stores them; nothing renders them |
| 11 | DNF | the only genuinely new state, and the strongest negative signal |
| 7 | Surprise Me | §16.10 already specifies it |
| 14 | New in your fandoms | a time window on an existing join |

**Speccing next:** the local client (Lorebook) as its own section, anchored to §30
and §13.5.

**Specced, implementing after the above:** M45-28 WIP adoption (§56,
`docs/plans/m45-28-wip-adoption.md`).

**Deliberately excluded, with the argument above:** items 2 (archive-wide
scraping), and the streak / self-calibration / email-digest trio — those three are
in §57.8 and I will not add them later without you overriding me.

**Not yet assessed:** the remaining ~70 items. Several are genuinely good and
several (generalized media platform, the full seeding protocol) are multi-month.
I will work through them in impact order after the above, and I will keep saying
which ones I think are not worth doing.

## 4. One process note

I tried to ask you these three questions with the `clarify` tool and it rejected
the option lists three times. Per your standing decision style I proceeded on my
own judgment and am notifying you here rather than blocking. If any of the three
calls above is wrong, say so and I will change course — but you should know that
the reason nothing was asked is a broken tool, not that I judged your input
unnecessary.