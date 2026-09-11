# Milestone 8 — Library, saved views, bookmarks and updates: **landed**

Spec §16 as the plan numbers it; §14 in the spec text. Tag `v0.09-library`.

The plan's shape was followed and two things were decided while building it. Both
are stated here rather than only in the code, because each one changes what a
reader can do.

## What landed

| Piece | Where |
|---|---|
| Migration 0009 — `shelves`, `shelf_items`, `bookmarks`, `private_tags`, `reading_status`, `saved_views`, `update_checks` | `migrations/{sqlite,postgres}/0009_library.sql` |
| The vocabulary and the rules | `crates/domain/src/library.rs` (18 tests) |
| Storage: reads, writes, the query builder, storage accounting | `crates/db/src/library.rs` |
| HTTP: shelves, shelf items, bookmarks, private tags, statuses, saved views, the listing, storage, the check | `crates/app/src/routes/library.rs` |
| The update check as a job | `crates/app/src/library_updates.rs`, `JobKind::UpdateCheck` |
| Acceptance tests | `crates/app/tests/milestone_8.rs` (10 tests) |
| The library page | `frontend/src/routes/Library.svelte` (+ 8 tests) |
| Whole-work mode | `frontend/src/routes/Reader.svelte` (+ 3 tests) |
| The card's three densities | `frontend/src/lib/components/WorkCard.svelte` |

## Two decisions that were not in the plan

### 1. The library listing replaced the one M6 built, and carries the reader's own facts

`GET /library/items` was M6's: a flat page of imported works with a cursor, no
filters. It could not stay beside M8's, because two routes cannot occupy one path
— so M8's replaced it, keeping the same item projection (`items`, `next_cursor`
per spec §3.3) and adding `total` and the filters.

The listing also carries each item's reading status, private tags and shelves.
That is not a property of the work, and it could have been three more requests per
card; instead it is three queries per *page*
(`library::facts_for_items`), so one screen is one request.

The item projection was extracted to `routes::library::item_json`, and M6's
`list_library` was left in place, marked retired, rather than deleted silently —
its shape is the one the new route has to keep.

### 2. Deleting a reference and deleting a copy are different *moments*, not different references

The plan's `delete_copy` reads as though the reference survives one way and not
the other. It cannot: a `content_references` row pointing at a deleted library
item is reachable by nothing and collectable by nothing, so it holds the blob
alive for ever.

So the item and its references always go, and `delete_copy` decides **when the
bytes follow**:

* `true` — now: the blobs that lost their last reference are reported and freed,
  and `freed_bytes` is what the reader is told they freed.
* `false` — later: the reference is gone and the bytes are unreferenced, which is
  exactly what the maintenance sweep collects (`BlobStore::unreferenced`).

The reader-visible difference is the timing, and the storage panel says so.

## The update check reads anonymously, and says what it could not see

It re-reads each item's **metadata** — the chapter list and the source's own
"last changed" stamp — and never a chapter body: the point is to say something
changed, not to change it. An item whose source needs a credential cannot be read
this way, and rather than fail the whole run for one item the check records that
it could not see the work and moves on.

Using the reader's saved credential here is the obvious next step and is **not
done**. The report says "not checked" rather than implying a check found nothing,
because the second is a claim and the first is a fact.

## Whole-work mode paginates, and that is the only way §9.2 and §9.2 agree

Spec §9.2 asks for a way to read a work through and, in the same breath, that long
works must not require rendering every paragraph at once. Both only hold if the
mode pages: the reader gets the chapter the address names, and the next one is
appended when they approach the end of what is loaded (a 600 px margin, so it is
usually there before they arrive). A work of two hundred chapters is two hundred
fetches nobody notices and nobody pays for at once.

`remember()` follows the chapter actually on screen rather than the one the
address names, so a reader who reads on and leaves resumes where they stopped.

## The migration parity test was strengthened, and falsified before being trusted

`crates/db/src/migrate.rs` compared the two dialects' migration **ids** and
stopped there — the gap written down in `verification.md` after PostgreSQL's
`reading_progress` index lost `device_id`. It now compares table names, column
names and index columns per migration id. It was falsified by removing a column
from the PostgreSQL half and confirming it names the migration and the table.

Types are deliberately not compared: `INTEGER`/`BIGINT` and `REAL`/`DOUBLE
PRECISION` are *supposed* to differ (ADR 0004).

## Left open, and stated

* ~~**No live PostgreSQL run for these tables.**~~ **Closed after this document was
  written.** The journey was then run against PostgreSQL 17.11 — 47 steps, 0
  failures — and it found four defects in this milestone's own new code that SQLite
  could not see (a raw branch passing `?` instead of `$1`, `?::bigint::boolean`,
  a missing `::uuid`/`::text` pair in the per-page facts query, and `SUM()` of a
  `bigint` decoding as `numeric`). See `verification.md`, *Milestone 8*, for each
  one. The lesson is in the plan rather than only in the fix: the dialect rules
  being followed is not the same claim as PostgreSQL having accepted the SQL.
* **No live network run for the update check.** The route and the job are covered;
  the per-item fetch is not.
* **A saved view holds a list per facet; the filter bar holds one.** Clicking a
  view loads the first value of each facet and *says so* when the view carried
  more, rather than applying a filter silently narrower than the one the reader
  saved. The API and the store keep the whole list; only the bar is narrower.
* **`M1-09` is `partially-implemented`.** The work-card variants are done; the
  combobox, the permission-request panel, the extension-slot boundaries and the
  M10 panels are with the milestones that need them.
