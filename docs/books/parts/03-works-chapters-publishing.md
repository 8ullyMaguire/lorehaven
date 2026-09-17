# Part 3 — Works, chapters, revisions and publishing

Checkpoint: `v0.03-identity`

Everything in a fanfiction site is downstream of this part. If the work model is
wrong, four later parts inherit the mistake: comments anchor to chapters, the
reader remembers positions inside revisions, exports render a whole work, and the
positivity filter sees chapter text.

## 1. Checkpoint

```bash
git checkout v0.03-identity
```

## 2. What will work by the end

```bash
# Create a draft, add a chapter, publish it.
curl -X POST localhost:8080/api/v1/works -d '{"title":"Salt and Iron","kind":"fiction"}'
curl -X POST localhost:8080/api/v1/works/$WORK/chapters -d '{"title":"One"}'
curl -X PUT  localhost:8080/api/v1/chapters/$CH/revision -d '{"body":"…"}'
curl -X POST localhost:8080/api/v1/works/$WORK/publish

curl localhost:8080/api/v1/works/$WORK                 # the work, its chapters, its metadata
curl localhost:8080/api/v1/works/$WORK/revisions       # the revision history
```

A published work is readable by anyone, including signed-out readers, with the
chapters in reading order rather than upload order.

## 3. Concepts

- **A work is metadata; a chapter is a container; a revision is text.** Three
  levels, because that is what readers and writers actually change at different
  rates.
- **Drafts are private by construction.** Not "unlisted until published" — a
  draft has no published representation to leak.
- **Publishing is a transaction.** It either publishes the whole work or leaves
  it exactly as it was.
- **The revision cache is a cache, and it must be honest.** Reading a chapter
  must render the current revision, and a stale cache must never be served as if
  current.
- **Feedback preferences belong to the work, per chapter if needed.** A writer
  can ask for no critique on chapter one and everything on chapter twenty.

## 4. Commands

```bash
lorehaven migrate        # applies 0003_works and 0007_revision_cache
cargo test -p lorehaven-app --test milestone_3
cargo test -p lorehaven-app --test revision_cache
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0003_works.sql` | works, chapters, chapter revisions, tags |
| `migrations/sqlite/0007_revision_cache.sql` | the revision cache and its invalidation |
| `crates/domain/src/content.rs` | work states, chapter ordering, word counting |
| `crates/db/src/content.rs` | works, chapters, revisions |
| `crates/db/src/revisions.rs` | revision history and the cache |
| `crates/app/src/revisions.rs` | revision assembly for the API |
| `crates/app/src/routes/works.rs` | the work, chapter and publishing doors |
| `crates/app/src/routes/collaborators.rs` | co-authors and their permissions |
| `crates/app/tests/milestone_3.rs` | acceptance tests |
| `frontend/src/routes/Write.svelte` | the writer's dashboard |
| `frontend/src/routes/WorkEditor.svelte` | the editor (autosave lives here) |
| `frontend/src/routes/WorkPage.svelte` | the public work page |
| `frontend/src/lib/autosave.ts` | debounced draft saving |

## 6. The code that matters

### The three levels

```sql
works            id, owner_pseud_id, title, kind, lifecycle, language,
                 rating, created_at, updated_at
chapters         id, work_id, title, position, current_revision_id
chapter_revisions id, chapter_id, body, word_count, created_at, author_pseud_id
```

`current_revision_id` on the chapter is the single source of truth for "what a
reader sees". The revisions themselves are append-only. When you need "what did
this look like last week", you have it; when you need "what does it look like
now", you have one pointer and no ambiguity.

`position` is the reading order, and it is **not** the creation order. Writers
reorder chapters; importers append them out of order; a thread-scraped work
arrives back-to-front. Store an explicit position and sort by it — never by
timestamp.

### Publishing as a transaction

Publishing validates and then flips state:

