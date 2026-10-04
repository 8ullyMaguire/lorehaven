# M45-28 — WIP adoption, closure notes, and lineage (spec §56)

**Status:** spec written (`docs/spec.md` §56, commit `c1837b3`). Not yet implemented. This
plan is executable by an LLM with no other context.

## What this row is

`docs/requirements.csv` M45-28:

> WIP adoption and closure notes with lineage
> Notes: Gaps review C4. Mark abandoned WIPs up-for-adoption; publish planned endings;
> credit closure notes.

## What the spec already settles, and what it costs to get wrong

Read `docs/spec.md` §56 before designing anything. Three clauses, and two of them are
**prohibitions**, which is the hard part: an implementation that adds a prohibition
correctly is mostly an implementation that does not add the thing.

- **§56.1 ownership never moves.** `works.owner_pseud_id` is unchanged by adoption,
  permanently. The adopting pseud becomes a *contributor*.
- **§56.2 a closure note is optional and never a precondition.** The obvious failure is a
  conclude flow that refuses until the author writes a note, turning a courtesy into a tax.
- **§56.3 lineage is derived, never hand-written.** There is no lineage column for anyone
  to fill in.

### The schema already carries most of this

This is the finding that decides the shape of the work, and it is the same kind of finding
`docs/plans/m45-23-north-star.md` opened with: **the prerequisite is already built.**

- `works.completion` already carries `in_progress | complete | hiatus | abandoned` (§8.2,
  `migrations/sqlite/0003_works.sql:48`). `abandoned` is what §56.1 gates on, and it
  exists.
- `chapter_revisions.created_by_pseud_id` (`0003_works.sql:114`) already records who wrote
  each revision. **That column is the entire lineage source**, which is why §56.3 can say
  lineage is derived: there is nothing to add and therefore nothing to lie in.
- `works.owner_pseud_id` belongs to the **pseud**, not the account (ADR 0003), so §56.1's
  "ownership does not move" follows from the schema rather than being bolted on.

So the migration is two tables and no column changes to `works` at all.

## Step 1 — migration 0116, both dialects

Two new tables. Neither touches `works`, which is the point: §56.1 is enforced by the
*absence* of a path that could move ownership, not by a constraint that could be dropped.

```sql
-- adoption_offers
CREATE TABLE adoption_offers (
    id              TEXT/UUUID  PRIMARY KEY,
    work_id         TEXT/UUUID  NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    offered_by_pseud_id TEXT/UUUID NOT NULL REFERENCES pseuds(id),
    -- The author who stopped. Kept alongside `work_id` so withdrawing an offer is a
    -- state change on the offer rather than a permission check against the work's
    -- current owner -- see "Why both pseuds" below.
    author_pseud_id TEXT/UUUID  NOT NULL REFERENCES pseuds(id),
    -- NULL means "anyone at the publishing trust level may adopt".
    adopted_by_pseud_id TEXT/UUUID REFERENCES pseuds(id),
    status          TEXT        NOT NULL DEFAULT 'open',  -- open | withdrawn | adopted
    created_at      TEXT/TIMESTAMPTZ NOT NULL,
    updated_at      TEXT/TIMESTAMPTZ NOT NULL
);
CREATE INDEX adoption_offers_work ON adoption_offers (work_id, status);
CREATE INDEX adoption_offers_open ON adoption_offers (status, created_at);

-- closure_notes
CREATE TABLE closure_notes (
    id              TEXT/UUUID  PRIMARY KEY,
    work_id         TEXT/UUUID  NOT NULL REFERENCES works(id) ON DELETE CASCADE,
    -- §56.2: the note is versioned WITH THE WORK. One row per revision number, like
    -- chapter_revisions, so restoring an old note is a normal revision restore.
    revision_number  INTEGER     NOT NULL,
    body            TEXT        NOT NULL,
    author_pseud_id TEXT/UUUID  NOT NULL REFERENCES pseuds(id),
    created_at      TEXT/TIMESTAMPTZ NOT NULL
);
CREATE UNIQUE INDEX closure_notes_revision ON closure_notes (work_id, revision_number);
```

