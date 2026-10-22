# ADR 0029: A tag assertion carries five provenances, and provenance is not a request field

- Status: accepted
- Date: 2026-10-22
- Amends: [ADR 0026](0026-search-stays-on-sql.md) (§2, "Reader-applied tags:
  PARTLY — confidence yes, gravity no")
- Spec: §6.4 (`work_tag`), §2.2 (the theme overlap rule), §49.2 (gravity)
- Milestone: M6-04 (`T-M6-04` in the implementation plan §12)

## Context

ADR 0026 settled that a reader may *vote* on a tag but may not *apply* one, and
enforced it the cheapest possible way:

> `work_tags.source` remains constrained to `'author'` in the database, so "a
> reader tag is never a gravity input" is an invariant rather than a convention
> every write path must remember.

That constraint was a `CHECK` on PostgreSQL and a `BEFORE INSERT` trigger on
SQLite (migration 0103), and it made the invariant structural: there was one
legal provenance, so no write could name a worse one.

Spec §6.4 now fixes the vocabulary of a **tag assertion** — "a work's claim
about an entity, with provenance and confidence" — at five values: `author`,
`curator`, `imported`, `exchanged`, `inferred`. One value is no longer the
vocabulary, and the constraint that refused the other four has to go. The
question this ADR answers is what protects the invariant that constraint was
protecting, once it is gone.

## Decision

### 1. The invariant moves into the vocabulary

The five values contain **no `reader`**. A reader's contribution is a row in
`work_tag_votes` — one per (work, tag, pseud), `CHECK`ed to `-1 | 0 | 1` — and
the only column a vote is allowed to feed is `work_tags.confidence`, which is
recomputed from the votes and from nothing else
(`taxonomy::recompute_confidence`).

So ADR 0026's sentence is still true, and now in a stronger form: it is not
that a reader-sourced row is refused by a constraint, it is that **there is no
value a reader-sourced row could carry**. The `CHECK` was one guard; the
vocabulary is the guard, and it is legible in the schema rather than in a
trigger.

Consequence, stated plainly: `work_tags.source` is now a column on which
gravity *could* be keyed and is not. Nothing in the ranking path reads it.
§49.2's bar — "reader- or wrangler-confirmed" — is expressed by `confirmation`,
which is a different column with a different job.

### 2. The card's column names are swapped against the estate's, and the estate's are kept

The card (`T-M6-04`) asks for `role ∈ primary·secondary·background` **and**
`prominence INTEGER`. The estate already had both meanings under the other
names:

| the estate had | what it meant | after 0126 |
| --- | --- | --- |
| `prominence TEXT` (`primary·secondary·background`, since 0103) | the specification's **role** | `role` |
| `weight INTEGER` (since 0011) | the specification's **prominence** | `prominence` |

So migration 0126 renames rather than adds. Nothing is copied, nothing is
rewritten: the categorical value keeps its value and the number keeps its
number. `confidence` was already there and is unchanged.

The rename also repaired a live bug, which is the strongest evidence the names
were swapped rather than merely different: `revert_wrangling` wrote the tag's
number into the *categorical* column on SQLite (`INSERT OR REPLACE … prominence`
with an integer, since 0103 made `prominence` TEXT) and into `weight` on
PostgreSQL. The two arms disagreed, and one of them put `40` where
`primary|secondary|background` belongs. After the rename both arms write the
number to the number.

### 3. The table is not rebuilt

A `CHECK` whose vocabulary changes usually means a table rebuild, and the
estate's own notes warn about *"a 12-step table rebuild per table"*. This one
does not need it: SQLite renames a column in place (3.25 and later; the bundled
engine is 3.46), and on SQLite `source`'s constraint is a trigger, which is
dropped and recreated. PostgreSQL renames the column, renames the constraint
whose name had gone stale (`work_tags_prominence_ck` → `work_tags_role_ck`) and
re-adds the `CHECK`.

Migration 0103 already refused to rebuild this table *"for a constraint the
store already guarantees"*, and `work_tags` is the largest table in the schema.
A rebuild was attempted first and is recorded here because it failed loudly: it
declared a `node_id REFERENCES canonical_entities` foreign key that `work_tags`
never had, and the per-file dialect-parity test caught the two twins declaring
different tables for one file. The migration now declares no table at all, which
is what a rename should declare.

### 4. The defaults stay the estate's

The card asks for `source DEFAULT 'imported'` and `role DEFAULT 'primary'`.
Both are refused, for the reason migration 0099 gives about `confirmation`:
**a default must not manufacture a claim.** A tag is not the work's headline
description until someone says it is, and an unlabelled row is not an import
until an importer says so.

`role` therefore keeps `DEFAULT 'secondary'`. `source` keeps `DEFAULT 'author'`,
which is the weaker of the two arguments: it is the estate's default, and it is
a *claim*, but it is unreachable — every write path in the tree passes a source
explicitly (`imports.rs` → `imported`, the work-creation and inheritance paths →
`author`, the tagging route → derived, see §5), so the default only fires for a
writer that does not exist. Changing either default needs an SQLite table
rebuild, and 0103 refused that; the guarantee is the store, and it now
guarantees more.

### 5. Provenance is derived from the code path, never accepted from the request

The route `POST /works/{id}/tags` previously took `source` from the body. That
made the column a claim about a claim: a session could post
`{"source": "imported"}` and launder a person's assertion into the importer's,
or `{"source": "curator"}` and dress it in authority it does not hold.

The body no longer carries `source`. The route derives it:

- the acting pseud is a contributor of the work → `author`;
- the account holds an active curator role
  (`media_resilience::is_active_curator`) → `curator`;
- otherwise → **`403`, and no row**, because there is no provenance value that
  would describe the claim honestly, and ADR 0026 says a reader's channel is a
  vote rather than a tag.

The door is therefore **pseudonymous** (`RequirePseud`) rather than merely
authenticated: a claim with nobody to attribute it to is not a claim, and the
route-inventory table says so.

The role *is* still a request field: it is a strength, not a provenance, and an
author legitimately knows whether a tag is incidental. Its default in the route
is `secondary` — the modest one — because a bare tag is not the work's headline
description until someone says it is.

Consequence for a test that predates this: `milestone_10`'s
`search_fielded_fandom_uses_exists` tagged another account's work over HTTP and
now tags it as its author. That is not a test being bent to fit an
implementation; it is the same test expressing the rule the endpoint now has.

### 6. Five values, three writers, and that is honest

`author` and `imported` have writers today. `curator` is written by the route
for a curator's own assertion. `exchanged` and `inferred` have **no writer**:
the metadata exchange imports with `imported`, and nothing infers a tag yet. The
vocabulary is wider than the writers because it is the specification's, and a
value that no code path produces is a gap to be filled by the card that needs
it, not a licence to reuse a neighbour's name.

## Alternatives considered

- **Keep one value and label the new ones elsewhere** (e.g. a join table of
  provenance records). Refused: `source` is the column every surface already
  reads, and §6.4 names it.
- **Widen the `CHECK` and keep the body field.** Refused: a caller-chosen
  provenance is not provenance. The estate has no user-facing way to say "I am
  the importer", so the only correct writer for `imported` is the importer.
- **Rebuild `work_tags` on SQLite.** Refused for the reason 0103 gives, and
  shown unnecessary: SQLite renames a column in place, and the source
  constraint there is a trigger.

## Consequences

- `work_tags(prominence, source, role, confidence)` now means what §6.4 says.
  `role` is the categorical strength; `prominence` is the number; `source` is
  provenance; `confidence` is the votes' share, smoothed, and never a headcount.
- The API's `tag` object names the assertion in the specification's words:
  `prominence` where it used to say `weight`, plus `source` and `role` — the
  assertion as the store wrote it, so a caller can show what it claimed rather
  than what it hoped it claimed.
- A merge that drops a duplicate tag records `prominence|source|role` beside the
  drop (`dedupe_tags`), and the revert restores all three. A revert that
  restored the number alone would relabel a curator's claim as the author's and
  demote its role — three quiet edits wearing the name of one undo.
- ADR 0026 §2's last paragraph ("`work_tags.source` remains constrained to
  `'author'`") is superseded; the rest of §2 stands.