```text
for each chapter: it must have a current revision
the work must have at least one chapter
the title must be non-empty
the work's feedback preferences must be readable
-- all of it inside one transaction
lifecycle = 'published', published_at = now
```

If any check fails, nothing changes: the writer keeps their draft, gets a field
error, and fixes one thing. A partial publish — some chapters public, some not —
is the kind of state you cannot get out of later.

### Draft visibility is the whole rule

The reading decision from Part 2 applies here and is worth restating as code you
should write deliberately:

```rust
// crates/app/src/routes/works.rs
match reading_decision(&actor, &work) {
    Reading::Allowed => { /* serve it */ }
    Reading::Denied  => return Err(AppError::NotFound), // 404, never 403
}
```

Only contributors (owner, co-authors with the right grant, admins acting through
an audited tool) may read a draft. Everyone else gets 404 — because a 403 on a
draft URL confirms the draft exists, which is itself information about a writer
who has not published anything yet.

### Autosave is a client concern, and a server contract

```ts
// frontend/src/lib/autosave.ts
// debounce edits, PUT the chapter revision, recover from a failed save
```

The server side of this must be boringly strict: a save is accepted only if it
carries the revision the client based it on. Otherwise you get the classic
silent data loss — two tabs, or a phone and a laptop, and the second save
overwrites the first. Return a conflict the client can resolve, and make the
client say so visibly instead of failing quietly.

### Word counts, once

`word_count` is stored on the revision, computed by one function in the domain
crate, and reused by: the work page, the reader, the export, the dashboard and
the search index. Compute it in the route and you will have five answers to the
same question, all slightly different.

## 7. Tests

`milestone_3.rs` asserts:

- a draft is 404 to a signed-out caller and to a different account;
- publishing is atomic: a work with an empty chapter fails, and the work is still
  a draft afterwards;
- chapters come back in `position` order, not insertion order;
- editing a chapter creates a new revision and moves `current_revision_id`;
- the word count matches the stored text;
- a save against a stale revision is refused with a conflict, not accepted.

`revision_cache.rs` asserts the cache is invalidated the moment a chapter is
edited — including the case where the edit happens while a read is in flight.

## 8. Expected UI behaviour

- The writer's dashboard lists drafts and published works with real counts.
- The editor autosaves, and shows a conflict when someone else's tab saved first.
- The public work page shows chapters in order, with word counts and the byline
  of the pseud, not the account.
- An unpublished work is not reachable from a link, a search, a feed, or an
  export.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| Chapters appear in the wrong order | ordering by `created_at` | order by `position` |
| A reader sees an old chapter after an edit | the cache was not invalidated on the write path | invalidate in the same transaction as the revision insert |
| Two saves clobber each other | no base revision on the write | require and check it |
| Word counts differ between pages | computed in more than one place | one domain function, stored on the revision |
| A published work 404s for a signed-out reader | the door requires a session | `MaybeSession` + the visibility rule |

## 10. Consequences

- **Append-only revisions are a privacy surface.** A revision that was published
  stays in history unless you delete it deliberately. If a writer edits out a
  real name, the old revision still holds it: decide now whether history is
  public, private, or available to the author only, and say so in the UI.
- **`position` is a contract.** Anything that renumbers positions (an import, a
  merge, a fork) must do it in one transaction, or readers will see duplicates
  and gaps.
- **Draft metadata is still metadata.** Titles, tags and summaries of drafts
  leak intent. Exclude drafts from search, discovery, feeds and statistics
  everywhere — Part 8 and Part 9 have to re-apply this rule.

## 11. Checkpoint

```bash
git tag v0.04-publishing
```

Verified by `milestone_3.rs` and `revision_cache.rs` on both dialects, plus a
browser pass: create a draft, write two chapters, reorder them, publish, open the
work in a signed-out browser session. Owed: the reader (Part 4) still has no
memory, and nothing yet tells the author whether anyone read it.
