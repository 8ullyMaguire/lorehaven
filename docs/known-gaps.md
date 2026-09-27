# Known gaps

Defects and open questions found by tests, recorded rather than papered over.
Each one has a test that pins the *current* behaviour, so a fix shows up as a
red test rather than silence. A test named `KNOWN DEFECT` or a `KNOWN DEFECT`
comment in the test body is the entry's executable form.

## Open — product decisions

### M21-D03 · A re-subscribe returns an id that addresses no row

`subscribe_work` mints a fresh `Uuid` and returns it unconditionally, but its
`INSERT … ON CONFLICT (subscriber_pseud_id, subject_type, subject_id) DO UPDATE`
branch keeps the row's *original* id. On the second subscribe the caller gets an
id that is not in the table; `set_subscription_state(that_id)` moves zero rows.
`create_alert` has the same shape for `search_alerts`.

Not fixed, because the fix changes what the function promises. Either the
upsert returns `RETURNING id` so the caller always gets the live id, or the
route stops using the return value and looks the row up by
`(pseud, subject)`. Which one is right depends on whether callers need the id at
all on the re-subscribe path.

Pinned by `a_resubscribe_returns_an_id_that_addresses_nothing`.

### M21-D20 · Dismissing a recorded opt-out deletes the audit trail

`ai_training_opt_out_handler` is documented as "Post an AI-training opt-out
handler" but runs `DELETE FROM author_ai_training WHERE pseud_id = ? AND
opt_in = 0`. The user-facing answer is unaffected — `ai_training_status` treats
a missing row as "not opted in" — but the record that an author *declined* is
gone, so no consent audit trail can be built from this table.

The `WHERE opt_in = 0` guard is at least safe: an author's opt-in survives.

Pinned by `a_recorded_opt_out_can_be_dismissed_and_the_answer_does_not_change`.

### M21-D22 · A delayed opt-out dates the effect, not the request

`ai_training_opt_out_delayed{,_tx}` set `updated_at = now + delay_seconds` — the
time the opt-out takes *effect* rather than the time it was requested. Nothing
in the workspace calls them (the route uses plain `ai_training_opt_out`), so
this is dead code rather than a live bug. Whether `updated_at` should mean
"requested" or "effective" is undecided, and the column is not read by anything
else today.

Pinned by `a_delayed_opt_out_is_effective_immediately_and_dates_the_effect_in_the_future`.

### M18-05 · `expire_claims` re-expires the same claims forever

It selects rows with `fulfilled_at IS NULL` and then writes `fulfilled_at = NULL`
on them, so the next scheduled run matches the same rows and reports them again.
The fix needs a distinct expired state — a schema change, and one that changes
what a re-claim means. That is a product call.

Pinned by the `expire_claims` test in `milestone_18_events.rs`.

### M18-06 · Duplicates are reported two different ways

`fulfil_claim` returns `Err` on a duplicate; `join_event` also has a `bool`
return but errors rather than returning `false`. Both are typed
`Result<...>` with a boolean in the signature, so callers have to handle the
same condition two ways.

### M68-D21 · `increment_counter` interpolates a caller-supplied column name

`work_metrics::increment_counter(db, work_id, column, delta)` is `pub`, takes
`column: &str` and interpolates it into three places in the statement: the INSERT
column list, the VALUES clause, and the `ON CONFLICT DO UPDATE SET` clause. Its
comment says "the column name is validated by the caller (always a literal in
this module)" — but it has no callers outside the module, and `pub` makes it
reachable from anywhere in the crate, so the validation the comment relies on
does not exist anywhere. A caller that passed a request-supplied name would be a
SQL injection.

Not changed, because closing it is a design decision with two reasonable
answers: make the function private and add seven thin wrappers (safest, no escape
hatch), or validate the name against the known counter set and return an error
for anything else (keeps the seed script working). An unknown column name does
fail loudly today, so nothing is corrupting a row — the risk is the shape, not a
live exploit.

Pinned by `increment_counter_will_interpolate_any_column_name_it_is_given`.

### M43-D10 · `regenerate_fingerprint` cannot regenerate

`instance_fingerprints.instance_host` is `UNIQUE` and the INSERT has no
`ON CONFLICT` clause, so the second call for a host is a unique violation. The
function is the only way to refresh a fingerprint, and `get_fingerprint` filters
on `valid_until > now` — so a fingerprint becomes invisible after 30 days and can
never be replaced. Nothing in the workspace calls it yet, which is why it has
gone unnoticed. The fix is a `DO UPDATE` on the host, or dropping the UNIQUE and
selecting the newest row.

Pinned by `regenerating_a_fingerprint_fails_instead_of_refreshing_it`.

### M43-D19 · A failed federation delivery is never retried

`mark_queue_failed` sets `status = 'failed'` and increments `attempts`, but
`get_pending_queue` selects only `status = 'pending'` and the schema has exactly
three states (`pending | sent | failed`). So a delivery that fails once leaves
the queue for good, and the `attempts` counter that exists to drive a backoff is
written and never read. The index is already `(status, created_at)` — the shape
a retry sweep wants — so the missing piece is a retryable state or a query that
also takes `failed` rows below an attempt ceiling.

Pinned by `a_failed_delivery_leaves_the_queue_for_good`.

### M32 · `meta_verdicts` counts the opposite of what its name and doc say

Documented as "a caster's meta-moderation record", the query joins
`forum_votes v ON v.id = m.vote_id WHERE v.pseud = ?` — `v.pseud` is the vote's
*author*, so it counts verdicts cast **on** that author's votes, not that
pseud's own rulings. The whole weight chain hangs off it
(`caster_weight_bp` → `meta_verdicts` → that clause), so "how was I judged as a
voter" and "what weight do I cast at" are currently the same question.

Not changed, because a steward abusing the system looks identical under one
reading and different under the other, so which is intended is a moderation
policy question. Pinned by the `meta_verdicts` and `caster_weight_bp` tests in
`milestone_32_data.rs`, whose names say which reading they assert.

## Fixed

Listed because the pattern recurs and the fix is the reference for it.

* **`category_governance`: six broken PostgreSQL statements.**
  `vote_on_proposal` and `vote_on_entry_mod` were non-functional on PostgreSQL
  entirely, plus four `MAX()`/`SUM()` aggregates decoding `INT4` into `i64`
  fields, and a `merge_categories` that produced redirects pointing at nothing.
* **`thread_modes`: `join_critique` broke on the second member**, PostgreSQL
  only, from an `INT4` aggregate.
* **`subscriptions`: every AI-training consent call was a 500 on PostgreSQL.**
  Three separate sites — the `INSERT` bound a Rust `bool` into a `BIGINT`
  column, and two reads asked sqlx to decode `BIGINT` as `bool`. Invisible on
  SQLite, which stores the bool as 0/1 without complaint.
* **`federation`: every peer query was an error on PostgreSQL.** `similarity` is
  `REAL` (FLOAT4) and the struct field is `f64` (FLOAT8); sqlx matches types
  strictly rather than widening, so both the thresholded list and the full list
  failed to decode. SQLite stores REAL as a double, so only the PostgreSQL run
  caught it. Fixed with `similarity::double precision` in the PG arms only.
* **`subscriptions`: the delayed opt-out had no `text + interval` operator on
  PostgreSQL** (`updated_at` is RFC 3339 TEXT). The arithmetic now happens in
  SQL and is cast back to text, so the two dialects agree.

The recurring cause in every case above is the one in `ADR 0004` and house rule
§2.1: SQLite's untyped integers and booleans decode into anything, so a missing
cast or a wrong bind type is invisible locally and a 500 in production. A
PostgreSQL run is not optional.
