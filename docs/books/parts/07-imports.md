# Part 7 — Imports: fetching, adapters, sanitising, shelf CSVs

Checkpoint: `v0.08-positivity`

Two different things are called "import" in this project, and confusing them
costs a week:

1. **Works import** — fetch a story from another site and turn it into a work
   with chapters. Fetches the network, needs a source adapter, produces content.
2. **Shelf imports** — read a Goodreads or StoryGraph export and produce library
   rows for the reader. Never touches the network, never produces content.

They share a word and nothing else. This part builds both.

## 1. Checkpoint

```bash
git checkout v0.08-positivity
```

## 2. What will work by the end

```bash
# A URL import becomes a queued job, then a draft work.
curl -X POST localhost:8080/api/v1/imports/preview -d '{"url":"https://example.invalid/story/1"}'
curl -X POST localhost:8080/api/v1/imports -d '{"url":"…","kind":"work"}'
curl localhost:8080/api/v1/jobs/$JOB

# A shelf export becomes library rows, states included.
curl -X POST localhost:8080/api/v1/library/imports/csv \
  -H 'content-type: application/json' \
  -d '{"format":"storygraph","csv":"Title,Authors,ISBN,My Rating,Date Read,Review\n…"}'
# { "imported": 2, "kept_existing_state": 0, "refused": [ { "title": "line 3", "reason": "…" } ] }
```

## 3. Concepts

- **The fetcher is the only thing that talks to the outside world**, and it has
  its own rules: allow-listed schemes, bounded redirects, bounded body size, an
  honest `User-Agent`, and robots rules that are read rather than guessed.
- **An adapter is a pure function over documents.** Give it HTML, get chapters.
  It does no I/O, so it is testable against a saved file.
- **Sanitising is not optional and not a rendering concern.** Fetched text is
  cleaned *before* it is stored, so no later path — export, feed, API — can
  reintroduce what was stripped.
- **Imports are attributed.** An imported work carries its source URL, its
  adapter, and when it was fetched. That is the record that makes a takedown
  request answerable.
- **A CSV export is not a scrape.** It is a file the reader downloaded from a
  service about themselves. It gets no network call, no HTML parsing, and no
  assumptions about where any book's text is.

## 4. Commands

```bash
lorehaven migrate        # applies 0006_imports
cargo test -p lorehaven-scrapers
cargo test -p lorehaven-app --test milestone_6
cargo test -p lorehaven-app --test milestone_24
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0006_imports.sql` | imports, batches, source credentials, library items, provenance |
| `crates/scrapers/src/lib.rs` | the fetcher: schemes, redirects, size, robots |
| `crates/scrapers/src/html.rs` | document parsing helpers |
| `crates/scrapers/src/csv.rs` | Goodreads and StoryGraph shelf parsing, and the import plan |
| `crates/scrapers/src/csv/goodreads.rs`, `csv/storygraph.rs` | one file per export format |
| `crates/domain/src/imports.rs` | chapter identity, plan shape |
| `crates/db/src/imports.rs` | import rows, batches, `upsert_library_item` |
| `crates/app/src/imports.rs` | the job handler |
| `crates/app/src/routes/imports.rs` | preview, submit, cancel, and the shelf CSV door |
| `crates/app/tests/milestone_6.rs` | work-import acceptance tests |
| `crates/app/tests/milestone_24.rs` | shelf-import acceptance tests |
| `frontend/src/routes/Import.svelte` | the import page |

## 6. The code that matters

### The fetcher's contract

```rust
// crates/scrapers/src/lib.rs — the boundary between a pasted URL and the network
const MAX_REDIRECTS: usize = 5;
const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;
```

Every one of these is a real incident shape: an unbounded redirect chain to an
internal address, a body that fills the disk, a `file://` URL that reads the
server's own filesystem. Refuse first, fetch second. And name the site honestly:
a descriptive `User-Agent` with a contact URL, because the alternative is being
blocked for good.

### Adapters as pure functions

```rust
// crates/scrapers/src/html.rs — one adapter per source, no I/O inside
pub fn chapters_from_document(html: &str) -> Result<Vec<PlannedChapter>>
```

Test each adapter against a saved document from the real site, checked into the
repository. That file is also your evidence of what the site actually served on
the day you wrote the adapter — which is worth more than a screenshot when the
site changes and a reader asks why their import looks wrong.

### The shelf import plan

The CSV path is where a junior developer is most likely to write something that
works and is subtly wrong, so it is worth walking through the real flow:

