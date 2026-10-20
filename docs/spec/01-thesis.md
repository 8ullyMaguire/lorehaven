# 1. The thesis

**Status:** reserved
**Milestone:** M0
**Source:** the corresponding section of the Darkhold specification

## 1.1 What "ultimate" means here

Not "the union of three feature lists". Union is the failure mode — three good projects each
missing half of the product, welded together, produce a worse thing than any of them. "Ultimate"
means six properties, each falsifiable:

1. **Reading is the product.** Everything else is a means to the next chapter. The reader surface
   is the only surface never degraded, never paywalled, never gamified, and never made to wait for
   a service.
2. **Ingestion is unbounded and provenance-preserving.** Any work from any site can be brought in,
   read, and exported again, with the raw source bytes retained and the canonical record curated
   separately.
3. **Taste is learned, not configured.** The system works out what a reader likes by asking them
   pairwise questions and by watching what they keep — and it can do the same for tags, characters,
   ships, fandoms and authors, not just works.
4. **The instance has a voice.** The house theme is a *published editorial position*, chosen from
   named works and authors, and it pulls every reader's recommendations toward it. A stranger
   arrives and the site already has opinions.
5. **Governance works at 3 users and at 3,000.** Trust is earned, elastic, and drawn from the
   community; the operator is the last resort, not the first.
6. **Nothing is locked in.** Every work, collection, rating, tag, comment and statistic readable in
   the UI is readable by a machine, exportable in an open format, and never retrievable only by
   scraping this instance.

## 1.2 The failure this avoids

| project | the failure | the cost | the antidote |
|---|---|---|---|
| lorehaven | a 10,734-line spec monolith with 703 rows where **360 have never been held to the full-stack bar** | "built" and "works" diverged; 5 open defects shipped as features | §23 makes the ledger re-derivable and holds every row to one bar |
| ficnexus | 8 parallel authority columns on `users` and 3 separate XP ledgers, on top of 18 failing DB-gated suites | authority was ambiguous; a never-green suite was reported as a feature | §6.1 defines one ladder; §23 forbids "the suite is red" as a deliverable |
| kindred | the tag arena was **inert in production and silent about it** | the feature looked healthy and was dead | §12.7 makes degradation explicit and machine-readable |

Darkhold is the last of these lessons, not the first: the ledger, the re-derivation command, the
degradation flag, the elastic trust ladder and the two-engine gate are not features of the product.
They are the product.

## 1.3 The pitch

> **Darkhold — the dark side of fanfiction.** A self-hosted archive that learns your taste, says
> so, and tells you why it recommended what it recommended.

## 1.4 What is genuinely new

1. **A canonical vocabulary and ID space** (§5). The three projects call the same thing six
   different names and use three incompatible identifier schemes.
2. **A two-process architecture with one domain** (§8.2). A recommender holding a 124 MB
   in-memory graph and a forum holding a database pool have irreconcilable memory shapes. The
   archive is one workspace, one schema, one vocabulary, and two binaries — collapsing to one when
   the corpus is small.
3. **A published instance theme** (§12.1). Not a hidden operator preference — a *stated editorial
   policy*, chosen from works and authors, that pulls every reader's recommendations and is
   visible on the front page. This resolves the conflict that lorehaven and kindred could not
   both hold.
4. **Moods the reader names and the system learns** (§12.3). A mood is a user-created, named,
   online-trained weighting vector. It is not a filter, not a preset, and not a shipped model.
5. **Elastic trust + jury governance** (§14.4, §14.5). A three-user instance still has a
   moderator; a three-thousand-user instance does not hand the power out. Nobody has built this.
6. **A compute-tiered recommender** (§12.6). The expensive recommendation tier is paid for; the
   cheap one is not. Paying buys *compute*, never rank, never influence, never moderation.

---
