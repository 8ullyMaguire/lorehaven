# Part 4 — The reader: ratings, reviews, notes, history, goals

Checkpoint: `v0.04-publishing`

A site that publishes but does not remember is a blog. This part gives the
reader a memory: where they are in a work, what they thought of it, what they
noticed, and what they meant to read next.

## 1. Checkpoint

```bash
git checkout v0.04-publishing
```

## 2. What will work by the end

```bash
curl -X PUT localhost:8080/api/v1/works/$WORK/rating -d '{"stars":4,"is_public":true}'
curl -X POST localhost:8080/api/v1/works/$WORK/reviews -d '{"body":"…","is_public":true}'
curl -X PUT localhost:8080/api/v1/chapters/$CH/progress -d '{"position":0.42}'
curl localhost:8080/api/v1/me/history
curl localhost:8080/api/v1/me/goals
```

And in a browser: read a story, close the tab, come back and be offered the
chapter you stopped in the middle of.

## 3. Concepts

- **Ratings and reviews are different kinds of fact.** A rating is a number the
  reader can change or withdraw; a review is text with a publication state.
- **Progress is not history.** "Where I am" is current state; "what I read" is a
  log. Storing one as the other loses information you will want.
- **Private by default.** Notes and reading history belong to the reader. Making
  something public is an explicit act.
- **Goals are a promise to yourself**, and the honest way to render them is
  against real data, never a streak you cannot compute from the log.
- **Reactions are cheap and must stay cheap.** A quick reaction must not create
  a notification storm or an unbounded row per interaction.

## 4. Commands

```bash
lorehaven migrate        # applies 0004_reading
cargo test -p lorehaven-app --test milestone_4
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0004_reading.sql` | ratings, reviews, progress, history, notes, goals |
| `migrations/postgres/0004_reading.sql` | the same, PostgreSQL |
| `crates/domain/src/reading.rs` | progress maths, rating rules, goal arithmetic |
| `crates/db/src/reading.rs` | the reader's tables |
| `crates/app/src/routes/reading.rs` | the reader's doors |
| `crates/app/tests/milestone_4.rs` | acceptance tests |
| `frontend/src/routes/Reader.svelte` | the reader itself |
| `frontend/src/routes/History.svelte` | what I have read |
| `frontend/src/lib/reading.ts` | progress restore and local-first resume |

## 6. The code that matters

### Tables

```sql
rating          (pseud_id, work_id, stars, is_public, deleted_at)
review          (pseud_id, work_id, body, is_public, published_at, deleted_at)
reading_progress(pseud_id, work_id, chapter_id, position, updated_at)
read_history    (pseud_id, work_id, chapter_id, at)
note            (pseud_id, chapter_id, anchor_kind, anchor_value, body)
goal            (pseud_id, year, target, kind)
```

Two things to notice:

- **`deleted_at`, not `DELETE`.** A withdrawn rating must stop counting towards
  the average, and must be restorable. Soft deletion with a filter on every read
  is the cheap way to get both. Every aggregate must carry `deleted_at IS NULL` —
  this is exactly the kind of clause that gets forgotten in one of five queries
  and produces an average that disagrees with the count.
- **Progress is per work *and* per chapter.** A reader can be on chapter nine
  while only having finished it halfway. Store the unit you resume from.

### The public/private line

```text
rating.is_public = false   → counts for the reader, not for the author's average
review.is_public = false   → a private note-to-self, not a review
history and notes          → never public, never in an export addressed to anyone else
```

When you later build the creator dashboard (Part 6), the author's view counts
only public ratings and only published reviews. That is not a detail — a writer
seeing a private "I gave this two stars" is a privacy breach, not a feature.

### Resuming a read

Resolution order, and stop at the first hit:

1. server-side `reading_progress` for this pseud and work;
2. a local copy in the browser (for signed-out readers, and for offline reading
   in Part 9);
3. the start of the first chapter.

Write it in one function used by both the reader and the resume prompt, or the
two will disagree and the reader will be offered chapter three while sitting on
chapter nine.

### Reactions

Quick reactions (a heart, a bookmark, a "more like this") are stored as one row
per reader per target per kind, and the counts are aggregated on read. Do not
store a counter column you increment: you will need to subtract on withdrawal,
and a double-submit will silently corrupt it.

## 7. Tests

`milestone_4.rs` asserts:

- rating a work twice updates rather than duplicates;
- withdrawing a rating removes it from the average but keeps the row (restorable);
- a private rating does not appear in the work's public average;
- progress is stored and returned per chapter, and the resume point survives a
  chapter edit (the chapter exists, the revision changed);
- a private note never appears in a response to anyone else;
- a goal's progress is computed from the history log, not from a stored counter.

```bash
cargo test -p lorehaven-app --test milestone_4
```

## 8. Expected UI behaviour

- Opening a work you have read offers "continue in chapter nine" — and does not
  offer it if you finished.
- Ratings are editable and removable.
- Reviews show their own state: draft, published, or private.
- History is a real log with dates, not a synthetic streak.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| The average disagrees with the count | one query forgot `deleted_at IS NULL` | filter in every aggregate; test the pair together |
| Resume always restarts at chapter one | progress keyed on the revision, not the chapter | key on chapter id |
| History grows without bound | every page turn is a row | record a read once per chapter per session |
| A private rating shows on the author's dashboard | the dashboard query does not filter `is_public` | filter, and test it from the author's account |

## 10. Consequences

- **Reading history is the most sensitive data in the application.** It reveals
  what someone reads, when, and how far. It must be excluded from every export
  addressed to anyone but the reader, from every webhook payload, and from every
  administrator tool that is not audited.
- **Notes can contain anything.** Treat note bodies as user content subject to
  the same moderation surfaces as comments (Part 10) — or decide now that they
  are strictly private and never reportable, and enforce that they never surface
  in a report queue.

## 11. Checkpoint

```bash
git tag v0.05-reader
```

Verified by `milestone_4.rs` on both dialects plus a browser pass: read across
three chapters, reload, resume, rate, write a private note, and confirm none of
it shows to another account.
