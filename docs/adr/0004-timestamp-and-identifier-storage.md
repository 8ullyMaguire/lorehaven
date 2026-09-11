# ADR 0004 — Timestamp and identifier storage

Status: accepted (Milestone 0)
Date: 2026-09-10

## Problem

Spec §2.2 requires Lorehaven to run on **either** SQLite or PostgreSQL, and
spec §4 insists the engines are not interchangeable:

> Use database-specific migrations and repository implementations where SQL
> differs. Do not assume SQLite and PostgreSQL are interchangeable.

At the same time the spec fixes some conventions: UUID primary identifiers,
timestamps stored as UTC and presented as RFC 3339, native UUID columns in
PostgreSQL.

Those two requirements are in tension with a *third* constraint that is not in
the spec but matters more in practice: this project has one maintainer, and a
repository layer duplicated per dialect is a repository layer that drifts.

## Decision

Two rules, applied uniformly:

1. **DDL is per dialect; DML is per dialect too, but narrow.** Migrations live
   in `migrations/sqlite/` and `migrations/postgres/` with identical filenames,
   and a build-time check (a unit test) fails if the two catalogues define
   different sets of migration ids. Repository statements are supplied as a
   *pair* — `db.sql(sqlite_sql, postgres_sql)` — and PostgreSQL's string has its
   `?` placeholders rewritten to `$1…$n` so both bind positionally.

2. **Rows decode identically on both engines.** Every parameter and every
   selected column is `String` or `i64`:
   - identifiers are `TEXT` in SQLite and `UUID` in PostgreSQL, with PostgreSQL
     reading `id::text` and writing `?::uuid`;
   - **timestamps are `TEXT` (RFC 3339, UTC) in both engines**;
   - flags are avoided in the identity tables rather than being `INTEGER` in one
     dialect and `BOOLEAN` in the other.

## Alternatives considered

**Native `TIMESTAMPTZ` in PostgreSQL** (the conventional choice). Rejected for
now because sqlx decodes it as a time type while SQLite yields text, which
would force a per-dialect decode on *every* query that touches a timestamp — the
single largest source of duplicated repository code. RFC 3339 UTC text is
unambiguous, sorts lexicographically, is index-friendly in a B-tree, and is
exactly what the API already exposes (spec §3.1).

**ORM / query builder** (SeaORM, Diesel). Would remove most hand-written SQL and
much of the duplication. Rejected because the structured search in spec §14
compiles to correlated `EXISTS` clauses that are already at the edge of what a
builder can express, and because a builder hides exactly the domain-specific
details this schema is made of. Spec §4 also assumes hand-written migrations.

**The `Any` driver in sqlx.** The stated way to write one query for both
engines. Rejected because it restricts both dialects to a common subset, which
means giving up precisely the engine-specific behaviour spec §4 assumes we use
(for example PostgreSQL's partial unique indexes are fine, but `ON CONFLICT`
target syntax and type casts are not uniform) — and the resulting errors surface
at runtime.

**Storing UUIDs as text in PostgreSQL too.** Maximum uniformity, minimum code.
Rejected because spec §3.1 explicitly asks for native UUID columns, and native
UUID gives real integrity (a malformed id cannot be stored) and half the index
size.

## Consequences

- PostgreSQL is genuinely second-class in one respect: **no PostgreSQL
  integration test suite runs today**, because the development machine has no
  server installed. `docs/verification.md` records the single hand run that has
  happened, and the migration catalogue is kept identical by test so the
  divergence cannot silently widen. This is the largest open risk in the
  database layer. (Amended 2026-09-11 — see below.)
- The cast-based approach means `pseuds.account_id` etc. are compared as UUIDs
  on PostgreSQL (via `::uuid`) and as text on SQLite. Index usage is preserved
  on both, because the cast is on the parameter, not on the column.
- Timestamps are strings. Range queries work, but PostgreSQL date functions
  (`date_trunc`, interval arithmetic) are not available without a cast. Reporting
  that needs them should cast explicitly at the query, not change the column.
- Any future column that must decode as a different Rust type has to be added to
  both dialect strings, and the compiler will not catch a missing cast — only a
  test against a real PostgreSQL instance will.

## Conditions that would justify revisiting

- PostgreSQL becomes a supported deployment target for a real instance, at which
  point a CI job with a `postgres` service must run the same integration suite,
  and the casts should be reviewed statement by statement.
- A reporting feature needs native timestamp arithmetic or timezone conversion in
  SQL, which would justify `TIMESTAMPTZ` and the per-dialect decode cost.
- The dialect-pair approach is measured to cost more maintenance than an ORM
  would, for example if a schema change starts touching dozens of statements.


## Amendment, 2026-09-11 — the first live run

This decision was made without a PostgreSQL server to test against, and the
consequences section predicted its own failure mode: *"the compiler will not
catch a missing cast — only a test against a real PostgreSQL instance will."*
On 2026-09-11 the PostgreSQL half was run for the first time, against 17.11 in
Docker on loopback, and eight defects fell out of it. `docs/verification.md`
lists all eight; three of them change this decision.

**1. Rule 2 extends to the width of an integer.** SQLite's `INTEGER` is 64-bit;
PostgreSQL's is 32-bit. The repository decodes `i64`, and `sqlx` will not widen
`INT4` into `INT8`, so every `version`, count and 0/1 flag failed to read. Every
PostgreSQL column holding a number the repository reads as `i64` is now
`BIGINT` — 38 of them across the eight migrations. This is rule 2 applied
consistently rather than a new rule: the decode layer is shared, so the column
has to be the type the shared decode expects.

**2. Where the rule cannot hold, cast in the query.** A 0/1 flag the shared
struct reads as `i64` stays `BOOLEAN` in PostgreSQL (it is the right type, and
SQLite has nothing better), and is read as `bool::int::bigint`. The double cast
is not redundant: PostgreSQL has no boolean-to-bigint cast.

**3. `ON CONFLICT` targets cannot be inferred from a partial index's columns
alone.** The alternatives section predicted this. The target must name the index
*and repeat its predicate*, which means a statement can need a different string
depending on whether a column is null — `reading_progress` has two partial unique
indexes and now has two PostgreSQL statements.

**The defect worth remembering is none of those three.** `set_password_hash`
bound one parameter list for two statements whose placeholders are in different
orders. On SQLite this was *silent*: the surplus parameter shifted every value by
one, the `WHERE` compared an account id against a timestamp, no row matched, and
the function returned `Ok(())`. A password change did nothing and reported
success, and every test in the tree passed. PostgreSQL refused it outright, which
is how it was found.

That is the argument for the CI job the workflow already contains, and it is
stronger than "the dialect should be exercised": **the second engine is not only
a deployment target, it is the cheapest available check on the first one.** The
SQLite path had a silent data-loss bug that only a type-checking engine could
surface.
