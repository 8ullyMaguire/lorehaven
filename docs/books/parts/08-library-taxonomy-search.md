# Part 8 — Library, shelves, taxonomy and search

Checkpoint: `v0.07-importing`

A reader with four hundred imported rows needs to find one of them. This part is
about making a large personal library navigable, and about the one thing every
list endpoint gets wrong the first time: pagination.

## 1. Checkpoint

```bash
git checkout v0.07-importing
```

## 2. What will work by the end

```bash
curl -X POST localhost:8080/api/v1/shelves -d '{"name":"read-again"}'
curl -X PUT  localhost:8080/api/v1/library/items/$ITEM/status -d '{"status":"finished"}'
curl -X POST localhost:8080/api/v1/library/items/batch -d '{"ids":["…"],"action":"remove"}'

curl 'localhost:8080/api/v1/library/items?sort=recent&limit=50'
curl 'localhost:8080/api/v1/works?q=rating:4..5 mood:hurt-comfort words:<20000&limit=20'
curl 'localhost:8080/api/v1/works?q=title:salt&limit=20&cursor=…'
```

## 3. Concepts

- **Shelves are the reader's, tags are everyone's.** A shelf is a private
  grouping with a name the reader chose; a tag is public metadata on a work.
- **Saved views are queries with a name.** Not a separate storage system.
- **A query language needs a parser and a compiler**, and the compiler must
  produce SQL with bound parameters — never string interpolation.
- **Fuzzy matching is for humans typing**, so it ranks. Exact matching is for
  filters, so it filters.
- **Pagination is part of the API contract, not an optimisation.** A list
  endpoint with a silent `LIMIT` is a bug that shows up as "my work disappeared
  from the library".

## 4. Commands

```bash
lorehaven migrate        # applies 0009_library and 0011_taxonomy
cargo test -p lorehaven-app --test milestone_8
cargo test -p lorehaven-app --test milestone_9
cargo test -p lorehaven-app --test milestone_10
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0009_library.sql` | library items, shelves, shelf items, reading status, bookmarks, saved views |
| `migrations/sqlite/0011_taxonomy.sql` | tags, moods, tag applications |
| `crates/domain/src/library.rs` | shelf rules, reading states, view definitions |
| `crates/domain/src/taxonomy.rs` | tag normalisation, mood vocabulary |
| `crates/domain/src/query.rs` | the query language: lexer and parser |
| `crates/domain/src/query_sql.rs` | the compiler: AST → SQL + bound parameters |
| `crates/db/src/library.rs` | the library's tables |
| `crates/db/src/taxonomy.rs` | tags and moods |
| `crates/db/src/search.rs` | the retrieval paths (and `search/` for the fuzzy one) |
| `crates/app/src/routes/library.rs` | shelves, status, bookmarks, saved views |
| `crates/app/src/routes/taxonomy.rs` | tags and moods |
| `crates/app/src/routes/search.rs` | the query door |
| `frontend/src/routes/Library.svelte`, `Search.svelte` | the pages |

## 6. The code that matters

### The query language, compiled not concatenated

```rust
// crates/domain/src/query.rs    parse("rating:4..5 mood:hurt-comfort words:<20000")
// crates/domain/src/query_sql.rs  →  (sql_fragment, Vec<BoundValue>)
```

The compiler's output is a fragment containing `?` placeholders and a list of
values. Every value the user typed is a bound value. A query language is the
single most attractive place in a codebase to build a SQL injection, because the
shortcut ("just format the number in") works, tests green, and is exploitable.

Two more rules for this module:

- **Unknown fields are an error, not a no-op.** `ratng:4` silently ignored means
  the reader believes they filtered and sees unfiltered results.
- **Limits are clamped, not refused.** `limit=100000` becomes the maximum, with
  the effective limit in the response, so a client can tell.

### Cursor pagination, done properly

The bug, in its usual form:

