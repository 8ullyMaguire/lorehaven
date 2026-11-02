# M45-26 — the welcome rota: a reader is handed one work and 72 hours

Queue head by `docs/requirements.csv` (milestone column, then row id): M45-26
`community`, *"Feedback-drought intervention: welcome rota guarantees first-work comment in 72h.
Route readers to strong-completion/few-comment works; small comment bonus."*

## Premises, measured before designing (2026-11-02, live `darkhold` DB)

| claim | measurement |
|---|---|
| the `comments` table exists | yes (`information_schema`), **0 rows** |
| `library_items.status='complete'` | **397** rows |
| `library_items.likes > 0` | **1,247** rows |
| both at once | **0** — the two populations are disjoint |
| `(likes>0 OR status='complete')` | **1,644** rows |
| …of those, `work_id IS NOT NULL` | **290** — a comment needs a work id to attach to |
| `library_items` rows with no `source_updated_at` | 49,846 of 141,317 |

Two consequences, both taken into the design rather than worked around:

1. **A candidate rule of "complete AND endorsed" routes nothing, forever.** The estate import
   carries publication status with no engagement; the boards carry engagement with no clean status.
   The rule is therefore `likes > 0 OR status = 'complete'` — "strong-completion/few-comment" read as
   the notes wrote it, with the two signals as alternatives because on this instance they never co-occur.
2. **290 is the real supply, not 1,644.** A work with no `work_id` cannot receive a comment, so it is
   excluded and the report says so. That number grows when M63-32/M63-33 backfill work ids; it is not
   a defect of this item.

Also measured: `sessions.active_pseud_id` is a session property, and `SessionUser.pseud_id` is
`Option` — a session with no pseud chosen cannot comment, so it cannot take a welcome.

## Deferred, with the reason (writer's decision, §"what to defer")

**The "small comment bonus" is not built — it is already paid, and paid twice over if built.**
§58.4 pays `PATRONAGE_BP` (5.00 credits) on completion of a supply route, and that money exists
precisely to reward feedback. A second per-comment faucet would pay the same act twice.

It is also not the thing worth fixing first. Reading `crates/db/src/supply.rs:891-935` this session:
**`supply::complete` pays on a click and never checks that a comment exists.** §58.4 calls patronage
"a reward for giving feedback, not a purchase"; the door grants it for pressing a button. No live
row has been paid yet (`supply_routing` is empty on the host), so it is inert today and real
tomorrow. Filed as `M45-60` with that line reference rather than fixed here, because changing a
payment path deserves its own row, its own test and its own mutation — and the welcome rota is what
shows what the check should look like: `welcome::complete` refuses to close a row until a comment by
that pseud on that work exists after the assignment.

## Schema — `migrations/{sqlite,postgres}/0135_welcome_rota.sql`

Twin files, identical semantics (SQLite has no `UUID` type; both store ids as `TEXT`).

```sql
CREATE TABLE welcome_rota (
    id           TEXT PRIMARY KEY,
    work_id      TEXT NOT NULL,
    pseud_id     TEXT NOT NULL,
    author_key   TEXT NOT NULL DEFAULT '',
    assigned_at  TEXT NOT NULL,
    due_at       TEXT NOT NULL,
    completed_at TEXT,
    comment_id   TEXT
);
CREATE INDEX welcome_rota_open ON welcome_rota (pseud_id, completed_at);
CREATE INDEX welcome_rota_work ON welcome_rota (work_id);
```

- **No foreign keys**, for the same reason `supply_routing` has none: the record must survive
  erasure of the account it names. A cascade here would delete the evidence that a welcome was owed.
- **Timestamps are canonical** `YYYY-MM-DDTHH:MM:SSZ` (§M63-31, migration 0132). Trap 5 is that the
  database enforces it; a fixture writing `2026-11-02` is a bug in the fixture.
