# ADR 0002 — Content model

Status: accepted (Milestone 0)
Date: 2026-09-10

## Problem

Three requirements pull in different directions:

1. Spec §8 separates a work's *lifecycle* (`draft`…`published`…`withdrawn`) from
   its *completion* (`in_progress`…`complete`), and from its *visibility*
   (`public`/`unlisted`/`restricted`). These are presented to users as
   different questions and change on different schedules.
2. Spec §9 requires chapter revisions to be immutable enough that reading
   progress survives an edit, and that "a chapter update does not crash resume".
3. Spec §4.3 requires a structured editor document, with sanitized HTML and
   plain text as *derived* representations.

## Decision

- **Three orthogonal state fields on a work**: `lifecycle`, `visibility`,
  `completion`. They are separate columns because they are separate questions:
  a work can be `published` + `unlisted` + `in_progress` all at once.
- **The editor document is the source of truth.** `chapter_revisions` stores
  `document_json` (a restricted Tiptap schema), plus `sanitized_html` and
  `plain_text` as derived columns. Derived columns exist so that search,
  download and the reader do not each re-derive them differently; they are
  never edited and never authoritative.
- **Revisions are append-only.** Restoring an old revision creates a *new*
  revision (spec §8 acceptance). Nothing in the reader's history is ever
  rewritten, so a stored `content_revision` always resolves.
- **Publication is a state change plus an event, in one transaction.**
  `publication_events` records what happened and when; notifications and
  indexing are emitted as outbox rows in the same transaction and delivered
  later, so no email is ever sent from inside a database transaction (spec §8).

## Alternatives considered

**One `status` column** with values like `draft`, `published`, `complete`.
Rejected: it makes "complete but unlisted" unrepresentable, and those are
ordinary states.

**A generic `states` JSON blob.** Flexible, but it puts searchable state
outside the schema. Spec §4.1 explicitly rules this out: JSON for flexible
documents, not as a substitute for searchable relationship data. The taxonomy in
spec §14 depends on querying state.

**Mutable chapters** with a revision history table for auditing. Simpler to
write, but it makes resume fragile: a reader's stored position would point into
content that has since changed underneath it, which is exactly the failure
spec §9 forbids.

**Storing only the document JSON** and deriving HTML on every read. Attractive
for consistency, but it means either sanitizing on every request (expensive) or
trusting the client (unacceptable). Sanitizing once, on write, and storing the
result is the safer default.

## Consequences

- Reading position can be stored as a stable paragraph anchor plus a revision
  id, with a documented fallback to an approximate fraction when content
  changed (spec §9).
- Storage grows with edits rather than with size: a heavily revised work costs
  more disk than a flat file would. Revision pruning is a retention decision,
  not a schema decision, and is not implemented yet.
- Every consumer of chapter text must be told which representation it wants
  (document, HTML, or plain text); the columns make the wrong choice visible.

## Conditions that would justify revisiting

- Revision storage becomes a measured share of total disk that pruning cannot
  control.
- The Tiptap schema needs composition (embeds, tables) that `document_json`
  plus derived HTML cannot express safely — at which point sanitization moves
  into the render path and the derived columns become caches with a stated
  invalidation rule.
- A second consumer needs a fourth representation (for example a structured
  diff), which would argue for deriving all representations from one pipeline
  rather than storing three.
