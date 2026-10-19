# The Darkhold specification

**This directory is the specification. The monolith beside it is not.**

`../spec.md` is the 10,734-line specification this project was built against before the
conversion. It is **kept as the historical record** and it is not converted: the Darkhold
specification is a different document, and the honest way to hold two specifications is to keep
both and say which one governs.

## Which one governs

| | |
|---|---|
| **the governing specification** | `~/secondbrain/10-Projects/ultimate-fanfiction-site-spec.md` — Darkhold v2.3, and the split form of it in this directory |
| **the implementation plan** | `~/secondbrain/10-Projects/darkhold-implementation-plan.md` — 255 task cards, M0–M32 |
| **the historical record** | `../spec.md` — the pre-conversion specification |
| **the authority for what is built** | `../requirements.csv` — the requirement ledger, and ADR 0028 for how it is re-triaged |

§24.2 of the Darkhold specification is the rule the split follows: **a section number is a
citation target, and it is never renumbered.** The number space is reserved to §40 in blocks of
ten, the block titles are fixed when the block opens, and the directory numbers and the
document's section numbers are the same numbers — so the split does not renumber anything.

## The reserved skeleton

The slots are allocated in blocks of ten, and a section number is never reused:

```
docs/spec/
  00-index.md        the section ↔ milestone map, and the reserved skeleton
  01-…  10-…         the reserved skeleton, to §40, with each slot's title and owner
  …
  40-…              the last reserved slot
```

A retired section keeps its number and gets a "withdrawn" line. A new section takes the next
free number, and the index is updated in the same commit.

## Why the split exists

The monolith is the estate's clearest structural failure. A status document rots faster than
the code it describes, and a 10,734-line specification is a status document nobody re-derives.
The split gives each section a **milestone that owns it**, so a section is changed by the
person doing the work it describes, and a section's size is bounded by a thing a reader can
hold.
