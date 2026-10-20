# 24. The ledger and the milestone plan

**Status:** reserved
**Milestone:** M0
**Source:** the corresponding section of the Darkhold specification

## 24.1 The ledger

`docs/requirements.csv`, seeded from **lorehaven's 703 rows** and ficnexus's and kindred's feature
inventories, re-triaged into Darkhold's vocabulary. Rows that are decisions rather than requirements
(the non-goals, the refusals) are carried across as `unsupported` — a standing exclusion, never
counted as remaining work. That is the mechanism by which the non-goals are *enforced* rather than
merely stated: a row cannot be silently reclassified as `planned` without the diff being visible.

**The first ledger work is a triage, not an implementation.** 703 lorehaven rows + ~400 ficnexus
features + kindred's algorithms, mapped into §5's vocabulary, deduplicated, and re-statused against
Darkhold's definitions. The result is the roadmap, and it is measured, not estimated.

## 24.2 The spec is split, not monolithic, and the numbers are stable

lorehaven's 10,734-line spec is the estate's clearest structural failure. Darkhold's spec is this
document, split into sections under `docs/spec/` with an index, and **each section is owned by a
milestone**. No section exceeds a length a reader can hold. A section that grows past that is split,
with the split recorded.

**A section number is a citation target, and it is never renumbered.** lorehaven has been through
a revision that renumbered its §-numbers, and there are notes in this vault that cite specific
sections — a renumber is a silent break in every one of them, and nothing reports it.

So the number space is **reserved before it is used**:

```
docs/spec/
  00-index.md        the section ↔ milestone map, and the reserved skeleton
  01-…  10-…         the reserved skeleton, to §40, with each slot's title and owner
  …
  40-…              the last reserved slot
```

**The rules:**

1. **A section number is never reused and never renumbered.** A retired section keeps its number
   and gets a "withdrawn" line.