- `docs/snapshot-column-policy.json` gains one entry per column. The gate names what it wants:
  run `python3 scripts/check-snapshot-pii.py` after adding the file and add exactly the columns it
  reports, with `pseud_id` → `rekey_pseud` (a pseudonym, not an account) and `author_key` → `keep`
  (already public as `library_items.author_text`).

## Store — `crates/db/src/welcome.rs`

Every statement is assembled per backend and goes through `sql_owned` (trap 11); `likes` is INT4 on
PostgreSQL and decodes as `i64` only through `CAST(… AS BIGINT)`; `id::text` where a text id is
compared on PostgreSQL.

```rust
pub const DUE_HOURS: i64 = 72;

pub struct Assignment { id, work_id, title, source_key, author_key, assigned_at, due_at, overdue }
pub enum Claim { Assigned(Assignment), AlreadyOpen(Assignment), NothingToWelcome { eligible: i64 } }
pub enum Complete { Done(Assignment), NoCommentYet { work_id: String }, NotYours, NoSuchAssignment }
pub struct RotaReport { measured: bool, reason: Option<String>, eligible, assigned, completed, overdue }

pub async fn claim(db: &Database, pseud_id: &str) -> Result<Claim>;
pub async fn mine(db: &Database, pseud_id: &str) -> Result<Vec<Assignment>>;
pub async fn complete(db: &Database, pseud_id: &str, id: &str) -> Result<Complete>;
pub async fn report(db: &Database) -> Result<RotaReport>;
```

Candidate query (one back end at a time, same predicate):

```sql
SELECT li.id, CAST(li.work_id AS TEXT) AS work_id, li.title, li.source_key,
       lower(trim(li.author_text)) AS author_key
  FROM library_items li
 WHERE li.work_id IS NOT NULL
   AND (li.likes > 0 OR li.status = 'complete')
   AND NOT EXISTS (SELECT 1 FROM comments c
                    WHERE c.subject_type = 'work' AND c.subject_id = CAST(li.work_id AS TEXT))
   AND NOT EXISTS (SELECT 1 FROM welcome_rota r
                    WHERE r.work_id = CAST(li.work_id AS TEXT) AND r.completed_at IS NULL)
   AND lower(trim(li.author_text)) NOT IN (SELECT author_key FROM welcome_rota)
 ORDER BY (li.likes > 0) DESC, CAST(li.likes AS BIGINT) DESC, li.title
 LIMIT 1
```

- **One open welcome per reader.** `claim` returns the open one rather than taking a second, so a
  reader cannot bank assignments and the 72-hour promise stays one promise.
- **One welcome per author**, not per work: the point is that an author hears from someone, and
  three comments on three of their works is the same author twice.
- **Completion is evidenced, not pressed.** `complete` looks for a comment by that pseud on that
  work created at or after `assigned_at`; with none it answers `NoCommentYet` and writes nothing.
  A button that marks the rota done without the comment is the feature failing while showing green.
- `overdue` is computed, not stored: `completed_at IS NULL AND due_at < now`.

## Routes — `crates/app/src/routes/welcome.rs` (module id `welcome`)

| door | who | answers |
|---|---|---|
| `GET /api/v1/welcome/rota` | reader (`RequireSession`) | `{assignments: [...], overdue: n}` — what I owe |
| `POST /api/v1/welcome/rota/claim` | reader | `201` + the assignment, or `409 NO_WELCOME_NEEDED` in words when nothing qualifies |
| `POST /api/v1/welcome/rota/{id}/complete` | reader | `200` + the closed row, or `409 COMMENT_REQUIRED` naming the work |
| `GET /api/v1/welcome/report` | operator | eligible / assigned / completed / overdue, or a refusal naming why it cannot be measured |

- A session with `pseud_id == None` answers `409 PSEUD_REQUIRED` and does not take an assignment:
  the promise is made to a pseud, and a pseud is what can write the comment.
- `409 NO_WELCOME_NEEDED` carries `eligible` so an operator can tell "everything has a comment"
  from "the query is broken".
- Reader doors never expose another reader's assignment: `mine` filters on `pseud_id`.

