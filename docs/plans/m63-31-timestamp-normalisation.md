# M63-31 — normalise catalogue timestamps at the database, so the window is a plain comparison

Status: planned → implementing. Authority for the catalogue: `docs/plans/m63-02-window-grammar.md`
(it shipped the grammar and the read-side fix this item removes).

## What M63-02 left behind, in numbers

Live database, measured 2026-11-01 (`select count(*) … group by shape`):

| shape | rows | producer |
|---|---|---|
| `YYYY-MM-DD` | 91,471 | the XenForo importer — **outside this workspace** |
| `YYYY-MM-DDTHH:MM:SS+00:00` | 46,281 | `crates/app/src/imports.rs` (`Rfc3339` on an `OffsetDateTime`) |
| `YYYY-MM-DDTHH:MM:SSZ` | 60 | older fixtures |
| `YYYY-MM-DDTHH:MM:SS.NNNNNN` (no zone) | 3 | older fixtures |

`crates/db/src/catalogue.rs` therefore carries a shape-deciding `CASE` per row
(`pg_instant`), and a `datetime()` per row on SQLite. That is a function call on the column, so
PostgreSQL cannot use an index: `?updated=7d` measured 1.7 s cold on 141,317 rows.

## What this item does

1. **One canonical shape, enforced by the database**: `YYYY-MM-DDTHH:MM:SSZ`, second precision.
   The plan first said `+00:00`; measurement overturned it. `time`'s `Rfc3339` formatter writes `Z`
   for UTC (`OffsetDateTime::now_utc().format(&Rfc3339)` → `2026-11-01T15:48:17.643518374Z`,
   verified with a throwaway test), so `+00:00` would have made the database refuse its own
   writer's output. Thirty-odd fixtures that mirror production failed, which is how the mistake
   announced itself. Postgres gets a `CHECK`; SQLite gets `BEFORE INSERT`/`BEFORE UPDATE` triggers
   with `RAISE(ABORT)` (SQLite cannot add a constraint to an existing table without a rebuild).
2. **Backfill** in the same migration, before the constraint: the other three shapes are rewritten
   by the *same rule M63-02 already applies at read time* — a bare date or a zone-less datetime is
   UTC by definition, a `Z` is an instant — so the backfill changes no row's meaning. Existing rows
   whose text a client may have cached keep their instant; only the rendering changes.
3. **The read path loses the `CASE`**: the predicate becomes `(col)::timestamptz` /
   `datetime(col)`, and a plain btree index on the column becomes usable.

## A second defect, found because the first draft was wrong

Formatting `now_utc()` with `Rfc3339` keeps **nanoseconds**, so every timestamp this server writes
carries nine fractional digits. Two consequences, both real:

- The canonical shape cannot be second precision while the writer emits `…17.643518374Z`, so the
  writer is where this had to change: `darkhold_db::identity::format_rfc3339` now truncates to
  seconds, and `library_input` in `crates/app/src/imports.rs` formats the source's date through it
  instead of calling `Rfc3339` directly.
- Fractional seconds with a variable number of digits do not sort correctly as text (`.5` sorts
  before `.12`), so any window or ordering that compared them as strings was wrong for those rows.
  The live data holds only 3 such rows, which is why nothing had noticed.

## Decision: enforce, and let the out-of-tree writer fail loudly

The bare dates are produced by an importer that is **not in this workspace** — `crates/scrapers`
returns `OffsetDateTime` and `crates/app/src/imports.rs` formats RFC3339, so neither can produce
`2026-10-13`. That is exactly why the guarantee belongs in the database: a rule enforced only in
this tree would be bypassed by the one writer that broke the shape in the first place.

Consequence, stated plainly and in the item's notes: the next run of that external loader will
**fail** with `new row for relation "library_items" violates check constraint
"library_items_canonical_timestamps"` instead of storing `YYYY-MM-DD`. That is the intended
trade — a loud failure at the writer beats a silent hour-long shift in every window on the
catalogue. If the operator would rather coerce silently, the alternative is a `BEFORE` trigger that
rewrites the value; it is recorded here as option B and rejected because it hides the defect class
M63-02 spent a session finding.

## Files

