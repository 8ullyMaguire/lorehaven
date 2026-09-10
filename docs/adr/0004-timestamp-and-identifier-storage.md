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
  integration tests run today**, because the development machine has no server
  installed. `docs/verification.md` records this as *implemented but not
  executed*, and the migration catalogue is kept identical by test so the
  divergence cannot silently widen. This is the largest open risk in the
  database layer.
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