Mount: one `module_mount(&state, "welcome", routes::welcome::router(), RouteClass::…)` in
`crates/app/src/server.rs`; the registry-row test fails if a module is mounted without a row.

## Registry row — `crates/domain/src/modules/registry.rs`

```rust
id: "welcome", name: "Welcome Rota",
summary: "M45-26 — one finished, uncommented work handed to a reader, with 72 hours to be its first commenter.",
default_on: true, stability: Stability::Stable, toggle: Toggle::PerUser,
requires: &[], milestone: "M45-26", spec: "§58.6",
nav: &[NavEntry { label: "Welcomes", href: "/welcome", slot: "reader_sidebar" }],
relevance: 8, source: "crates/app/src/routes/welcome.rs",
```

`per_user` because taking a welcome is a reader's own choice; the instance may still switch the
surface off. `relevance: 8` — a community mechanic that pays nothing and costs a reader one comment.

## Spec — `docs/spec.md` §58.6

A short subsection under §58 stating the rota, the 72-hour promise, the evidence rule (a comment,
not a click), and the deferral of the credit bonus with its reason.

## Tests and the mutations each must catch

`crates/db/tests/welcome_rota.rs` (both engines, `--test-threads=1`):

| test | mutation that must break it |
|---|---|
| `a_commented_work_is_never_offered` | drop the `NOT EXISTS comments` clause |
| `a_work_with_an_open_welcome_is_not_offered_twice` | drop the open-assignment clause |
| `one_author_gets_one_welcome` | drop the author exclusion |
| `a_reader_holds_one_open_welcome_at_a_time` | let `claim` insert unconditionally |
| `complete_refuses_without_a_comment` | mark complete before the comment lookup |
| `complete_accepts_the_comment_that_evidences_it` | ignore `assigned_at` in the comment window |
| `the_due_date_is_seventy_two_hours_out` | change `DUE_HOURS` |
| `a_work_with_no_work_id_is_never_offered` | drop `work_id IS NOT NULL` |

`crates/app/tests/m45_26_welcome_rota.rs`:

- all three reader doors `401` anonymously; the report `401` for a reader;
- `claim` → `201` with a `work_id`; a second `claim` returns **the same** assignment id;
- `complete` before any comment → `409 COMMENT_REQUIRED` and the row is still open;
- after inserting a comment as that pseud → `200`, `completed_at` set, `comment_id` recorded;
- a session with no active pseud → `409 PSEUD_REQUIRED`.

## Gates, in order

```bash
cargo test -p darkhold-domain -p darkhold-db -p darkhold-app --lib
sudo docker start lh-pg-test
cargo test -p darkhold-db --test welcome_rota            # sqlite
LOREHAVEN_TEST_PG_URL=… cargo test -p darkhold-db --test welcome_rota
cargo test -p darkhold-app --test m45_26_welcome_rota
python3 scripts/ideas_catalogue.py                        # the idea catalogue must stay green
python3 scripts/check-snapshot-pii.py
cargo clippy -p darkhold-db -p darkhold-app -p darkhold-domain --all-targets
```

## Deploy and live proof

Per `darkhold-thinkcentre-deploy`: sync `local` → reset on the mount, build release on the host,
stop, `migrate` **from the tree root** (0135 is new; `--status` never applies), start, `is-active`.

Live proof, in this order:

1. `GET /api/v1/welcome/report` anonymously → `401`.
2. `GET /api/v1/api/v1/modules` → `welcome` present with `enabled: true` and a `nav` entry.
3. throwaway reader: register → create pseud → `POST /welcome/rota/claim` → `201` with a work;
   second claim → same id; `complete` → `409 COMMENT_REQUIRED`.
4. delete the throwaway account and its rota rows (a synthetic welcome must not become product signal).

## Docs that move with it

`requirements.csv`: M45-26 → `implemented-fully-tested` with the test paths as evidence; new row
`M45-59` for the deferred bonus; `notes` naming the 290-candidate measurement and why the rule is
`OR` not `AND`. `docs/handoff.md` section. Vault session note.
