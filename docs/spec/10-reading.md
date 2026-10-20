# 10. Reading

**Status:** reserved
**Milestone:** M5
**Source:** the corresponding section of the Darkhold specification

## 10.1 The reader surface

- a **server-rendered** page that works without JavaScript;
- a typography control that is the reader's alone, independent of the site theme;
- chapter navigation, a table of contents, a next-unit link;
- **position save** that survives a reload, a tab close and a device change;
- a **spoiler gate** the reader configures, and that a work can require;
- content warnings that are **visible, filterable, and never hidden by default**;
- a progress indicator that is honest;
- a "did not finish" action that is private, reversible, and never a judgement.

## 10.2 Content warnings and spoiler gating

| | what it does | where it lives |
|---|---|---|
| **content warning** | metadata about the work, filterable before opening it | `work_warnings`, `content_warnings` |
| **spoiler gate** | hides part of a page until the reader asks | the reader's preference + the work's `discussion_mode` |
| **reader filter** | removes matching works from a listing | `content_filters`, per reader, per domain |

**A warning is information, never a gate on the whole archive.** A reader who filters "graphic
violence" sees a *filtered list*, and the filter is visible and reversible. A reader who filters
nothing sees everything, with the warnings attached.

## 10.3 Reading state

`reading_progress` · `reading_event` · `reading_session` · `reading_aggregate` ·
`did_not_finish` (private) · `interaction_warmth` (private to the reader; shown to the *author* as
an aggregate band only — "this month, 34 lurkers, 12 reactors, 4 commenters, 2 creators" — and
never to the reader themselves).

**A warmth meter on your own reading is a streak-shaped dark pattern.** It is not shown.

## 10.4 Exports

Every work exports to **EPUB, HTML, TXT, Markdown, PDF, MOBI, AZW3** (MOBI/AZW3 via an optional
converter sidecar), plus **OPDS** and **Atom**. A work that a reader can read can be taken away.

The export must be a **fidelity claim** or an honest gap: "this export contains the metadata but
not the body, because this instance does not retain bodies" is a good message. A silently truncated
EPUB is not.

## 10.5 Offline

A service worker caches reader responses with stale-while-revalidate and shows an **offline
banner** naming the age of the cached copy. A reader must never mistake a stale page for a current
one.

## 10.6 The reading room

The reading experience is the one surface with no compromises, and it is re-skinned with the new
identity (§21) without changing any of the properties above.

---