**Why both pseuds on `adoption_offers`.** `offered_by_pseud_id` and `author_pseud_id` are
usually the same row, and storing both looks redundant. It is not: the offer is a *record of
a past act*, and §56.1 says the author may withdraw it later. If only `work_id` were stored,
withdrawal would require re-reading the work's current owner, which changes the meaning of
an existing record when a pseud is transferred between accounts. Storing both makes the
offer immutable evidence of who offered what, and `author_pseud_id` is the authority for
withdrawal.

The unique index on `(work_id, revision_number)` is what makes §56.2's "each edit is a new
revision" a database guarantee rather than a service-level hope.

## Step 2 — domain types

**File:** `crates/domain/src/wip_adoption.rs` (new)

```rust
pub enum AdoptionStatus { Open, Withdrawn, Adopted }

pub struct AdoptionOffer {
    pub id: String,
    pub work_id: String,
    pub offered_by_pseud_id: String,
    pub author_pseud_id: String,
    pub adopted_by_pseud_id: Option<String>,
    pub status: AdoptionStatus,
    pub created_at: String,
    pub updated_at: String,
}

pub struct Contributor {
    pub pseud_id: String,
    pub handle: String,
    /// §56.3: FIRST contribution, so an adopter does not leapfrog chapter one's author.
    pub first_revision_at: String,
    pub revisions: i64,
}

pub struct ClosureNote {
    pub work_id: String,
    pub revision_number: i64,
    pub body: String,
    pub author_pseud_id: String,
    pub created_at: String,
}
```

`Contributor::first_revision_at` is the field that makes §56.3's ordering rule a property
of the type rather than of a `ORDER BY` someone might get wrong. The ordering is
**ascending** on it — deliberately the opposite of a "most active first" list, because
"most active first" is how an adopter of chapter nine outranks the author of chapter one.

## Step 3 — the store

**File:** `crates/db/src/wip_adoption.rs` (new)

| function | the rule it enforces |
|---|---|
| `open_offer(db, work_id, author_pseud)` | author must own the work; `completion = 'abandoned'` or `'hiatus'` |
| `withdraw_offer(db, offer_id, author_pseud)` | §56.1: only `author_pseud_id` may withdraw, with no notice period |
| `adopt(db, offer_id, adopter_pseud)` | §56.1: trust level ≥ publishing; §56.1: `completion != 'complete'` |
| `list_open_offers(db, limit)` | only `status = 'open'` |
| `contributors(db, work_id)` | §56.3: `ORDER BY first_revision_at ASC` |
| `put_closure_note(db, work_id, author_pseud, body)` | §56.2: next revision number |
| `latest_closure_note(db, work_id)` | §56.6: `MAX(revision_number)` |

**`adopt()` must not write `works.owner_pseud_id`.** That is the whole clause. There is no
update to `works` anywhere in this module, and the acceptance test asserts the column value
before and after rather than asserting an outcome.

**`put_closure_note()` must not require a body to be non-empty at the conclude path.**
The conclude path calls it *optionally*. Write the function so an absent note is a `None`
the caller chose, not an error the caller has to catch.

### Two SQL hazards, both already paid for once this repository

- **`created_at` on `adoption_offers` and `closure_notes` is TEXT on SQLite and TIMESTAMPTZ
  on PostgreSQL**, following whichever the nearest existing migration used. Read the
  neighbouring migration before writing the bind; a bare `$1` against a TIMESTAMPTZ column
  is `42883`, and `docs/plans/REMAINING-2026-10-03.md` records three of those.
- **`contributors()` groups by `created_by_pseud_id` across `chapter_revisions`.** If it
  also reads a correlated subquery over an ungrouped column, PostgreSQL returns `42803`
  and SQLite returns the right answer. That exact bug is in this repository's history; see
  the `north_star` store for the shape and the fix.

## Step 4 — routes

