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

### M31-D03 · `migrate_comments_to_topic` is not transactional

The tool links a topic, then loops over the work's comments: for each one it
creates a forum post and soft-deletes the original. Those two writes and the
per-comment loop run outside any transaction, so a failure part-way leaves some
comments moved and some not, with a topic already created. It is also
destructive to the comment bodies (replaced with a tombstone), so a mid-way
failure loses text that was never successfully copied.

Re-running is safe -- already-migrated comments are soft-deleted and skipped --
so recovery is possible, but it is manual. Wrapping the loop in one transaction
is the obvious fix and the module's own design points at it (`link_topic` and
`create_post_with_timestamp` both take `&Database`, not a `&mut Transaction`, so
they would need transaction-aware variants first).

Idempotency and ordering are pinned by `migrating_twice_moves_nothing_the_second_time`,
`each_comment_becomes_a_post_with_its_text` and
`a_post_keeps_the_comment_s_author_and_timestamp`.

### M25-D02 · `attach_derivative_job` is the one statement that skips `version`

Every other state-changing statement in `derivative.rs` increments `version`:
`mark_derivative_built`, `mark_derivative_failed`, `mark_derivative_stale`,
`touch_derivative_verified`. `attach_derivative_job` does not.

The omission looks deliberate -- its own doc comment frames the job link as
bookkeeping ("the link an operator follows", "the link the verification sweep
uses to skip a rebuild that is already running") rather than a state change,
and it does leave `state` alone. But `version` is otherwise the module's
optimistic-concurrency counter, and a writer that read-modify-writes a
derivative would miss this one.

Pinned as current behaviour by `attaching_a_job_does_not_bump_the_version` so
that a future writer using `version` for concurrency has a test to update
rather than a silent hole. Deciding whether the statement *should* bump it is a
one-word change plus a comment, but it belongs with whoever first adds a
`version`-based writer, not with a test suite.

### M18P59-D01 · `contribute_to_bounty` is not atomic across the funding update and the ledger

`contribute_to_bounty` reads the bounty, computes `funded_amount` in Rust, then
writes the new total, then writes the contribution row — three separate
statements with no transaction and no row lock. Two concurrent contributions
read the same `funded_amount`, both add their own amount, and the second write
wins, so one contribution's credits vanish from the total while its ledger row
remains. The ledger and the total then disagree, which is the one invariant the
table exists to make checkable.

The same window means activation is not safe: two contributions that each cross
the threshold both report `activated = true`.

Fixing it means wrapping the read-modify-write in a transaction and taking
`SELECT ... FOR UPDATE` on the bounty row (a no-op clause to skip on SQLite),
which is a real change to a money path. Recorded, not fixed, pending a decision
on whether crowdfunded bounties can plausibly receive concurrent contributions
in practice.

### M45-D01 · `upsert_card` freezes a shipped card's whole row, not just its stage

§44.6 requires that a `shipped` or `rejected` card's *stage* is never
downgraded. The guard is written as a `WHERE` on the `ON CONFLICT ... DO UPDATE`
clause:

```sql
ON CONFLICT(id) DO UPDATE SET
  title=excluded.title, category=excluded.category, stage=excluded.stage, updated_at=excluded.updated_at
WHERE roadmap_cards.stage NOT IN ('shipped','rejected')
```

A `WHERE` on `DO UPDATE` gates the **whole** update, not one assignment, so once
a card reaches `shipped` or `rejected` its title, category and `updated_at` are
frozen too. A re-seed cannot correct a typo in a shipped feature's name, and an
operator renaming a card through the seeder sees no error and no change.

Guarding only the assignment would look like
`stage=CASE WHEN roadmap_cards.stage IN ('shipped','rejected') THEN roadmap_cards.stage ELSE excluded.stage END`
with the `WHERE` removed.

**Not fixed here deliberately.** This changes what a user sees on a public
board, so it is a product call: either the freeze is intended (a shipped card is
immutable and renames must go through the operator's `update_card_stage` path)
or it is a bug. Pinned by
`a_protected_cards_whole_row_is_frozen` in
`crates/app/tests/milestone_45_roadmap.rs` so the current behaviour cannot
change silently either way.

### M45-D02 · `normalize_title` strips hyphens instead of separating words

`find_card_by_title_normalized` deduplicates cards by normalizing both titles
with `normalize_title`, which keeps alphanumerics and whitespace and **drops**
everything else. A hyphen is therefore removed rather than turned into a space:

| title | normalizes to |
| --- | --- |
| `Dark Mode` | `dark mode` |
| `Dark-Mode` | `darkmode` |
| `DarkMode` | `darkmode` |

So `Dark-Mode` is treated as a duplicate of `DarkMode`, and neither is a
duplicate of `Dark Mode`. For a feature board whose titles are ordinary prose
this silently fails to catch the most common re-punctuation, and the false
merges are only visible if you happen to try them.

Fixing it means replacing punctuation with a space instead of deleting it,
which is a one-line change to the `filter` step. Not done here because it
changes which cards a re-seed merges, so it is a data-migration decision as
much as a code one. Pinned by
`a_title_is_found_despite_case_punctuation_and_spacing`.

### M12-D01 · `notifications.work_id` has no foreign key

Migration `0023_notifications.sql` declares `work_id UUID` with no
`REFERENCES` clause; only `account_id` cascades. Two consequences, both live:

1. A notification can reference a work that never existed, or one that has
   since been deleted. The row survives, the inbox renders, and the link
   404s.
2. Deleting a work does not clean up its notifications, so an author deleting
   a work leaves the reader's inbox pointing at nothing.

Not fixed here, because the fix is a migration plus a decision about existing
rows: a bare `ADD CONSTRAINT ... REFERENCES works(id) ON DELETE CASCADE`
would fail outright if any row already points at a missing work, and the
scratch databases in this suite create exactly such rows. The order that works
is: find and resolve or delete the dangling rows, then add the constraint.
Which of "delete the notification" and "leave the work tombstoned and the link
dead" is right for a reader's inbox is a product call.

Pinned as current behaviour by
`a_notification_for_a_missing_work_is_stored_not_rejected`, so the day the
constraint lands the test flips rather than silently passing.

### M12-D02 · the resolver default and the column default disagree

`settings::resolve_notification_channel` returns `Some("email")` for an account
with no route row, while migration `0071` gives `delivery_channel` a column
default of `'in_app'`. Only the function's value is reachable in normal use --
`notify` always supplies it -- so the column default is a fallback for direct
INSERTs, and an account with no configured routes has its notifications
labelled `email` while the schema's own default says `in_app`.

Neither is wrong in isolation, but a reader inspecting the column directly would
read `in_app` and a reader going through `notify` would read `email`, and
nothing in the code says which is authoritative. Worth picking one and making
the other derive from it.

Pinned by `the_column_default_and_the_resolver_default_differ` and
`an_unset_channel_resolves_to_email`.

### M18-P42-D03 · re-pinning returns an id that was never stored

`roles::pin_work` mints a fresh `uuid` on every call and returns it, but its
`ON CONFLICT(account_id, work_id) DO UPDATE` clause updates `pinned_at`,
`pin_reason` and `message` only — never `id`. So pinning a work a second time
returns a *second* id for a row that only ever had the first, and
`routes::vanguard::pin_work` puts that id straight into its 201 response body.
A client that stored the id from its re-pin holds an identifier that resolves
to nothing.

Not changed, because the fix is a product decision: either the statement
adopts the new id on conflict (`DO UPDATE SET id = excluded.id`, which changes
the row's identity and invalidates any id a client already holds) or the
function returns the *stored* id, which means `RETURNING id::text` and a
signature that can report "the existing pin" rather than always "a new one".
The second is the honest fix; the first is the smaller diff. Worth noting the
route returns 201 either way, which is arguably wrong for a re-pin regardless
of which id it reports — that would be 200.

Pinned by `re_pinning_returns_an_id_that_was_never_stored`.

### M31-D02 · an unreadable `discussion_mode` is indistinguishable from a deleted work

`work_discussion_mode` returns `Option<WorkDiscussionMode>` and builds it with
`value.and_then(|v| WorkDiscussionMode::parse(&v))`, so a stored mode that
`parse` does not recognise collapses into the same `None` as "no row" -- which
is also what a soft-deleted work returns. A caller cannot tell a corrupt value
from an absent one, and falls back to the default (`CommentsOnly`).

Failing that way is the safe direction: an unreadable mode does not silently
become `thread_only`, which would move a work's discussion out from under its
readers. So this is logged rather than fixed -- the fix is a second return
shape (`Result<Option<Mode>>` with a distinct "unreadable" arm) and that changes
every call site. Note the column is `NOT NULL DEFAULT 'comments_only'`
(0038), so "unset" is only reachable via a deleted work.

Pinned by `an_unreadable_stored_mode_reads_as_absent` and
`a_soft_deleted_work_is_not_readable_and_not_writable`.

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
