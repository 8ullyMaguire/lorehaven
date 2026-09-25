# PostgreSQL migration chain: repaired, and the baseline it reveals

Date: 2026-09-25. Commits `de9555a` and `6789f5a`.

## What was broken

The PostgreSQL migration chain **could not apply past migration 0041**. Every
test that opened a scratch PG database failed during `migrate`, before a single
assertion ran:

    migrate: applying migration 0041_spoilers_readability
    error returned from database: foreign key constraint
    "reader_work_progress_work_id_fkey" cannot be implemented

A `TEXT` column cannot reference a `UUID` primary key. Seven columns across two
migrations, all pre-existing:

| migration | table | column | target |
|---|---|---|---|
| 0041 | `reader_work_progress` | `work_id` | `works(id)` UUID |
| 0041 | `reader_work_progress` | `account` | composite key, follows `work_id` |
| 0041 | `post_drafts` | `account` | `accounts(id)` UUID |
| 0041 | `reader_warning_prefs` | `account` | `accounts(id)` UUID |
| 0042 | `forum_sanctions` | `account` | `accounts(id)` UUID |
| 0042 | `forum_sanctions` | `actor` | `accounts(id)` UUID |
| 0042 | `forum_post_first_vote_notified` | `account` | `accounts(id)` UUID |
| 0042 | `forum_posts.featured_by` (ALTER) | `featured_by` | `accounts(id)` UUID |

`forum_topics.original_topic_id` is **correct** — `forum_topics.id` is genuinely
`TEXT`, so a text FK is right there and was left alone.

These migrations had never successfully applied anywhere (no checksum, no row in
`_migrations`), so they were edited in place rather than superseded by a new
numbered migration.

## The other defect, same feature, opposite shape

Migration 0035 broke `device_deliveries` differently in each dialect:

- **SQLite** renamed `export_jobs`, and SQLite rewrites *referencing* foreign
  keys on rename — so the 0008 constraint was repointed at `export_jobs_old`,
  which 0035 then dropped. Every insert: `no such table: main.export_jobs_old`.
- **PostgreSQL** dropped `device_deliveries` outright to invalidate its
  constraints, then recreated `download_grants` and `bulk_export_items` but never
  `device_deliveries`. The table has been **absent** on PG since 0035.

Both shipped green because **nothing in the codebase inserts into that table**.
The schema was wrong and unexercised, not working.

## Why this survived so long

SQLite is permissive about foreign-key type mismatches; PostgreSQL refuses the
constraint outright. Every test ran on SQLite, so nothing ever noticed that the
PG dialect could not even start. The workspace's own pitfall note warns about
exactly this (`works(id)`, `accounts(id)`, `pseuds(id)` are `UUID` in Postgres,
so any FK referencing them must be `UUID` too) — the warning was right and the
defect was still there.

## The state this reveals

With the chain repaired, the full PG suite runs for the first time:

| | SQLite | PostgreSQL |
|---|---|---|
| suites | 82 | 83 |
| passed | 1785 | 1637 |
| failed | 0 | 151 |

**The 151 PG failures are not regressions** — they are the first evidence of
what the dialect has been doing. Breakdown of the 21 database-level errors:

- 20 × `42804 column "id" is of type uuid but expression is of type text`
- 5 × `42804 column "created_at" is of type timestamp with time zone but
  expression is of type text`
- 3 × `42804 column "enabled" is of type boolean but expression is of type
  integer`
- 1 × `42601 syntax error at or near ","` (a `::` cast in a SQLite arm)
- 1 × `22P02 invalid input syntax for type uuid`

**Every one of the 20 `id`-cast errors is in a test file, not production
code** — verified by parsing the panic locations out of the log. The remaining
failures are assertion failures in tests written for SQLite semantics.

## The rule this establishes

`Database::sql()` renumbers `?` to `$n` for PostgreSQL but **does not cast**, and
sqlx sends a bound `&str` as `text`. So a statement that reads

    SELECT dimension_key FROM arena_weights WHERE account_id = $1

passes on SQLite and fails on PostgreSQL with 42804. The house pattern is the
two-arm `db.sql(sqlite, postgres)` form, spelling the PostgreSQL arm's id
placeholders `?::uuid` — exactly as `create_export` in `crates/db/src/exports.rs`
does. `crates/app/tests/device_delivery_fk.rs` demonstrates it.

## It is not only the tests

Fixing the first file (`arena.rs`) surfaced the more important half: the same
mistake is in **production code**, and the two failure modes are different.

- **Writing**: a bare `$n` into a `uuid` column → 42804.
- **Reading**: a `uuid` column into a Rust `String` → `ColumnDecode`, and sqlx
  will not decode `UUID` into `String` or `INT4` into `i64`. The column has to be
  cast in the `SELECT`: `w.id::text`, `matches_played::bigint`,
  `COALESCE(wc.word_count, 0)::bigint`.

`crates/db/src/taste_vectors.rs` had five of these, in `record_arena_ballot`,
`get_arena_weights`, `update_arena_weights`, `get_arena_pool` and the work-rating
count. They have never run on PostgreSQL at all, because nothing in the suite
exercised them there before this.

`scripts/check-uncast-pg-placeholders.py` finds them: **68 sites in 16 files**,
of which 62 are genuine (the rest are false positives the script's own report
identifies). It is a heuristic and reports where to look, not a verdict — a
wrong cast on the wrong column still passes review. Run it after touching any
`db/src` query.

## Deterministic test ids

Fixing fixtures also needed a shared helper. `test_support::id(label)` maps a
readable slug (`"work-a1"`) to a stable UUID, so a fixture binds the same value
when it inserts and looks it up later. Two details that cost time and will cost
the next person time:

- **FNV-1a with a fixed offset basis, not `DefaultHasher`.** `DefaultHasher` is
  seeded per process, so two fixtures in one test binary produced colliding ids
  and `works.id` failed UNIQUE on the second insert.
- **Not `Uuid::new_v5`** — the workspace enables only uuid's `v4` feature, and
  adding `v5` for a test helper would widen the dependency surface of every
  crate.

## Next

The 68 sites are the last major piece of real PostgreSQL coverage. They are
mechanical (cast the placeholder, or cast the column in the SELECT) but they
span 16 files, and each fix should be gated on both dialects. Order by the
detector's output: `media_resilience` (8), `community` (7), then the rest.

Run it as its own milestone, gated on both dialects every time — the whole
failure mode here was running one.

## Running PG locally

    sudo systemctl start docker
    sudo docker run -d --name lh-pg --shm-size=512m \
      -e POSTGRES_PASSWORD=lhreview -e POSTGRES_USER=lorehaven \
      -e POSTGRES_DB=postgres -p 55433:5432 postgres:17
    sudo docker exec lh-pg psql -U lorehaven -d postgres \
      -c "ALTER USER lorehaven PASSWORD 'lhreview';"
    cd ~/code-local/rust/lorehaven
    export LOREHAVEN_TEST_PG_URL='postgres://lorehaven:lhreview@127.0.0.1:55433/postgres'
    CARGO_TARGET_DIR=$HOME/.cargo-target/lorehaven cargo test --workspace --no-fail-fast

`--shm-size=512m` is required: the default 64 MB exhausts under concurrent
scratch-database creation and produces `No space left on device` cascades that
look like every test failing.
