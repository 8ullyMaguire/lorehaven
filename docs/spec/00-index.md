# 0. The Darkhold specification — the index

**This directory is the specification.** The vault file it was split from
(`~/secondbrain/10-Projects/ultimate-fanfiction-site-spec.md`) remains the master
until the split is complete; after that it becomes the index and this directory
is the specification.

**The four rules** (spec §24.2):

1. **A section number is a citation target, and it is never renumbered.** The number space is reserved to §40 in blocks of ten, the block titles are fixed when the block opens, and the directory numbers and the document's section numbers are the same numbers — so the split does not renumber anything.
2. **A retired section keeps its number** and gets a withdrawn line. A number is never reused, so a citation to a retired section still resolves to the thing it cited.
3. **A new section takes the next free number,** and the index is updated in the same commit as the section.
4. **The vault file remains the master until the split is complete.** After that it becomes the index, and this directory is the specification.

## The section map

| slot | file | title | milestone |
|---|---|---|---|
| **01** | `thesis.md` | The thesis | M0 |
| **02** | `evidence.md` | Evidence base | M0 |
| **03** | `inheritance-and-decisions.md` | Inheritance | M0 |
| **04** | `non-goals.md` | Non-goals — the hard refusals | M0 |
| **05** | `vocabulary.md` | The canonical vocabulary | M1 |
| **06** | `domain-model.md` | Domain model | M1 |
| **07** | `data-model.md` | Data model | M1 |
| **08** | `architecture.md` | Architecture | M1 |
| **09** | `ingestion.md` | Ingestion | M8 |
| **10** | `reading.md` | Reading | M5 |
| **11** | `search-and-discovery.md` | Search and discovery | M7 |
| **12** | `taste-themes-moods-channels.md` | Taste, themes, moods and channels | M13, M14, M17, M18 |
| **13** | `community.md` | Community | M12 |
| **14** | `governance.md` | Governance — guests, trust, jury | M3, M20, M21 |
| **15** | `privacy-and-safety.md` | Privacy and safety | M22 |
| **16** | `extensions.md` | Extensions and the marketplace | M23 |
| **17** | `federation.md` | Federation and interoperability | M25 |
| **18** | `the-supporter-tier.md` | The Supporter tier — compute, not rank | M24 |
| **19** | `configuration.md` | The configuration contract | M1 |
| **20** | `api-conventions.md` | The API conventions | M19 |
| **21** | `frontend-and-design.md` | Frontend and design | M26 |
| **22** | `operations-and-budgets.md` | Operations, deployment and budgets | M32 |
| **23** | `verification.md` | The verification discipline | M1 |
| **24** | `the-ledger-and-the-milestones.md` | The ledger and the milestone plan | M0 |
| **25** | `risks.md` | The risk register | — |
| **26** | `open-questions.md` | Open questions | — |
| **27** | `appendix.md` | Appendix — the measured numbers this spec inherits | M0 |

**Slots 28 to 40 are reserved and unallocated:** **28**, **29**, **30**, **31**, **32**, **33**, **34**, **35**, **36**, **37**, **38**, **39**, **40**.

## Which document governs

| | |
|---|---|
| **this directory** | the Darkhold specification, split into sections — **the governing document** |
| **the master** | `~/secondbrain/10-Projects/ultimate-fanfiction-site-spec.md` — the master until the split is complete, and the index afterwards |
| **the implementation plan** | `~/secondbrain/10-Projects/darkhold-implementation-plan.md` — 255 task cards, M0–M32, the build order |
| **the historical record** | `../spec.md` — the pre-conversion specification, kept and not converted |
| **the authority for what is built** | `../requirements.csv` — the requirement ledger, and ADR 0028 for how it is re-triaged |

## How this directory was made

`python3 scripts/split-spec.py` re-derives every file from the master, and
`python3 scripts/check-split-spec.py` asserts that each file's heading number
matches its filename and that the split is in step with the master. **A split
that is done by hand is a split that is done once**, and a section that changed
in the master would show up as a divergence nobody notices.

## How to read this

**Normative vocabulary.** MUST / MUST NOT / SHOULD / MAY are binding. "Defined" means a term has
exactly one meaning and the three source projects' words for it are mapped in §5. "Proven" means a
number was measured on real hardware against real data and is re-derivable — never "estimated".

**The evidence rule.** Every claim of the form "X works" cites a measurement or says plainly that
it has not been built. A feature mentioned in a document is not evidence of implementation.

**Reading order for a fresh agent.** §1 (thesis) → §3 (inheritance) → §8 (architecture) →
§12 (taste) → §23 (verification) → §24 (ledger). Everything else in the order it is needed.

---
