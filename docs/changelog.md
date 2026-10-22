# Changelog

The instance's breaking changes, in order. **Before v1**, so a breaking change
is a line here and not a version bump — but it is a *line*, and a breaking
change that nobody was told about is a breaking change that looks like a bug.

Each entry says what changed, what a client has to do, and which milestone and
requirement shipped it.

---

## M4 — a new work's discussion lives in its forum thread (M4-01, `DM4-01`)

Spec §13.2: **`thread_only` is the default for new works** — the work page
carries a typed-vote reaction bar, and all text discussion lives in the linked
forum thread. `comments_only` is the legacy shape, supported indefinitely.

### What changed

**A work created from now on is `thread_only`.** The work's page carries the
reaction bar, and `POST /api/v1/works/{id}/comments` is **refused** for it —
the refusal names the rule and points at the thread:

> This work's discussion lives in its forum thread; use the Discuss link.

**A client that posts comments has to opt in.** Set the work's
`discussion_mode` to `comments_only` (or `both`) to keep the comment surface.

### What did not change

**Existing works keep their own mode.** The mode is applied when the work is
created and never looked up at read time, so a later change of the instance's
default does not rewrite a work that already exists. The rows that predate
this change keep `comments_only` — which is what migration 0038 said they
should, and what its note said the instance default was for.

The instance's default is `forum.work_discussion_default`, and it is now
**live**: the creation reads it. It was configuration that read as adjustable
and was not.

---

## M4 — the canonical vocabulary is `unit` and `revision` (M4-02, `DM4-02`)

Spec §5 is normative: a **unit** is a part of a work — a chapter, a track, an
episode — and a **revision** is an append-only version of a unit's text.
`chapters` and `chapter_revisions` were lorehaven's words, and the three
projects' words for the same thing must not leak into the code (R-2).

### What changed in the API

**The response field `chapters` is now `units`.** Every work object the API
returns — the work's page, the work's own list, the author's list — carries
`units` where it used to carry `chapters`.

```jsonc
// before
{ "id": "…", "title": "…", "chapters": [ { "id": "…", "title": "…" } ] }

// after
{ "id": "…", "title": "…", "units":   [ { "id": "…", "title": "…" } ] }
```

**A client has to rename one field.** Nothing else about the request changes:
the ids, the order, the titles and the word counts are the same, and the
`current_revision_id` on a unit keeps its name.

### What did *not* change

**The URLs keep their shape.** `/api/v1/works/{id}/chapters` and
`/api/v1/chapters/{id}` are the same URLs they were. A URL is the most
stable public surface an instance has, and the *model* is what this change
renames.

The paths are the one place the source's word survives in the API, and it
survives on purpose: the URL is a contract with the clients that already
exist, and the model is a contract with the code.

### What changed in the database

Migration `0122` renames the tables, their columns and their indexes. **It
does not rewrite the data**: every row keeps its identity, its order and its
text. A rename is a change of vocabulary and not of content, and a migration
that rewrote content would be a migration with a data-loss story.

| before | after |
|---|---|
| `chapters` | `units` |
| `chapter_revisions` | `revisions` |
| `import_chapters` | `import_units` |
| `chapter_id` (eleven tables) | `unit_id` |
| `anchor_chapter_id` | `anchor_unit_id` |
| `chapter_key` | `unit_key` |
| `chapter_start`, `chapter_end` | `unit_start`, `unit_end` |
| `last_chapter` | `last_unit` |
| `source_chapter_key` | `source_unit_key` |
| `spoiler_scope_chapter` | `spoiler_scope_unit` |

`current_revision_id` keeps its name, and so do the two places where the
*source's* word is the right one: the metadata exchange's signal carries the
source's `chapter_count` (spec §15.2 lists "chapter count" among the fields a
signal may carry), and the source adapters keep the site's word — an adapter's
job is to translate it.

---

## M6 — a tag says where it came from, and the caller cannot say it (M6-04, `DM6-04`)

Spec §6.4: a **tag assertion** is *"a work's claim about an entity, with
provenance and confidence"*. `work_tags` already carried `source` and
`confidence`; it now carries the specification's whole vocabulary, under the
specification's names.

### What changed in the API

**`POST /api/v1/works/{id}/tags` no longer reads `source` from the body.** The
route derives it: the work's own author gets `author`, an active curator gets
`curator`, and anyone else is **refused** — the specification has no provenance
value for "a signed-in reader", and a reader's channel is a tag *vote*, which
feeds `confidence` and nothing else (ADR 0026, ADR 0029).

```jsonc
// before — the body chose its own provenance
{ "node_id": "…", "weight": 40, "source": "imported" }

// after — the body names a strength and a role; the code path names the source
{ "node_id": "…", "prominence": 40, "role": "primary" }
```

**The response's `tag` object names the assertion in the specification's
words**: `prominence` where it used to say `weight`, plus `source` and `role`.
A client that reads `tag.weight` has to read `tag.prominence`.

**And the door is pseudonymous, not merely authenticated**: a session with no
selected pseud has no one to attribute a claim to, and is refused.

### What did not change

**The URL.** `/api/v1/works/{id}/tags` is the same door.

**The rows.** Migration `0126` renames two columns and replaces one constraint.
**It does not rewrite the data**: the tag's number keeps its number and a row
that said `source = 'author'` still says it. Nothing is rebuilt — SQLite renames
a column in place, and the source's constraint there is a trigger.

| before | after |
|---|---|
| `prominence` (TEXT: `primary`·`secondary`·`background`) | `role` |
| `weight` (INTEGER) | `prominence` |
| `source` = `'author'` only | `author`·`curator`·`imported`·`exchanged`·`inferred` |

The two names were **swapped** against the card and against the specification:
the estate's `prominence` was a role and its `weight` was a prominence. The
rename also repaired a live bug — the merge-revert path wrote the number into
the categorical column on one dialect — and a merge's revert now restores a
dropped duplicate's provenance and role along with its prominence.