```rust
// crates/scrapers/src/csv.rs
pub struct ImportPlan { pub items: Vec<PlannedItem>, pub refused: Vec<RefusedRow> }

pub fn plan_shelf_import(shelf: &ImportShelf) -> ImportPlan
```

The plan is a **pure function of the parsed file**. Nothing in it touches the
database; the route walks the plan and writes. That is what lets you unit-test
every refusal rule without a server.

The rules the plan enforces:

| Input | Result |
|---|---|
| title and author present | an item |
| `Date Read` = `2019/03/09` | state `finished`, `finished_at` = `2019-03-09T00:00:00Z` |
| `Date Read` = `2019-12-31T10:11:12Z` | the timestamp, with its time |
| `Date Read` = `31 December 2019` | **refused**, naming the value as written |
| no title | **refused by file line** |
| rating, shelves, review text | carried in `provenance_json` |

Two details in that table are the ones people get wrong:

**The timestamp branch must be tested first.** A full RFC 3339 timestamp also
starts with a `YYYY-MM-DD` shape. If the date-only branch runs first, the time is
silently dropped and `2019-12-31T10:11:12Z` becomes midnight. This is a bug that
survives review because the date is right.

**A row the parser could not read is refused by line number.** A row with no
title has no other identity in the file. "3 rows skipped" leaves the reader
searching their export by eye; "line 3 has no title" does not. That is why the
parsers record the line numbers they skip instead of only counting them:

```rust
pub struct ImportShelf {
    pub rows: Vec<ShelfRow>,
    pub skipped: usize,
    /// The file lines the parser could not read, 1-based, header included.
    pub skipped_lines: Vec<usize>,
}
```

### Imported rows do not overwrite the reader

```rust
// crates/db/src/library.rs
pub async fn set_imported_reading_status(...) -> Result<bool>   // writes only if absent
```

A re-import reports `kept_existing_state` rather than overwriting a state the
reader set themselves. The mirror-image rule applies to the item row: the unique
key `(account, source, source_work_key)` makes a re-import update the same row,
so importing the same file twice is idempotent by construction rather than by a
check that could race.

### No invented URLs

```rust
source_url: format!("import://{format}/{}", item.source_work_key)
```

A shelf export carries no URL for a book. A fabricated `https://` link is a link
somebody will later follow and act on. An `import://` URI says where the row came
from and is not fetchable by anything. When in doubt, prefer a value that cannot
be mistaken for a live address.

## 7. Tests

`milestone_6.rs` (works import) asserts:

- a `file://` URL and a redirect chain are refused before any request is made;
- an oversized body is refused rather than buffered;
- a saved document produces the expected chapters, in order, with sanitised text;
- an imported work is attributed: source URL, adapter, fetch time;
- the import job is visible to its owner and to nobody else.

`milestone_24.rs` (shelf import) asserts:

- a StoryGraph-shaped file imports two rows and refuses the untitled one by name;
- the finished row carries the reader's `Date Read`, not the import time;
- re-importing the same file adds nothing and reports `kept_existing_state`;
- an unknown format is refused before anything is written.

## 8. Expected UI behaviour

- The import page shows a preview before it creates anything: the work, the
  chapter count, the source.
- A running import shows progress per chapter.
- The library page shows imported rows with their provenance, and never claims
  the instance holds the text of a book it does not have.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Import hangs | no overall timeout on the fetch | bound connect, read and total time |
| Chapters in the wrong order | the adapter took document order | order by the site's own ordinals; fall back to document order only when there are none |
| Imported text contains scripts or odd markup | sanitising happened at render | sanitise before storing |
| Re-import duplicates rows | no unique key on (account, source, source_work_key) | add it; make the write an upsert |
| Imported count is wrong after a re-import | counting rows written rather than states set | count the state changes, and report what was kept |
| `Date Read` loses its time | date-only branch matched first | check full timestamps first (see §6) |

## 10. Consequences

- **You are storing someone else's text.** The attribution record and the
  takedown path are not bureaucracy; without them a single complaint has no
  answerable response.
- **Credentials for source sites are secrets** on the Part 5 rules: encrypted at
  rest, decrypted only at use, never returned by an API, and included in the
  "revoke everything" path when an account is closed.
- **An imported shelf is a statement about a person's reading.** It is private
  data: never public, never in a shared export, and deletable in one action.

## 11. Checkpoint

```bash
git tag v0.07-importing
```

Verified by the `lorehaven-scrapers` unit suite, `milestone_6.rs` and
`milestone_24.rs`, and by hand: import a saved document from a test fixture and
import a StoryGraph CSV, then re-import each and confirm nothing duplicates.