2. **The skeleton is allocated in blocks of ten** (sections 1–10, 11–20, 21–30, 31–40 of the
   *repo's* spec directory), and the block's ten titles are fixed at the moment the block is
   opened. The directory numbers and this document's section numbers are the same numbers, which
   is the point — the split does not renumber anything.
3. **A new section takes the next free number**, and the skeleton's index is updated in the same
   commit.
4. **The spec in this vault is the master until the split lands**, and after the split the vault
   copy is the index — the canonical text lives in the repo, under the same numbers.

The point is not tidiness. It is that a citation like *"§12.12 means the voted cluster weights"*
has to keep meaning that in three years, and a note in this vault that says *"§9.8 fixes the
import"* has to keep pointing at the import.

## 24.3 Milestones

Each milestone is a vertical slice that ships, is verified, and updates the ledger. The order
follows the dependency graph, not the feature list.

| # | milestone | ships | depends on |
|---|---|---|---|
| **M0** | **The rename** | the repo, the binary, the config prefix (`LOREHAVEN_*` → `DARKHOLD_*`), the database name, the CSS token namespace, the docs, the domain, and the amendment pointer in this note's file name. The git history is kept; the ledger is re-triaged in place | — |
| **M1** | **Skeleton** | the workspace, the domain crate, the `domain`/`db`/`app` split, both migration sets, the CLI (`serve`/`migrate`/`doctor`/`seed`), the health endpoints, the embedded frontend shell | M0 |
| **M2** | **Identity** | accounts, pseuds, sessions, CSRF, scoped tokens, the actor model, the 404-not-403 doctrine | M1 |
| **M3** | **Governance foundation** | the trust ladder, trust events, the modlog, the audit trail | M2 |
| **M4** | **Content** | works, units, revisions, the publication event log, series, relations | M2 |
| **M5** | **The reader** | the server-rendered reader, the typography scale, the position save, content warnings, the spoiler gate, the progress state | M4, M3 |
| **M6** | **Taxonomy, themes and channels** | the canonical entities, the aliases, the closure, the work-tag assertions, the character and relationship assertions, the reader tag votes, **the six first-class themes**, the **tag-family author** rule, the channel object, the channel directory, **the instance tag de-prioritisation** (§6.10) | M4 |
| **M7** | **Search** | the query language, FTS on both engines, the trigram fallback, `main_char_attr`, `theme:` as a search field, the body full-text index and the backfill bin | M6 |
| **M8** | **Sources and ingestion** | the adapter contract, the safe fetcher, the sanitiser, the robots posture, the source sites, the credentials, the adult gate, the first 12 adapters ported | M1, M4 |
| **M9** | **The launch corpus** | **the whole AO3 mirror — every work with metadata, 111,686 measured (§9.8.1) — imported as exchange signals; the 10,266 QQ EPUBs enriched with tags from their source threads; the owner's 13 written works; and the held-out evaluation set** (§9.8) | M8, M6, M7 |
| **M10** | **Library and identity merge** | library items, the work source model, the identity proposals, the merge/split votes, the import flows, **the author identity** — the author's sibling of the work identity: the author names, the grouping, the source-linked pseuds, the opt-out, the reserved authors, and the author page that lists every work by that author from every source (§6.3.1) | M9, M6 |
| **M11** | **Exports and offline** | EPUB/HTML/TXT/MD/PDF, the optional converters, OPDS, Atom, the service worker, the offline banner | M5 |
| **M12** | **Community** | the forum, the work discussion modes, the collections, the requests, the notifications, the feeds | M4, M3 |
| **M13** | **The taste engine** | the CSR graph, the spectral embeddings, the co-bookmark index, the signals, the catalogue export, the `taste` binary, the budget gate, **T0/T1/T2 tiers** | M6, M7, M9 |
| **M14** | **The instance theme** | the theme object, the work/author picker, the derived house taste vector, the theme's publication, versioning and the front-page statement, the reason strings, **the instance tag de-prioritisation applied to the house vector** | M13, M4, M10 |
| **M15** | **Weight fitting** | the signal weights fitted by grid search against an NDCG@10 objective, the deterministic hold-out evaluation harness (precision@k, recall@50, NDCG@10), the per-fandom cap measured rather than assumed, and the fitted values replacing the inherited table in §12.7 (§24.4) | M9, M13, M14 |
| **M16** | **The arena** | Glicko-2 with draws, the pairing strategies, the entity arenas, the theme-weighted consensus, the block page, the degradation honesty | M13 |
| **M17** | **Moods** | the mood object, the online training loop, the activation session, the cold-mood degradation, the trust/supporter gate, the training-event audit | M13, M14 |
| **M18** | **Voted cluster weights** | the theme ballots, the approval tally, the seed/community blend (`effective_weight`), the per-period rate limit, the k-anonymity floor on published tallies, the versioned weights in the catalogue, the channel's published weight profile, the withdrawal-without-deleting-votes path (§12.12) | M6, M13, M14, M12 |
| **M19** | **Discovery** | the ranker, the strategy registry, the RRF fusion, the golden guarantee, the shadow mode, the sort contract, half-life, the "more like this" ladder with reasons, **the channels' streams** | M13, M15, M16, M17, M18 |
| **M20** | **Guests and the fold** | GS-3, GS-1, GS-2 | M2, M3 |
| **M21** | **The jury** | GS-4's elasticity, GS-5, the jury draw, the appeals, the outcome ladder | M3, M12, M20 |
| **M22** | **The exchange and the snapshot** | the metadata exchange, the signed snapshot, the onion transport, the privacy check | M6, M21 |
| **M23** | **Extensions** | the marketplace, the four tiers, the trust-gated publication, the remix, the manifest | M19 |
| **M24** | **The Supporter tier** | the entitlement, the grant sources (self-hosted, payment, federated, code), the tier gate, the lapse path, the "compute not rank" mutation test | M13, M19 |
| **M25** | **Federation** | ActivityPub, the JSON-LD, the cross-instance deferral's shape | M11, M12 |
| **M26** | **The Darkhold identity** | the token rework, the new palette and type scale, the reader's dark surface with a measured contrast ratio, the three new presets, the front page | M5, M6, M14 |
| **M27** | **Site profiles** | the profile system, the first four skins (AO3, FFN, Wattpad, RoyalRoad), the parity checklist per skin | M5, M11, M12 |
| **M28** | **The remaining adapters** | the 107-site parity list, ported and re-verified | M8 |
| **M29** | **Media** | lorehaven's §32 media model generalised, the resilience and mirroring | M4, M8 |
| **M30** | **The expansion workflow** | the corpus expansion's contract (§9.10) built on Darkhold's side: the exchange at volume, the resumable and idempotent catalogue write, the expansion's seed and configuration surface, the run record, and the **author identity's** application to the expanded corpus | M8, M9, M10 |
| **M31** | **The expansion run** | one expansion run executed and **evidenced**: the seed set, the plan's exact request counts, the run's checkpoint, the works and authors it added, and the **metadata-extraction verification** — the assertion that the extracted metadata is actually being stored and is actually being read | M30 |
| **M32** | **The Pi deployment** | the whole instance — archive, taste service, Postgres — running on a Raspberry Pi 5B, with the RSS of each process measured against §22.3's caps, the total footprint measured against the 2.5 GiB frame, and the tiers' behaviour re-verified on the real hardware | M19, M24, M25, M31 |

**A deployable archive exists at M11. A recommender exists at M19. A self-governing instance
exists at M22. The instance's voice exists at M14. Fitted weights exist at M15. Moods exist at M17.
Community-weighted themes exist at M18. The authors are people again at M10. The corpus grows by a
workflow at M30 and the run is evidenced at M31. The whole thing fits on a Pi at M32.**

**The order rationale.** M0 and M1 first because everything lands in the renamed repo. **M9 early**
because the corpus — now the **whole mirror, 111,686 works** (§9.8.1) — is the substrate for the
theme (M14), the engine (M13) and the evaluation fixture, and it is the largest single thing the
instance holds. M13 before M14 because the theme's derived vector is computed by the same engine
that ranks. **M14 before M15** because `house_taste` is one of the fitted weights and the house
taste has to exist to be fitted. **M15 before M19** because the discovery milestone ships the
weights the ranker uses, and shipping inherited weights as if they were measured is exactly the
failure the estate keeps recording. M14 before M17 because a mood's zero vector must be able to
say "cold" honestly, which requires the house taste to exist and be distinguishable from a mood.
M17 and M18 before M19 because the mood reweight and the voted weights both ride the ranker.
**M18 after M12** because a disputed vote is contestable through the ordinary report path —
**a vote does not need a jury to be meaningful**, it needs a way to be disputed. **M26 late among
the visible work** because re-skinning pages whose backend does not exist yet is how the 543 tests
get weakened instead of re-derived. **M10 before M14** because the instance theme is chosen from
**authors** (D-11), and choosing an author before the instance knows who authors are chooses a
string. **M30 and M31 after M9** because the expansion grows a corpus that must already be
imported, and the run is only evidence if there is a catalogue to compare it against. **M30 and
M31 before M32** because the Pi's footprint is measured against a corpus, and a footprint
measured against a corpus that is about to grow is not a measurement. **M32 last of all** because
the Pi deployment is not a port —
it is the **validation of the tier model**, and you cannot validate a tier model on hardware you
are not deploying to.

## 24.4 The first three tasks, concretely

1. **The rename (M0) — done.** Repo, binary, library crates, config prefix, database name,
   test scratch directories, docs, ADRs, and the amendment pointer. The history was kept
   (ADR 0027), the 703-row ledger is re-triaged in place rather than restarted (ADR 0028), and
   the instance's tagline is *the dark side of fanfiction*.

   **The CSS token namespace was deliberately NOT renamed** — the tokens are `--color-*`,
   `--reader-*`, `--font-*`, `--radius-*`, `--leading-*` and `--duration-*`, 114 declarations,
   and not one carries the product name. This specification names `--reader-*` itself
   (§21.2 constraint 1), so a `--dh-*` namespace would have contradicted the spec and churned
   every `var(--…)` use for no gain. **The finding is recorded, not the change.**

   **One command reproduces the committed history exactly** — the same 1,048 commit SHAs, the
   same 1,265 blobs, the same tip — by re-running the rewrite against the pre-rename bundle
   and comparing. **A rewrite that cannot be re-run is a rewrite that cannot be checked.**
   The evidence is the `## 2026-10-19 — M0` entry in `docs/verification.md`.
2. **The triage (§24.1).** Produce the ledger: every source row mapped into §5's vocabulary, the
   duplicates merged, the statuses re-derived, the non-goals carried as `unsupported`.
3. **The theme seed (§2.1 → M14).** The 54 works of §2.1 become the instance theme's seed:
   one merged profile, written and bookmarked at equal weight, with the tag vector derived and the
   six themes attached, each expanded by its PMI neighbours (§2.2). This is the smallest thing
   that makes the instance have a voice.

**And the one piece of work that is measurable before any of it:** the **weight fitting (M15)**, which
the estate already has both halves of. `ao3_recommender/optimizers/weight_optimizer.py` grid-searches
the signal weights against an **NDCG@10** objective, and `ao3_recommender/evaluation/` is the
measurement that says whether the optimisation helped — its own docstring is the finding:

> *"The missing half of WeightOptimizer. That module grid-searches weights to maximise NDCG@10 —
> and then had no way to check whether the optimisation helped, because nothing ever measured the
> result. This is the measurement."*

The **deterministic** hold-out is what makes it usable: *"A random holdout would make two runs of
the same config produce different metrics, and a metric that moves when nothing changed cannot be
used to decide whether a change helped."* The held-out set is the owner's 41 bookmarked works
(§9.8.2), and the fitted weights replace the inherited table in §12.7.

---