| file | change |
|---|---|
| `migrations/postgres/0132_catalogue_timestamp_canonical.sql` | backfill, `CHECK … NOT VALID` + `VALIDATE`, plain btree index |
| `migrations/sqlite/0132_catalogue_timestamp_canonical.sql` | backfill, insert/update triggers, plain btree index |
| `crates/db/src/catalogue.rs` | delete `pg_instant`, the `CASE`, and the "three formats" comment block; predicates become one cast per column |
| `crates/db/tests/catalogue_window.rs` | fixtures move to the canonical shape; add the two new tests below |
| `crates/app/tests/milestone_8.rs` | its one `…Z` library fixture moves to `…+00:00` |

## Steps, with the command and the output each must produce

1. **Plan** (this file). Nothing to run.

2. **Migration, PostgreSQL.** Applied by `migrate` against the live database.

   ```sql
   UPDATE library_items SET source_updated_at = CASE
       WHEN source_updated_at IS NULL THEN NULL
       WHEN source_updated_at ~ '^\d{4}-\d{2}-\d{2}$' THEN source_updated_at || 'T00:00:00+00:00'
       ELSE to_char((CASE WHEN source_updated_at ~ '(Z|[+-]\d{2}:\d{2})$'
                          THEN source_updated_at::timestamptz
                          ELSE source_updated_at::timestamp AT TIME ZONE 'UTC' END)
                    AT TIME ZONE 'UTC', 'YYYY-MM-DD"T"HH24:MI:SS"+00:00"')
   END;
   ```
   the same for `last_synced_at` and `created_at`, then
   ```sql
   ALTER TABLE library_items ADD CONSTRAINT library_items_canonical_timestamps CHECK (
        (source_updated_at IS NULL OR source_updated_at ~ '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\+00:00$')
    AND (last_synced_at   IS NULL OR last_synced_at   ~ '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\+00:00$')
    AND (created_at       IS NULL OR created_at       ~ '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}\+00:00$')
   ) NOT VALID;
   ALTER TABLE library_items VALIDATE CONSTRAINT library_items_canonical_timestamps;
   CREATE INDEX IF NOT EXISTS idx_library_items_source_updated_at ON library_items (source_updated_at);
   ```

   Verification, on the live database:
   ```
   sudo -u postgres psql -d darkhold -c "select count(*) from library_items where source_updated_at !~ '…\+00:00$'"   → 0
   ```

3. **Migration, SQLite.** Same backfill via `strftime`/`datetime`, then:
   ```sql
   CREATE TRIGGER library_items_canonical_source_updated_at_insert
     BEFORE INSERT ON library_items WHEN NEW.source_updated_at IS NOT NULL
       AND NEW.source_updated_at NOT GLOB '[0-9][0-9][0-9][0-9]-[0-9][0-9]-[0-9][0-9]T[0-9][0-9]:[0-9][0-9]:[0-9][0-9]+00:00'
     BEGIN SELECT RAISE(ABORT, 'source_updated_at must be canonical UTC (+00:00)'); END;
   ```
   plus the `UPDATE` twin, for each of the three columns. `GLOB` because `RAISE` needs a `WHEN`
   that SQLite evaluates without a function call per row before the match.

4. **Read path.** In `windows_sql`, `pg_instant(column)` → `({column})::timestamptz`, and the
   SQLite arm `datetime(col)` stays (it is index-compatible when the index is an expression; here it
   becomes unnecessary — see step 5).

5. **Which comparison, decided by measurement, not by taste.** After the backfill every value has
   the same 25-character shape, so a plain text comparison is chronologically correct *if* the
   database's collation orders digits positionally. That is verified, not assumed:
   ```sql
   WITH s AS (SELECT source_updated_at t,
                     row_number() OVER (ORDER BY source_updated_at) r_text,
                     row_number() OVER (ORDER BY (source_updated_at)::timestamptz) r_inst
              FROM library_items WHERE source_updated_at IS NOT NULL)
   SELECT count(*) FROM s WHERE r_text <> r_inst;      -- expected 0
   ```
   If it is 0, the predicates compare text and a plain btree index serves them. If it is not 0, the
   predicate keeps the `::timestamptz` cast and the index becomes an expression index on the same
   cast. The result goes in the plan either way.

