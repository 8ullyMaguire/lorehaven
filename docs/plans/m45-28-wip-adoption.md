# M45-28 — WIP adoption, closure notes, lineage (§56)

**Status:** rewritten 2026-11-02 against measurement. The 2026-10-04 draft is kept as
`docs/plans/m45-28-wip-adoption-draft-2026-10-04.md`; it proposes migration `0116`, which is now
`0116_account_darkhold_columns.sql`. Migration `0137`.

## Premises, measured 2026-11-02

| claim | measurement | verdict |
|---|---|---|
| §56 exists with three clauses | `docs/spec.md` 10548–10617 (`56.1`–`56.4` + Acceptance) | holds |
| `works.completion` carries `abandoned` | live `darkhold`: **58,042** works — `in_progress` 42,531, `abandoned` **15,220**, `complete` 291 | holds, and there is real supply |
| `chapter_revisions.created_by_pseud_id` is the lineage source | **the names are old**: `0122` renamed `chapters`→`units` and `chapter_revisions`→`revisions` (§5), and the live instance has **127,474** rows in each | holds, under the current names |
| lineage is unreadable live | **wrong, and corrected**: the earlier probe looked for `chapter_revisions`/`chapters`, which 0122 renamed. Live: `revisions` 127,474, `units` 127,474, `created_by_pseud_id` non-null in all of them, **3** distinct authors | **lineage reads live**, and the `available: false` branch stays for schemas that lack the tables |
| something writes revisions | `grep -rn chapter_revisions crates/` finds no writer, but the table arrives **populated** (an import) | the read path has supply; the write path is elsewhere |
| a publishing trust level exists to gate adoption | no publish route, and no canonical level: `trust_levels` is `(account, level, computed_at, basis)` with no seed; `Tier` is a *reading* tier (`House/Personal/Full`) | must be **configured**, not inferred |
| nothing moves `owner_pseud_id` | no `UPDATE works SET owner_pseud_id` anywhere | §56.1 is a prohibition the schema already keeps |

Two consequences decide the design:

1. **Lineage answers in words when it cannot read.** `contributors()` reads `revisions` joined to
   `units`; on a schema without them the door answers `200 {contributors: [], available: false,
   reason: "…"}`, not a `500` from a missing relation — the refusal shape §58.2 uses for a drought
   that cannot be measured. **Corrected 2026-11-02 mid-implementation:** the first draft of this
   plan asserted the tables were missing live, because the probe asked for `chapter_revisions` and
   `chapters`. `0122` renamed them; the live instance has 127,474 of each and lineage works. The
   guard stayed because a guard that is not needed today is the only kind that survives, but the
   claim about the live instance was wrong and is recorded as wrong rather than quietly dropped.
2. **The adoption bar is configured.** `[adoption] min_trust_level` (default `2`), and the refusal
   names both the bar and the caller's level. A hard-coded number would smuggle a policy decision
   into code, and `Tier` is about recommendation access, not publishing authority.

## Step 1 — migration `0137_wip_adoption.sql`, both engines

Two tables, **no change to `works`**: §56.1 is enforced by the absence of a path that could move
ownership, not by a constraint that could be dropped.

```sql
CREATE TABLE adoption_offers (
    id                  TEXT   PRIMARY KEY,
    work_id             TEXT   NOT NULL,   -- FK works(id) ON DELETE CASCADE
    offered_by_pseud_id TEXT   NOT NULL,   -- no FK: §51 rekeys pseuds
    author_pseud_id     TEXT   NOT NULL,   -- authority for withdrawal, kept as evidence
    adopted_by_pseud_id TEXT,
    status              TEXT   NOT NULL DEFAULT 'open',   -- open | withdrawn | adopted
    created_at          TEXT   NOT NULL,
    updated_at          TEXT   NOT NULL,
    CHECK (status IN ('open','withdrawn','adopted'))
);
CREATE UNIQUE INDEX adoption_offers_open_work ON adoption_offers (work_id) WHERE status = 'open';
CREATE INDEX adoption_offers_status ON adoption_offers (status, created_at);

CREATE TABLE closure_notes (
    id              TEXT   PRIMARY KEY,
    work_id         TEXT   NOT NULL,
    revision_number BIGINT NOT NULL,
    body            TEXT   NOT NULL,
    author_pseud_id TEXT   NOT NULL,
    created_at      TEXT   NOT NULL
);
CREATE UNIQUE INDEX closure_notes_revision ON closure_notes (work_id, revision_number);
```

- **Partial unique index** `(work_id) WHERE status='open'`: one *open* offer per work while withdrawn
  and adopted offers accumulate as history. A plain unique index forbids a second offer after a
  withdrawal — the opposite of "the author may withdraw at any time".
- **`work_id` gets a real FK** (`ON DELETE CASCADE`): a work row is never erased by rekeying, and an
  offer for a missing work is a bug. **The pseud columns get no FK**, for the §58.3 reason: erasure
  rekeys a pseud rather than deleting it, so a cascade would never fire and a key would make an
  erasure fail. Both facts declared in `docs/snapshot-column-policy.json`.
- `revision_number` is `BIGINT` on both engines so the migration-catalogue test compares like with like.

## Step 2 — domain (`crates/domain/src/wip_adoption.rs`, pure)