```
POST /works/:id/adoption-offer          open an offer (work's owner pseud)
DELETE /works/:id/adoption-offer        withdraw it (no notice period)
GET /adoption-offers                    list open offers
POST /adoption-offers/:id/adopt         adopt (trust level ≥ publishing)
GET /works/:id/contributors             lineage, §56.3 ordering
PUT /works/:id/closure-note             create or edit, versioned
GET /works/:id/closure-note             latest
```

`DELETE` for withdrawal is right: withdrawal is a reversal of the author's own act, and
`DELETE` says that. It is a soft status change, not a row deletion — the offer row stays.

`RouteClass::Write` for all five mutations, `Default` for the three reads.

## Step 5 — tests

**Files:** `crates/app/tests/wip_adoption.rs`, `crates/app/tests/closure_notes.rs`

Cases that will not pass by accident, each paired with the implementation it rules out:

| # | case | rules out |
|---|---|---|
| 1 | adopting does not change `works.owner_pseud_id` — assert the column **before and after** | any implementation that transfers ownership |
| 2 | an author can withdraw an open offer; afterwards it is not in `list_open_offers` | "withdrawal is a no-op" |
| 3 | adoption below the publishing trust level is refused, **and the refusal names the bar** | a bare 403 |
| 4 | `completion = 'complete'` cannot be adopted | a missing guard |
| 5 | a work concludes with **no** closure note and the conclude path succeeds | the courtesy-as-a-tax failure |
| 6 | a closure note's author is shown and it earns credit equal to a chapter | treating the note as metadata |
| 7 | editing a note creates a new revision; the old one is restorable | overwriting |
| 8 | contributor order is **first contribution ascending**, with an adopter joining after chapter one | `ORDER BY revisions DESC` |
| 9 | two works adopted by the same pseud rank identically in search and recommendations | lineage leaking into ranking |
| 10 | an unaliased subquery would be caught — see "the gate" below | nothing; belt and braces |

Cases 1 and 9 are the two that matter. Case 1 is the promise §56.1 makes, and asserting it
on the *column* rather than on a returned struct is what makes it able to fail. Case 9 is
the only way to test §56.3's "lineage never affects ranking": you cannot assert an absence
of effect on one work.

## The gate

`scripts/check-pg-subquery-alias.py` is the gate that would catch the `contributors()`
hazard, and it is already wired into CI with its self-test. Run it:

```
python3 scripts/check-pg-subquery-alias.py crates
python3 scripts/check-pg-subquery-alias.py --self-test
```

Expected: `OK: every FROM subquery is aliased` and `10 cases passed`. A finding in
`wip_adoption.rs` means the `contributors()` subquery needs an alias, and the PostgreSQL
test run is what proves it.

## What this deliberately does not do

- **No automatic abandonment.** Nothing marks a work `abandoned` without a human. §8.8's
  derived `dormant` is a reading, not a state change.
- **No ownership transfer, ever.** There is no route that can move `owner_pseud_id`.
- **No compensation for adopters.** §0.3 and the standing rule against bought trust.
- **No lineage ranking.** §56.3.

## Verification

Both engines, per-suite, not once for the workspace:

```
# SQLite (default — LOREHAVEN_TEST_PG_URL unset)
cargo test -p lorehaven-app --test wip_adoption --test closure_notes
cargo test -p lorehaven-db --test migration_catalogue

# PostgreSQL
sudo docker start lh-pg-test
LOREHAVEN_TEST_PG_URL='postgres://lorehaven:lorehaven@127.0.0.1:55433/postgres' \
  cargo test -p lorehaven-app --test wip_adoption --test closure_notes
unset LOREHAVEN_TEST_PG_URL

cargo fmt --all -- --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
python3 scripts/check-pg-subquery-alias.py crates
python3 scripts/check-uncast-pg-placeholders.py crates
python3 scripts/check-pg-uuid-casts.py
```

Then prove each of cases 1, 5 and 8 by mutation — and **confirm the mutated build actually
ran**. Three of four mutation attempts in the previous pass failed to compile and proved
nothing; see the record in `docs/plans/REMAINING-2026-10-03.md`.