6. **Tests, each watched failing first.**
   - `db/tests/catalogue_window.rs::every_shape_is_now_one_shape` — the four legacy strings, inserted
     through the backfill's rule, come back canonical and compare equal to their instants.
   - `…::a_non_canonical_timestamp_is_refused` — inserting `2026-10-13` must fail on both engines
     (Postgres: `check constraint`; SQLite: `RAISE(ABORT)`).
   - the existing four window tests keep their assertions with canonical fixtures.
   Mutations to run, each reverted afterwards: drop the CHECK → `a_non_canonical_timestamp_is_refused`
   fails; backfill only `source_updated_at` → `every_shape_is_now_one_shape` fails; canonicalise with
   the session timezone instead of UTC → `a_zone_less_bound_does_not_move_the_window` fails (the
   backfill would shift 27,464 instants by an hour).

7. **Gates** (from the standing prompt): domain/db/app libs, `catalogue_window` on SQLite and
   PostgreSQL, `catalogue_window_route`, clippy on the touched crates.

8. **Deploy, then prove it on the running artifact.** Rebuild, `systemctl --user restart darkhold`,
   then:
   ```
   curl -s -o /dev/null -w '%{time_total}\n' 'http://127.0.0.1:8081/api/v1/catalogue?updated=7d&limit=20'
   ```
   against the 1.7 s baseline, plus the same 96-row answer as before the change (the backfill moves
   no instant, so the result set must be identical — `@2026-10-27` must still return 10).

9. **Docs**: the `requirements.csv` row (status, evidence, notes incl. the out-of-tree writer
   consequence), this plan's "what landed", and the session note in the vault.

## What is explicitly not in this item

- Changing the column type to `timestamptz`. It would make text-vs-instant questions disappear, but
  the column is bound as `Option<String>` in the API, the exporter and ~40 call sites, and the
  wire format a client sees is that string. Recorded as a follow-up, not smuggled in here.
- Rewriting the external XenForo loader. It is not in this workspace; the constraint is what makes
  its next write visible instead of silent.

## What landed (`9254e5e6`), and what the live run measured

Deployed as build `0.1.0+9254e5e6`; migration 0132 applied to the live database in 226 s
(327,764 rows touched across the three columns, then `VALIDATE`, then the index). 129 migrations
applied, 0 pending.

| claim | command | output |
|---|---|---|
| one shape, no exceptions | `select count(*) … where col !~ '^\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}Z$'` | **0** across all three columns |
| no instant moved | snapshot table `m6331_before` vs the new values | 3 rows differ, **max drift 0.948 s** — the sub-second truncation, by design |
| the partition still partitions | `>= '2026-10-25T12:00:00Z'` / `<` | 105 + 91,366 = **91,471**, exactly the number of dated rows before |
| the same day answers the same | `?updated=@2026-10-27` | **10 items**, unchanged from before the migration |
| the window uses the index | `EXPLAIN` on the endpoint's predicate | `Bitmap Index Scan on idx_library_items_source_updated_at` (was `Parallel Seq Scan`) |
| a legacy shape is refused live | `INSERT … '2026-10-13'` in a transaction | `ERROR: violates check constraint "library_items_canonical_timestamps"` |
| the running server has it | `/api/v1/meta` | `Darkhold 0.1.0 0.1.0+9254e5e6` |
| nothing else broke | journal since restart | 0 errors, 0 constraint violations; `?source=sb&sort=quality` 200 in 0.23 s |

Latency, same host, same data, `limit=20`:

| query | before | after |
|---|---|---|
| `?updated=7d` | 0.34–0.71 s | **0.01 s** |
| `?updated=2026-01-01..2026-10-15` (44 k rows in range) | 0.29–0.35 s | **0.03 s** |

The wide window is the honest caveat: `EXPLAIN` on a predicate covering 44 k rows chooses a bitmap
scan and takes 227 ms for the page — the index is not a win there, and the planner is right not to
use a plain index scan. It is a win for the windows people actually ask for (`7d`, `@date`), which is
the case that was slow.

## Deferred, with reasons

- `updated_at` has the same mixed history (58,728 `Z` + 66,875 `+00:00` in `created_at`, similar in
  `last_synced_at`) and is still unconstrained. It is not a window column, so it was out of scope;
  the next migration that touches this table should include it.
- The column type stays `TEXT`. `timestamptz` would remove the text-versus-instant question, but the
  value is bound as `Option<String>` in the API, the exporter and ~40 call sites, and the wire format
  a client sees is that string. Its own item.
