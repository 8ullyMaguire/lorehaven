# Part 9 — Discovery, exports and offline reading

Checkpoint: `v0.11-search`

Two halves of the same idea: get new things in front of a reader, and let them
take what they have away with them.

## 1. Checkpoint

```bash
git checkout v0.11-search
```

## 2. What will work by the end

```bash
curl 'localhost:8080/api/v1/discover'                    # a real front page
curl 'localhost:8080/api/v1/discover/recipes'            # the recipes behind it
curl -X POST localhost:8080/api/v1/exports -d '{"work":"…","format":"epub"}'
curl localhost:8080/api/v1/jobs/$JOB                     # the export, built by the worker
curl localhost:8080/api/v1/exports/$EXPORT/download
```

And in a browser: load the site, go offline, keep reading the work you had open,
then reopen the tab and see your progress reconciled with the server.

## 3. Concepts

- **Discovery is a query with a purpose, not a set of hand-picked shelves.** A
  "recipe" is a named, explainable rule ("finished works over 20k words tagged
  hurt/comfort that you have not opened").
- **Every recommendation must be explainable in one sentence.** If you cannot
  say why a work is on the page, it should not be on the page.
- **Taste influence is derived, private and inspectable.** A reader must be able
  to see what the instance thinks they like, and turn it off.
- **Exports are jobs.** An EPUB of a 400k-word work takes long enough to time out
  a request, and long enough that a client will retry — hence Part 5.
- **Offline is a first-class state, not an error state.** The reader must work
  with no network and reconcile honestly when it comes back.

## 4. Commands

```bash
lorehaven migrate        # applies 0008_exports and 0012_discovery
cargo test -p lorehaven-app --test milestone_11
cargo test -p lorehaven-domain exports
```

## 5. Exact file changes

| Path | What it is |
|---|---|
| `migrations/sqlite/0008_exports.sql` | export rows, formats, delivery |
| `migrations/sqlite/0012_discovery.sql` | recipes, taste signals, dashboards |
| `crates/domain/src/discovery.rs` | recipe evaluation, ranking, explanations |
| `crates/domain/src/exports.rs` | the export document model |
| `crates/domain/src/exports/epub.rs` | the EPUB builder |
| `crates/db/src/discovery.rs`, `db/exports.rs` | storage |
| `crates/app/src/exports.rs` | the export job handler |
| `crates/app/src/routes/discovery.rs`, `routes/exports.rs` | the doors |
| `frontend/src/routes/Discover.svelte`, `Exports.svelte` | the pages |
| `frontend/src/lib/offline.ts` | the offline queue and reconciliation |

## 6. The code that matters

### A recipe is a function with a name

```text
recipe "continue what you started":
  a work with reading progress < 100%
  and at least one chapter read
  and not finished
order by  updated_at desc
explain  "you read chapter 4 of 12 two days ago"
```

The `explain` clause is not decoration — it is the acceptance criterion. Write
the explanation function next to the query, and assert on it in tests.

### Taste signals

Signals are cheap, private rows: `(pseud_id, kind, subject, weight, at)`. They
come from things the reader already did — finished a work, rated it highly, read
three chapters of another. Two rules:

- **Never include a signal derived from a blocked party's content.**
- **Expose them.** `GET /api/v1/me/taste` (or the settings page) must show what
  the instance has inferred, with a way to clear it. A recommendation system the
  user cannot inspect is a surveillance system with a nicer font.

### The EPUB builder

```rust
// crates/domain/src/exports/epub.rs
// mimetype (stored first, uncompressed) → META-INF/container.xml →
// OEBPS/: content.opf, nav.xhtml, chapters, style.css
```

Details that will cost you an afternoon each if you get them wrong:

- `mimetype` must be the first entry and stored uncompressed, or readers refuse
  the file.
- Chapter order in `content.opf` must match the `spine` order; the nav document
  must list the same order.
- Escape everything: a chapter title containing `&` breaks XML, and the failure
  appears only in the reader app, not in your tests — unless you test with a
  hostile title.
- Attribution goes in the document: the author's pseud and the source URL for an
  imported work. An export that strips attribution is a licence problem.

Test the builder by asserting the ZIP entry order, the OPF spine and the presence
of each chapter's text — not by eyeballing a file in Calibre.

### Offline reading

```ts
// frontend/src/lib/offline.ts
// 1. cache the work's chapters when reading starts
// 2. queue progress writes locally when the network fails
// 3. on reconnect, reconcile: newest timestamp wins, and say so
```

The reconciliation rule has to be written down, because "merge" means nothing on
its own. Pick newest-wins for progress, never-wins for anything destructive, and
show the reader a line like "Synced — your offline progress was kept" rather than
silently choosing.

## 7. Tests

`milestone_11.rs` and the discovery/export tests assert:

- every recipe returns only published, readable works for the calling reader;
- a blocked work never appears in discovery for the blocker;
- the explanation for each recommended work is non-empty and derived from the
  query, not from a template that is always the same string;
- clearing taste signals changes the next discovery response;
- an EPUB has `mimetype` first and uncompressed, a spine matching the nav, and
  every chapter's text present;
- an export of a work the caller may not read is 404;
- an export job for a work edited mid-export either completes against a snapshot
  or fails cleanly — it never produces a file with missing chapters;
- offline progress queued while offline is applied once, not twice, on reconnect.

## 8. Expected UI behaviour

- The front page changes for a signed-in reader and is honest for a signed-out
  one (popular published works, not a personal feed that cannot exist).
- Every recommended work has a one-line reason.
- An export request shows a job, then a download, and the file opens in a real
  reader.
- Losing the network mid-chapter shows a quiet offline indicator, and reading
  continues.

## 9. Troubleshooting

| Symptom | Cause | Fix |
|---|---|---|
| EPUB rejected by a reader app | `mimetype` not first or compressed | write it first, stored |
| Missing chapters in an export | the work was edited during the job | snapshot the revision ids at job start |
| Discovery empty for a new reader | recipes all depend on history | include a cold-start recipe over published works |
| Recommendations repeat forever | no exclusion of what was already opened | filter on read history |
| Duplicate offline progress | the queue is applied on every reconnect | clear the queue after a successful write, keyed by an idempotency token |

## 10. Consequences

- **Recommendations can out someone.** "Readers who liked X also liked Y" on a
  site with sensitive content can reveal what someone reads. Never surface a
  person in a recommendation the reader did not publish themselves.
- **An exported EPUB leaves your instance forever.** It carries the work and its
  attribution, and it cannot be recalled — which is a feature for the reader and
  a reason to be careful about what goes into it.
- **Offline caches hold content on a device you do not control.** Say so, and
  give the reader a way to clear it.

## 11. Checkpoint

```bash
git tag v0.12-discovery
```

Verified by the milestone tests plus a manual pass: build an EPUB of a large
work, open it in a real reader app, then read offline and reconcile.