```rust
pub enum OfferStatus { Open, Withdrawn, Adopted }
pub enum Opening { Opened, NotOwner, NotAbandoned, AlreadyOpen }
pub enum Adoption { Adopted, NotOpen, NotAbandoned, BelowBar { bar: i64, caller: i64 } }
pub enum Withdrawal { Withdrawn, NotTheAuthor, NotOpen }
pub struct Contributor { pub pseud_id: String, pub handle: String, pub first_revision_at: String, pub revisions: i64 }

pub fn may_adopt(min_trust_level: i64, caller_trust: i64) -> bool;
pub fn order_contributors(list: Vec<Contributor>) -> Vec<Contributor>;  // first_revision_at ASC, then pseud_id
pub fn next_revision(current: Option<i64>) -> i64;                      // 1, then +1
```

`order_contributors` is a function rather than an `ORDER BY` in one query so §56.3's ordering rule is
unit tested and cannot be "fixed" by a later reader who thinks most-recent should come first.

## Step 3 — store (`crates/db/src/wip_adoption.rs`)

| function | rule |
|---|---|
| `open_offer(db, work_id, pseud_id)` | caller owns the work; `completion='abandoned'`; no open offer already |
| `withdraw(db, offer_id, pseud_id)` | only `author_pseud_id`; only while `open` |
| `adopt(db, offer_id, pseud_id, min_trust, caller_trust)` | status `open`; work still `abandoned`; trust ≥ bar; **never writes `works`** |
| `list_open(db, limit)` | `status='open'`, newest first |
| `contributors(db, work_id)` | from `revisions` ⋈ `units` (the post-0122 names); **`available: false` + reason when either table is absent** |
| `put_closure_note(db, work_id, pseud_id, body)` | next `revision_number`; returns the revision |
| `latest_closure_note(db, work_id)` | `MAX(revision_number)`, `None` when none |

`has_table(db, name)` is one catalog query (`to_regclass` on PostgreSQL, `sqlite_master` on SQLite),
called only by `contributors`; its answer is the `available` flag, never an error.

## Step 4 — routes (`crates/app/src/routes/wip_adoption.rs`), module `wip_adoption`

| method + path | who | answers |
|---|---|---|
| `POST /api/v1/works/{id}/adoption-offer` | work owner pseud | `201` with the offer, or `422` naming `not_abandoned` / `not_the_owner` / `already_open` |
| `DELETE /api/v1/works/{id}/adoption-offer` | the offer's author | `200 {withdrawn: true}`; `404` when there is no open offer |
| `GET /api/v1/adoption-offers` | anyone | `200 {offers: [...]}`, open only |
| `POST /api/v1/adoption-offers/{id}/adopt` | session | `201`, or `422` naming `not_open` / `not_abandoned` / `below_bar` with both numbers |
| `GET /api/v1/works/{id}/contributors` | anyone | `200 {contributors: [...], available: bool, reason?}` |
| `POST /api/v1/works/{id}/closure-note` | work owner pseud | `201` with the new revision number |
| `GET /api/v1/works/{id}/closure-note` | anyone | `200` latest, or `404` when the work has none |

`RouteClass::Write` for the mutations, `Default` for the reads. Module row: id `wip_adoption`,
relevance **7**, `default_on: true`, `Toggle::Operator`, spec §56, no nav (it lives on the work page).

## Step 5 — tests

`crates/db/tests/wip_adoption.rs` (both engines), 11 cases: offer opens on an abandoned work the
caller owns; a `complete` work is refused naming `not_abandoned`; a non-owner is refused naming
`not_the_owner`; one open offer per work and a second offer **after a withdrawal** succeeds (the
partial index); withdrawal by a non-author is refused; **adoption leaves `works.owner_pseud_id`
unchanged, read before and after**; below-bar adoption reports both numbers; adopting sets
`adopted_by_pseud_id`/`status` and leaves the open list; `contributors` without the table answers
`available: false` with a reason; closure notes version 1→2→3 and `latest` is the highest; a missing
note is `None`, never an error.

`crates/app/tests/m45_28_wip_adoption.rs`, 7 cases: anonymous writes `401`; the module is registered
with no nav; the contributor door answers `available: false` on a live-shaped schema; a complete work
cannot offer; adopting from a second account does not change the owner; closure note `404` → `201` →
`201` at revision 2; the offers list is open-only.

## Verification

| # | command | expected |
|---|---|---|
| 1 | `cargo test -p darkhold-domain --lib wip_adoption` | 5 passed |
| 2 | `cargo test -p darkhold-db --test wip_adoption` | 11 passed |
| 3 | `DARKHOLD_TEST_PG_URL=… cargo test -p darkhold-db --test wip_adoption` | 11 passed |
| 4 | `cargo test -p darkhold-app --test m45_28_wip_adoption` | 7 passed |
| 5 | mutations (below) | every mutation caught |
| 6 | `cargo clippy -p darkhold-domain -p darkhold-db -p darkhold-app --all-targets` | no new warning |
| 7 | `python3 scripts/ideas_catalogue.py` | 0 homeless |

Mutations that must be caught: drop the ownership check from `open_offer`; accept `hiatus` as
abandoned; drop the `status='open'` guard from `adopt`; drop the trust comparison; write
`owner_pseud_id` in `adopt`; order contributors by revisions descending; reuse revision numbers;
answer `available: true` when the table is missing; withdraw without checking the author.

## Deferred, with reasons

- **Credit for a closure note equal to a chapter** (§56.2) — credit is computed in the economy, and
  nothing in this tree awards per-chapter credit yet (`chapter_revisions` has no writer). Paying a
  note against a ledger that pays no chapters would be a number with nothing to be equal to.
- **A `hiatus` work may be offered** — the draft allowed it; the spec says `abandoned`, and a hiatus
  is an author who intends to return. The spec wins.
- **A restore route for old closure notes** — revisions are rows, so restoring is reading
  `revision_number = n`. A route for it is a second write surface nobody asked for.