```sql
-- wrong: stable only if nothing is inserted while you page
SELECT … ORDER BY created_at DESC LIMIT 50 OFFSET 50
```

Offset paging over a table that is being written to skips and repeats rows. Use a
cursor that carries **the whole ordering key**, exactly what `ORDER BY` compares:

```text
ORDER BY position, created_at, id
cursor = "<position>|<created_at>|<id>"
next_cursor returned only when the page was full
```

`id` at the end is not decoration: without a unique final tiebreaker, two rows
with the same position and timestamp make the cursor ambiguous and the pager can
loop. Return `next_cursor` only for a full page, or every client will make one
extra empty request forever.

```bash
# the walk that proves it, in the tests
# seed five works, page with limit=2 → three pages, each item exactly once, in order
```

That test — not a single-page assertion — is what proves pagination is right.

### Fuzzy matching, and where it belongs

```text
q=title:salt         → exact/stemmed match, ranked, cheap
q=salt and iron      → full-text
q=salt and iorn      → fuzzy (edit distance) — ranked last, always
```

Fuzzy matching must never be the *only* path: it is expensive, and it produces
confident nonsense on short strings. Rank it below exact matches, cap its result
count, and never let it satisfy an exact field filter.

### `library_items` and its nullable `work_id`

An imported row has no work: it is a book the reader read elsewhere. A row that
came from a local work has a `work_id`. **Never** join library rows to works with
an inner join and call the result "the library" — half the rows vanish. This is
the same shape as the import rule in Part 7: the honest model is "a library row
that may point at a work", and every aggregate has to tolerate the null.

## 7. Tests

`milestone_8.rs` (library):

- a shelf is private to its owner; another account gets 404 for its id;
- setting a reading status twice is idempotent;
- a batch removal reports per-id outcomes rather than failing wholesale;
- a saved view round-trips and its stored query re-parses.

`milestone_9.rs` (taxonomy):

- tags are normalised (case, whitespace, punctuation) before storage;
- two spellings of a tag resolve to one tag;
- a mood filter returns only works tagged with that mood.

`milestone_10.rs` (search):

- each operator (rating, words, mood, tag, status) filters correctly;
- an unknown operator is a 400;
- `rating:4..5` is inclusive at both ends — off-by-one here is invisible until
  someone compares two pages;
- a two-page cursor walk returns each work exactly once;
- `limit=0` and a malformed cursor are refused;
- a signed-out reader never sees a draft in any result.

## 8. Expected UI behaviour

- Library filters are shareable URLs.
- "Saved views" appears in the sidebar with the reader's own names.
- Search shows which filters are active, not just a result list.
- Paging keeps scroll position and never repeats a row.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| A row appears on two pages | offset paging over a written table, or a non-unique cursor | cursor with the full ordering key |
| "My library is empty" | inner join to works | left join; imported rows have no work |
| `q=ratng:4` returns everything | unknown field ignored | 400 on unknown operators |
| Mood filter is slow | filtering in Rust after fetching | filter in SQL, with the mood table indexed |
| Two tags that look identical | normalisation applied on write only once, or not at all | normalise in one function used by every write path |
| Drafts leak into results | the visibility rule was applied in the detail door only | apply it in the query |

## 10. Consequences

- **A library is a reading history, and reading history is sensitive** (Part 4).
  Every library endpoint is `RequirePseud`, and none of them may be served by
  anyone but the owner — including an administrator, unless the tool is audited.
- **Tags are public and therefore a moderation surface.** Plan for tag abuse
  (slurs, spam) before you open tagging to everyone.
- **A saved view can encode a private query.** Do not put the raw query in a URL
  that is shared publicly, and never let a view be executed as another account.

## 11. Checkpoint

```bash
git tag v0.11-search
```

Verified by `milestone_8.rs`, `milestone_9.rs`, `milestone_10.rs` on both
dialects, plus a browser pass: import a shelf, shelve a few rows, save a view,
and page through search results twice.
