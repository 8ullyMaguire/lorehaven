# Handoff — content filters finally hold: paging, PostgreSQL, and a gate that lied

## The short version

Content filters were broken in two ways that mattered, both of them invisible on
SQLite. Fixed, tested on both backends, and the exclusion now runs *inside* the
paged statement instead of after it.

The product gap is still open and is larger than the bugs I fixed: **filters are
enforced on exactly one surface of four.** That is the next work item, and it is
a direct violation of the spec's own invariant.

## The two bugs

**Filtering ran after `LIMIT`.** `search_works_ast_filtered` paged first and
applied the filter in Rust, so a page of 20 that lost 15 works to a filter
answered with 5 rows. A reader who blocked a common tag saw a short page and no
indication that more matched. The exclusion is now a correlated `NOT EXISTS`
against `work_tags`/`taxonomy_nodes`, rendered inside the statement, so it applies
before the limit — which also removes the N+1 (it cost one query per result row).

**Content filters did nothing at all on PostgreSQL.** `work_tag_values`'s PG arm
used `wt.work_id::text = ?` — a bare `?`, which is not a bind parameter in
PostgreSQL at all. The server answered `syntax error at end of input`, the caller
swallowed it with `unwrap_or_default()`, and every reader's filters were silently
empty.

**And my fix for that reintroduced the same class of fault one layer up.** The
old code's `::text` was on the *bind* side, which is why it was legal: it turned
a String bind into something comparable to a UUID column. When I rewrote the
predicate I put `::text` on the *column* side, `cf_wt.work_id::text`, reasoning
that `work_tags.work_id` was `TEXT` like its sibling `node_id`. It is `UUID` —
`node_id` is the `TEXT` one, so the table holds one of each — and the comparison
became `text = uuid`. All four content-filter tests 500'd on PostgreSQL and passed
on SQLite. It sat in a commit for one run before PG caught it.

The gate's self-test now pins `work_tags.work_id` as `uuid` and
`work_tags.node_id` as `text` so that inference cannot come back. And a regex
detail worth copying: `(?<![:\w])` before the column name in the cast rule.
Without it, `work_id::text = $1::uuid` matches with the column read as `text`,
so a self-test asserting on this exact bug passes against a gate looking at the
wrong column — which is exactly what happened on the first attempt. Same defect in four `settings.rs` reads (search settings, content filters,
notification routes, and both DELETEs) and one `dnf.rs` read. All now go through
`db.sql`.

Two latent bugs surfaced while fixing them, both in the signed-in PG arm that had
never run: `blocks.blocked = pseuds.account_id` compares `TEXT` to `UUID`
(`operator does not exist: uuid = text`), and `SUM(word_count)` returns `NUMERIC`
in PostgreSQL so the `::bigint` cast is load-bearing for decoding. Both are fixed
per-dialect with the reason in a comment.

## The new file

`crates/db/src/search/content_filter_sql.rs` — one place that knows how to render
the exclusion, so a surface cannot forget a dialect. `predicate` / `predicate_pg`,
`binds`, `for_pseud`, `by_type`. Both statements are written with positional `?`
and handed to `sql_owned`, which renumbers only the PG one; the previous code
renumbered just the user fragment and hand-computed the `LIMIT` index, and the two
arms bound the viewer a different number of times, which is the kind of asymmetry
that works until a bind moves.

## The test that makes the paging fix provable

`content_filters_narrow_the_page_instead_of_shortening_it` (milestone_10.rs). The
existing `search_respects_content_filters` seeds two works and asks for a
one-item page, so a correct implementation and a post-`LIMIT` one both answer with
the single unblocked work — it cannot tell them apart. The new one seeds 12,
blocks 7, orders them so the blocked works lead the unfiltered page (empty term
index ⇒ every score is 0 ⇒ the order is `updated_at DESC`), and asserts a *full*
page of clean works. Under the old implementation it returned nothing. Verified
by reverting the exclusion and watching it fail.

Note: site search answers `items`; the `titles` helper in that file reads
`results`, which is the in-work search envelope.

## The checker was wrong, and that mattered more than the bugs

`scripts/check-uncast-pg-placeholders.py` was green for a long time while judging
the **SQLite** arm of every `db.sql(a, b)` pair. Root cause: the argument walk
compared a call-relative index against a file offset, adding `open_at` to a value
that was already absolute. So the separator comma looked as though it lay after
the literal, and both halves reported as the first.

Three related defects in the same function, all found the same way:

- `sql_owned(db, a, b)` leads with the database, so its SQLite half is argument
  **1**, not 0. Conflating the two layouts judges every pair backwards.
- Apostrophes in prose (`the reader's`) opened what the scanner read as an
  unterminated string, losing every argument boundary after them.
- Quoted identifiers inside comments were matched as literals.

Four of my own "fixes" to this function made things worse before I found the
actual cause — the tell was that the gate went from flagging 1 site to flagging
**none**, and a lint that fails quiet is worse than no lint. `in_line_comment`
and the `sql_owned` offset are now covered by `ARM_TEST_CASES`, which feeds whole
calls instead of single strings. Two of those cases do not yet fail when the
logic they cover is broken — a real coverage gap, noted rather than papered over.

**Lesson worth keeping: prove a gate goes red before trusting it green.** Every
"the gate passes" claim in a handoff should name a site it caught.

## Baseline, measured

Six PostgreSQL failures were standing at the start of this change, each
confirmed against a clean HEAD by `git stash` and a re-run. They were real
defects, and five of the six are now **fixed** rather than baselined:

    export_import_round_trip            JSONB read as String  -> cast to ::text
    my_audit_log_filters_by_account     JSONB read as String  -> cast to ::text
    translation_memory_...              BIGINT read as i32    -> read i64
    a_group_is_created_...              text = uuid          -> drop the casts
    curator_bounty_queue_...            text = uuid          -> drop the casts
    repeated_login_attempts_...         not a defect; timing, see above

The last one is worth knowing: it takes 83 s on its own and is starved by a
parallel suite. Use `-- --test-threads=2` for a full run on this machine, and
treat "0 passed; N filtered out" as a filter that matched nothing, not a pass.

Do not "verify" a baseline by guessing which file a test lives in — two of the
six read as passing under a `--test` filter that matched no test at all, which
is how a baseline check quietly proves nothing. Match the test *name* to its
file with `grep -rl` first.

## A second gate, and six more PostgreSQL-only 500s

Fixing the four `::uuid`-on-a-`TEXT`-column bugs was not a lucky guess. I built
the column type map straight out of `migrations/postgres` (1,804 columns, 297 of
them `uuid`) and scanned every SQL string for `$n::uuid` compared against a
column declared `TEXT`. Six were live:

    community.rs      groups.id, groups.owner, group_members.account x2, group_id
    federation.rs     ap_follows.id, federation_queue.id x2
    taxonomy.rs       taxonomy_nodes.id  (its `id::text` cast went too)

Each is a 500 that only ever appears on PostgreSQL. This is what the
"baseline" of six failing tests was made of: the three group/audit/translation
ones are now **fixed**, not baselined. `repeated_login_attempts_are_rate_limited`
is not a defect — it needs 83 s alone and starves when the suite runs
`-j`-wide; it passes in isolation and under `--test-threads=2`.

The script is `scripts/check-pg-uuid-casts.py`, wired into CI next to the
placeholder checker. Two things it has to get right, both of which I got wrong
first:

- **Alias resolution.** A statement touching six tables must not blame all six.
  `p.account_id = $1::uuid` where `p` is `pseuds` is correct even though
  `category_votes.account_id` is `TEXT`. Naive cross-matching reported **81**
  findings, most of them false; alias-aware reports 6, all real.
- **Unqualified columns are only reported when every table in the statement
  agrees** they are `TEXT`, so a join cannot manufacture a finding.

Its `--self-test` is 6 cases and it is verified to go red: reinstating the
`federation_queue` cast flips it to rc=1, and the tree is clean at rc=0.

## Filters now bind the recommendation surface — and the pseud is the key

All three recommendation engines enforce content filters. The interesting part
was not the SQL.

**`for_pseud` is the only lookup, and that is a design decision, not an
omission.** A filter belongs to a pseud; which pseud is a *session* property
(`sessions.active_pseud_id`, switchable per session) while an account can have
several. I first wrote a `for_account` that resolved account → pseud, then
deleted it: there is no such column, and picking an arbitrary pseud from the
account's several means applying that reader's *wrong* filter set. So the engines
take `Option<Uuid>` — the pseud the session is acting as — and the route resolves
it exactly the way `routes/settings.rs` does, including the fall back to the
account id when a session carries no pseud. The two must agree: if they did not,
a filter would apply to search and not to recommendations, which is the bug.

**The exclusion correlates on an expression, not a table.** Two engines write
`FROM works w`; the media-reference engine selects `work_media_references` and
groups on `wmr.work_id`. So the predicate takes a work-id *expression*. This is
not a stylistic choice: passing `w.id` to the media-reference query is
`no such column: works.id` on PostgreSQL, and the whole route 500s. I got this
wrong first — the search path's default was `"works"`, not `"works.id"`, which
SQLite tolerates and PostgreSQL rejects. Verified by mutation in both directions.

**`Exclusion::clause()` exists because a spliced empty predicate is a footgun.**
The first version wrote `AND {filter_pred}`, and with no filters the statement
ended in a dangling `AND` — a syntax error in the common case, the one no positive
test covers. The fragment now renders its own conjunction, so a caller writes
`{filter_clause}` and cannot get it wrong.

**Bind order is positional and unchecked.** The exclusion's `?` sit in the WHERE,
ahead of `LIMIT ?`, so they bind ahead of it. I had this backwards in three
places at once; a swapped pair is a wrong answer, not an error. The reason it
cannot be shared into a helper: `sqlx::query_as` monomorphises on the backend, so
one built query cannot cross a `SqlitePool` and a `PgPool`, and the generic
helper that would fix the duplication is rejected as a bound cycle. The
duplication is commented as deliberate.

Two new tests, each verified to go red by mutation:

- `content_filters_reach_the_recommendation_surface` (milestone_10.rs) — the two
  `works`-shaped engines. Removing the enforcement restores the blocked work.
- `media_ref_collab_applies_content_filters` (media_ref_collab.rs) — the engine
  whose work id is not on `works`. Correlating on `works.id` there gives
  `no such column: works.id`. Note every other test in that file passes `None`,
  so nothing else would have caught the wrong expression.

Plus 7 unit tests on the predicate itself, including one that both dialects bind
identical values for identical rules.

## Still open

1. **M29 recommendation transparency is not implemented at all.** Both endpoints
   are fabricated, and this is the next milestone-sized piece of work.

   `GET /discovery/slots/{slot_id}/explanation` returns a hardcoded
   `SlotExplanation` for *any* `slot_id` — `"taste_signal: matching your reading
   history"`, every other field `None` — touches no database, and takes
   `MaybeSession(_session)`, so it cannot even tell who is asking.
   `GET /me/attention-report` returns `{enabled: false, lines: null}` regardless
   of the reader's settings.

   Worse, there is nothing for either to be right about. Grepping the tree for
   `slot_id` finds 5 hits in the route file and 1 in a test; there is no table,
   no writer, and the discovery route never emits a slot id. The blend in
   `routes/discovery.rs` is in-memory per request, so nothing about a slot
   survives to be explained afterwards. The endpoint is unreachable in practice
   *and* unanswerable — a client cannot obtain a `slot_id` to ask about.

   What §33.3 actually requires, since it settles the shape:
   "Every recommended slot can name its reader-side reasons"; the explanation
   "must never reveal the administrator's taste"; and for the attention report,
   "private to its reader, includes at least one 'held back by your own
   settings' line, and stays disabled until the reader enables it." The
   `instance_curation` field's comment ("always shown as one undifferentiated
   line") is a §29.2 arena-language requirement already half-honoured in the
   struct.

   The design decision that has to come first, because everything else follows
   it: **persist the candidate set, or compute the explanation on demand?**
   Persisting means a new table keyed by slot with a retention window, and it is
   the only way to answer "why *this* one" after the request that produced it has
   gone. Computing on demand means re-running the blend for the slot's inputs and
   recording what the inputs were, which is cheaper but only correct if the
   engines are deterministic for a recorded input — and they are not obviously
   so, since the RRF blend depends on a registry that can be reconfigured between
   requests.

   The second decision is narrower and I have a recommendation: for a work the
   reader has content-filtered, the explanation should return 404, not a filtered
   reason. A 404 leaks that the work exists, but a reason that names the matching
   tag leaks the same fact more explicitly, and §46.7.1 is about the work never
   reaching the client at all. The honest answer is that a filtered work should
   not have a slot to explain, and that is achieved upstream by the filter work
   in this commit.

2. **No lint for a bare `?` reaching a PG pool.** That is the defect class that
   broke filters. I tried a regex rule and abandoned it: "is this literal the
   PostgreSQL arm" is not decidable from the text (three legal call shapes,
   literals on the line after their binding, fragments interpolated into callers'
   statements), and four heuristics in a row each reported 100–500 correct
   statements as faults. A reachability check is the right shape; a regex is not.
3. **Keyword filters and shareable/followable filter lists** — neither exists and
   the spec describes neither. Answers in the thread above; the design decision
   that needs your call is copy-on-follow vs live subscription, whether a list may
   carry *user* blocks (it should not — `blocks` already owns that with four mute
   scopes), and who authors a matched block for appeal purposes.

## How to resume

```
cd /home/alvaro/code-local/rust/lorehaven
export CARGO_TARGET_DIR=$HOME/.cargo-target/lorehaven-pg
cargo fmt --all && cargo build --workspace --tests
for g in check-uncast-pg-placeholders check-pg-uuid-casts; do
  python3 scripts/$g.py --self-test    # 25 and 6 cases respectively
  python3 scripts/$g.py                # must be silent and exit 0
done
export LOREHAVEN_TEST_PG_URL='postgres://lorehaven:<pw>@127.0.0.1:55433/postgres'
cargo test --workspace --no-fail-fast
unset LOREHAVEN_TEST_PG_URL && cargo test --workspace --no-fail-fast
```

**Give every checkout its own `CARGO_TARGET_DIR`, including `git worktree`.**
Setting up a worktree of the previous commit to get a pre-change baseline, I left
it on the default `CARGO_TARGET_DIR` — the same one the main tree used. Cargo
keys its fingerprint cache on the *package path*, not the source root, so the
worktree's build overwrote the main tree's `lorehaven-app` lib artifact with one
compiled from three-argument engine calls. The main tree then reported
`E0061: unexpected argument #3 of type Option<Uuid>` at a call site I had
already fixed, in code I had already built clean minutes earlier. It cost about
twenty minutes of chasing a nonexistent regression before I noticed that
`/tmp/lh-prev/crates/db/src/discovery.rs` has the *old* three-argument signature.

If a build error contradicts something you verified minutes ago, suspect a shared
target dir before you suspect your own edit. Check with a build in a fresh
`CARGO_TARGET_DIR`; if that passes, the error was never yours.

**Compare full runs only, and always against the baseline commit.** Mid-run I
diffed a partial current run against a *previous partial* run and read the
difference as 7 new regressions. They were not: all 7 (`milestone_32` voting,
karma, taxonomy override) fail identically at the baseline commit, where they
were simply buried inside the 54. Against the real baseline the honest number is
6 fixed, 0 regressions, 48 still failing.

The lesson is narrow and worth keeping: a set difference between two runs is
only meaningful if both runs completed. A truncated run is a subset, and
`A - B` over subsets produces phantom regressions that cost an hour of chasing
a 500 through code I had never touched.

**The unread count is filtered, and that is a deliberate mismatch.** A reader
who blocks a tag sees `unread_count` drop below the number of entries in the
list. That looks like a bug and is not: the alternative is a badge that counts
a notification the reader cannot open, which leaks the existence of the hidden
work through one digit. If someone "fixes" this by counting unfiltered rows, the
test `content_filters_reach_the_inbox_and_the_unread_count` fails on the count
assertion — which is what it is for.

**Positional placeholders cannot be numbered when the bind count is dynamic.**
`notifications::list_filtered` appends the content-filter's binds in a loop, so
its SQL uses unnumbered `?` and lets `sql_owned` renumber the PostgreSQL form.
I first wrote `WHERE account_id = ?1 ... LIMIT ?2` — and SQLite answered
`datatype mismatch`, which points at nothing useful. `?2` is fixed but the
filter contributes an unknown number of binds, so the two numbering schemes
collide. The error names a type when the fault is a count.

**A test that pins a schema fact can encode the wrong one.** When I removed the
bad `work_id::text` cast, three unit tests in `content_filter_sql.rs` went red —
not because the fix was wrong but because they asserted the pre-fix SQL text,
including the comment "`work_tags.work_id` is TEXT and `works.id` is UUID". That
comment was my own wrong inference, and I had written a test around it. They
also asserted on literal aliases `wt`/`tn` when the code uses `cf_wt`/`cf_tn`
constants, so they were only ever passing by coincidence of an earlier shape.

The rewrite pins what is actually true and cannot drift: the correlation is
`cf_wt.work_id = <caller's work column>` with no `::text` anywhere, for every
alias. `render` is dialect-independent, which let the whole `_pg` half of the
module go -- `predicate_pg`, `build_pg`, `build_for_pg` and the `match
db.backend()` in `exclusion_for`, whose `db` argument three callers were passing
purely to select an arm that no longer exists.

Assert on behaviour and on facts read from the migrations, never on the text of
a comment you wrote. If a test fails the moment you fix the thing it describes,
the test is the defect.

Use `-- --test-threads=2` on both runs. Fully parallel, the suite starves the
rate-limit test (83 s on its own) and it fails for timing, not logic.

The PG URL lives in the session env; the container needs `--shm-size=512m` or
scratch-DB creation starves and every binary reports 0 passed.

# Handoff — the same class of fault, run in the opposite direction

## The short version

The previous session closed "a TEXT bind into a typed column". This session found
the mirror image: **a typed bind into a TEXT column**, which nobody was checking
for, and which had disabled the entire job runner on PostgreSQL.

`fix-timestamptz-binds.py` existed to add `::timestamptz` to every `_at` bind. Its
docstring asserted the reason: *"the SQLite schema spells the timestamp columns
TEXT; PostgreSQL types them TIMESTAMPTZ."* The first half is right. The second is
false — this project's PostgreSQL migrations mirror the SQLite types exactly, so
`jobs.available_at`, `outbox_events.created_at`, `comments.created_at` and about a
hundred more are all `TEXT`. Every cast the tool added was a defect:

    operator does not exist: text <= timestamp with time zone

26 statements across 24 files carried one. The job runner was the worst of it —
`claim_next`, every lease renewal, the expiry sweep, the retry schedule — so on
PostgreSQL nothing was ever claimed and no job ever ran. The tool is now disabled
(commit `3b9cc87`); only `--self-test` still runs.

**If you take one thing from this:** never infer a column's type from its name, its
`_at` suffix, or what the other dialect does. Read it out of `migrations/postgres`.
`check-uncast-pg-placeholders.py` now does exactly that, and that is the only reason
these are found rather than guessed at.

## The two other faults, same root

Both are "assumed the type instead of reading it":

- `work_view_log.is_automated` and `collections.is_public` /
  `work_contributors.public_attribution` are `INTEGER`, not `BOOLEAN`. Flag columns
  are spelled `BOOLEAN` in only 19 of the migrations, so neither spelling can be
  assumed. A Rust `bool` bind and a `= true` comparison are both rejected:
  `bigint = boolean`. This broke the anonymous work page, which is the first
  request a crawler makes.
- `work_kudos.created_at` and `work_metric_aggregates.updated_at` are `TEXT` and
  were written with a bare `now()`. `NOW_TEXT` in `work_metrics.rs` renders it in
  exactly the form SQLite's `datetime('now')` produces, so a row written by either
  dialect sorts and compares identically.

## Two diagnostics, so the next one is ten minutes not two hours

Locating a dialect fault by reading candidate SQL does not work — there are 250
tables and both spellings appear in the same file. Two permanent hooks:

- `LOREHAVEN_TRACE_SQL=1` with `RUST_LOG=lorehaven_db=debug` makes
  `Database::sql` log every PostgreSQL statement it builds. The failing one is the
  last line before the error. (It logs nothing for hand-written `sqlx::query`
  arms, which is itself the signal that the statement bypasses `db.sql`.)
- `public_view` names the step that failed. It fans out to four independent
  queries and a fault in any of them was an anonymous 500.

## The checker now has 24 self-test cases, and the negative cases are the point

Two rules were added, each with a negative case proving it does not fire on correct
SQL:

| fault | reported | must NOT report |
|---|---|---|
| uncast bind → typed column | `jobs.updated_at = $1` | `... = $1::text` |
| `::timestamptz` → TEXT column | `jobs.updated_at = $1::timestamptz` | `local_mirrors.expires_at = $1::timestamptz` (that column really is TIMESTAMPTZ) |
| boolean literal → integer column | `c.is_public = true` | `reviews.is_public = true` (that column really is BOOLEAN) |

Both new rules resolve table aliases, so `JOIN collections c ON ...` followed by
`c.is_public = true` is judged rather than skipped. Alias resolution also fixed
`work_backlink.rs`, which had the same integer-flag fault and had been passing
only because nothing looked.

## Also fixed, both pre-existing

- `find_curator_bounty_queue`'s **SQLite** arm carried `::text` and `::bigint`
  casts. SQLite cannot parse them (`unrecognized token: ":"`), so the curator
  bounty queue was a 500 on SQLite. The casts belong to the PostgreSQL arm.
- `find_by_perceptual_hash`'s SQLite arm had `CAST(width AS BIGINT)` with no
  alias, so the result column was named after the expression and the row reader's
  `r.get("width")` was `ColumnNotFound("width")`. Reverse image search never ran.
  Each cast now carries its column name back.

## Where things stand

SQLite: green. PostgreSQL: `milestone_10` 21/21, `milestone_21` 23/23,
`work_metrics` 3/3, `discovery` 3/3, `curator` 5/5.

---

## What this session found, in the order it had to be found

Writing the §32.7.2 curator routes produced a test suite that passed 9/9 on SQLite
and failed 6/9 on PostgreSQL, all with `401 authentication required` on routes the
caller had just authenticated for. Four independent defects were stacked on top of
each other; fixing any one of them changed the symptom from one failure to another,
which is what made this slow.

1. **My own test harness** hardcoded a `sqlite://` URL in `Config::database` while
   the pool was PostgreSQL, so the app read a database nobody had migrated. Every
   authenticated call 401'd for a reason that had nothing to do with sessions.
   `Harness::new` now branches on `TestDb::is_postgres()`.

2. **`create_session`** bound `created_at` / `last_seen_at` / `expires_at` bare.
   Harmless by accident: those columns are `TEXT` in `migrations/postgres`, and
   the earlier session in this repo had been "fixing" them with `?::timestamptz`.
   Fourteen such casts are now removed from `crates/db/src/sessions.rs`. They were
   the actual cause of the 401s: `expires_at > ?` compared TEXT against a coerced
   timestamptz, matched nothing, and the lookup reported the session as absent.
   `milestone_3` on PostgreSQL goes **0/11 to 10/11** from this one fix.

3. **`require_operator`** in `media_resilience.rs` and `media_health.rs` answered a
   signed-in non-curator with `AppError::AuthRequired` (401). `RequireSession` had
   already proven the caller was authenticated, so 401 was both wrong and
   actively misleading -- a logged-in reader was told to log in again. Both now
   return `AccessDenied` (403), which already existed in `AppError`.

4. **The detector was lying.** `check-uncast-pg-placeholders.py` skipped any
   statement containing `::` anywhere, on the theory that one cast meant casts
   everywhere. `create_session` casts three UUID binds and left three timestamps
   bare in the same statement, so the one statement with the live bug was skipped
   wholesale and the checker reported OK. The guard is gone; the per-placeholder
   comparison and INSERT-position checks do that job properly.

## The lesson, since it has now cost two sessions

Every check in `.github/workflows/ci.yml` had a hole, and the pattern is the same:
a heuristic that was reasonable when written, never tested, and impossible to
distinguish from working. `fix-timestamptz-binds.py` grew a `--self-test` last
session. This session the uncast checker grew the same thing, and it immediately
found that the fix it generates was **not idempotent** -- it edited the string while
iterating matches over it and turned `work_id = $2` into `work$2::uuidd`.

If you touch a checker here, add its negative cases in the same commit. A checker
with only positive cases is a checker nobody can trust to say "OK".

## Current state, measured

- `cargo test --workspace` on SQLite: **1811 passed, 0 failed**.
- The uncast-placeholder checker is **clean for the first time** (0 sites), and
  `check-uncast-pg-placeholders.py --self-test` runs in CI ahead of the check.
- §32.7.2 is complete end to end: the fetch job proposes, the curator queue lists,
  a decision confirms or rejects, and the whole lifecycle is covered on both
  backends (`media_resilience.rs` 34 db-level, `media_match_proposals.rs` 9
  route-level, both 9/9 and 34/34 on SQLite and PostgreSQL).

## Still open, honestly

- `taste_engagement` (3) and `media_health` (6) still fail on PostgreSQL. Not
  investigated this session. `milestone_3` is down to 1.
- The PostgreSQL route suites have never all been green in one run, so treat any
  "the suite passes" claim that does not name the backend as unverified.

## How to resume

```bash
cd ~/code-local/rust/lorehaven
export CARGO_TARGET_DIR=$HOME/.cargo-target/lorehaven
python3 scripts/check-uncast-pg-placeholders.py --self-test   # the checker's rules
python3 scripts/check-uncast-pg-placeholders.py crates        # the real gate
export LOREHAVEN_TEST_PG_URL='postgres://lorehaven:lhreview@127.0.0.1:55433/postgres'
cargo test --workspace
```

---

# Handoff — the PostgreSQL cast class, and one structural gap in the schema

Date: 2026-09-25. **The current entry.** Supersedes the 1637/151 baseline below,
which is now stale. `docs/postgres-migration-repair.md` is still the authority
on how the migration chain broke; that history has not changed.

## The state, measured not estimated

SQLite: **1788 passed, 0 failed, 83 suites.** `cargo fmt` clean,
`cargo clippy --workspace --all-targets -- -D warnings` clean, 0 warnings.
(Note for the next session: `-D warnings` must come *after* `--`, or clippy
rejects it as an unexpected argument and exits 1 having checked nothing.)

PostgreSQL: **1660 passed, 130 failed, 33 suites** carrying a failure, measured
by `cargo test --workspace --no-fail-fast` with `LOREHAVEN_TEST_PG_URL` set. The
worst five are `milestone_5` (16), `milestone_6` (14), `milestone_7` (9),
`milestone_40` (8), `milestone_32` (7). (I first wrote this section from a run
that had been cut off at 25 suites, which read 1762/125 across 30 — the 83-suite
run is the number to trust, and the lesson is not to quote a partial one.)

An earlier note in this file's history gave 1762/151; both figures were from
partial or per-suite runs. The 1660/130 above is the first full-workspace
measurement.

## What was actually wrong, and it was not what I first assumed

I spent a long stretch of this session on a wrong theory, so the record is worth
keeping. The 27 failures whose panic message was the bare string `sqlite` looked
like test fixtures reaching for `db.sqlite_pool()`. They were not: every one was
`crates/db/src/thread_modes.rs`, **production code**, where `create_forum_topic`
and `add_schedule_section` each had a `Backend::Postgres` arm whose statement was
written for PostgreSQL (`$1::uuid`) and whose executor was
`db.sqlite_pool().expect("sqlite")`. Under SQLite that arm is never taken, so
every SQLite test passed. Two tokens of production code, 27 dead tests.

`scripts/check-pg-arm-uses-sqlite-pool.py` is the permanent gate — a brace-depth
scan rather than a regex, because ~800 of the 824 `sqlite_pool()` uses in
`crates/*/src` are legitimately inside `Backend::Sqlite` arms. Verified as a gate
by running it against the pre-fix file: it reports exactly those two sites, at
the right lines.

## The class that dominates: TIMESTAMPTZ

PostgreSQL types the timestamp columns `TIMESTAMPTZ`; the SQLite schema spells
the same columns `TEXT`; the whole codebase binds an RFC-3339 string. So every
one of these passes on SQLite and fails on PostgreSQL:

    42804: column "updated_at" is of type timestamp with time zone
           but expression is of type text
    42883: operator does not exist: text <= timestamp with time zone

**82 sites remain**, all one shape: an `_at` column bound to a placeholder in a
PostgreSQL arm. The one that bit hardest was `jobs.requeue_expired_leases`,
which runs inside the worker pass — so it took down *every* test in a suite
rather than one assertion, and said `operator does not exist` rather than
anything pointing at a cast.

**The cast follows the Rust type, not the column.** TIMESTAMPTZ feeding a
`String` needs `::text`; INTEGER feeding an `i64` needs `::bigint`. Backwards is
a real trap: `version` was an INT4 read into an `i64`, which fails even though
the value is text-compatible.

Two structural notes, because they are why the class keeps reappearing:

- `jobs.rs::claim_sql` interpolates one `where_clause` into both dialects, so its
  cast must be chosen per backend inside the string builder, not written into the
  shared clause.
- A hand-written `Backend::Postgres` arm executed via `sqlx::query` directly gets
  no `?`→`$n` rewriting, so a `?::uuid` there reaches the server as a literal
  question mark. `sqlx::Either` is not a way out: the `either` crate has no
  `Executor` impl and `sqlx::Any` is not enabled here.

## A gap in the parity test, now closed

`the_two_dialects_declare_the_same_columns_and_indexes` compared table names,
column names and index names. It passed for all 74 migrations while **12 tables
declared `REFERENCES` in one dialect and not the other** — `payment_events`
among them, which is how a test inserting invented UUIDs was accepted on SQLite
and rejected with 23503 on PostgreSQL.

The parser now records foreign keys, including the `ALTER TABLE ... FOREIGN KEY`
spelling: a circular reference cannot be declared inline, and the two dialects
disagree about which to use for `chapters.current_revision_id`. The divergences
are listed in `KNOWN_FK_DIVERGENCES`, and a test asserts that list equals the
actual divergence — so a table that gets *fixed* fails the test instead of
keeping a stale allowance, and a new divergence fails too.

**Closing the gap is not done, and I judged it out of scope here.** It means a
12-step SQLite table rebuild per table, which needs `PRAGMA foreign_keys = OFF` —
a no-op inside the transaction `migrate()` runs each migration in. So it is a
decision about the migration runner first, and that decision is next.

## The detector, and a lesson about it

`scripts/check-uncast-pg-placeholders.py` reads the migrations and reports
placeholders bound to columns the PostgreSQL schema types. Its INSERT branch
**never fired**: `INSERT_COLUMN` was `^\s*(col)...(,|$)` under `re.MULTILINE`, so
`^` matched only a line start and the function returned the first column and
nothing else. Two more bugs in the same function — `split("VALUES", 1)` with
`rindex("(")` picked the wrong paren, and the column list's trailing `)` stayed
inside the slice, defeating the `(?=,|$)` lookahead and silently dropping the
last column, always `updated_at`.

It now reports 54 statements across 13 files; it was reporting 44, of which every
INSERT was a false negative. **A detector that reports "OK" is worth nothing
until you have watched it catch a real defect** — the way I found this was by
asking why it had missed `user_devices`, not by reading the code.

## What I tried and abandoned

I wrote `scripts/add-timestamptz-casts.py` to apply the 82 casts mechanically. It
corrupted two files in a dry run before I applied it anywhere: it wrote at byte
offsets computed from overlapping call spans, and `cargo check` reported
`character literal may only contain one codepoint`. I deleted it. **The casts are
hand-written and the script is not in the tree.** A script that edits SQL strings
by offset needs its own test suite before it goes near the repo; a half-applied
cast is a silent corruption, not a compile error.

## Next step, in order

1. **Fix `migrate()`'s transaction handling so `PRAGMA foreign_keys` works**,
   then add the 12 missing SQLite foreign keys and empty `KNOWN_FK_DIVERGENCES`.
2. Work the 82 TIMESTAMPTZ sites **by suite**, running that suite on PostgreSQL
   after each file. Do not batch them — worker-pass sites mask everything else in
   a suite, so a batch looks like no progress and then like everything at once.
3. Only then re-measure. Per-suite numbers are the useful signal; one workspace
   total hides which file you just fixed.

---

# Handoff — PostgreSQL migration chain repaired (P1); dedup branch still owed

Date: 2026-09-25. **Read `docs/postgres-migration-repair.md` first** — it is the
authority on what was broken and what the baseline is. Commits `de9555a`,
`6789f5a`, `55d1c7e`, pushed to both remotes.

**M32-07e did not get written, and that is the right outcome.** I set out to
implement the spec's §32.7.2 perceptual dedup branch. Before writing a line of
it, the FK-target checker flagged `device_deliveries.export_job_id ->
export_jobs_old` — a table that does not exist — and following that thread
turned up something much larger: **the PostgreSQL migration chain has never
applied past migration 0041.** Every test that opens a scratch PG database dies
during `migrate`, before a single assertion.

Seven `TEXT` columns referencing `UUID` primary keys, across 0041 and 0042. Plus
`device_deliveries` broken differently in each dialect by 0035 — SQLite renamed
the table and the rename rewrote the referencing FK onto a dropped table; PG
dropped the table outright and never recreated it. All of it shipped green
because **nothing in the codebase inserts into that table**.

**The number that matters:** with the chain repaired, the full PG suite runs for
the first time — 1637 passed, **151 failed**, against SQLite's 1785 / 0. Those
151 are not regressions; they are the first honest measurement of a dialect
this project ships and has never run. All 20 of the uuid-vs-text errors are in
test files, none in production.

**The defect class worth knowing:** `TestDb::sql()` renumbers `?` to `$n` for PG
but does not cast, so a test binding an id as `&str` into a `UUID` column passes
on SQLite and dies on PG with 42804. The house fix is the two-arm
`db.sql(sqlite, postgres)` form spelling the PG arm `?::uuid`, as
`create_export` does. `crates/app/tests/device_delivery_fk.rs` is the reference
implementation, verified 3/3 on both dialects.

## Gate

SQLite: fmt clean, clippy 0/0, suite re-running at commit time. Device-delivery
suite: 3/3 SQLite, 3/3 PostgreSQL 17.11. FK-target checker reports clean.

## What is still owed, in order

1. **The 151 PG failures.** Mechanical but not small: dual-arm the test
   fixtures, fix the `boolean`/`timestamptz` columns, remove the stray `::` in
   `milestone_22.rs:541`. Its own milestone, gated on both dialects every time —
   the whole failure mode here was running one.
2. **M32-07e itself**, still unwritten. `migrations/*/0074_media_match_proposals.sql`
   is staged and uncommitted — the table for the spec's curator-confirmation
   branch. `find_media_reference_by_content_hash` still has zero callers and both
   `require_curator_confirmation_*` config fields are still read by nothing.
3. `phash`/`whash`/`ahash` unimplemented; `audio_fingerprint` unapplied to audio;
   the worker not exercised in E2E.

Nothing deployed. `docs/handoffs/2026-09-25T181500+0200-m32-07d-media-fetch-job-handoff.md`
is the last feature handoff and is still accurate for the media work.

---

# Handoff — M32-07d: the media fetch job, driven end to end

Date: 2026-09-25. **The current, full handoff is
`docs/handoffs/2026-09-25T181500+0200-m32-07d-media-fetch-job-handoff.md`
— read that one.** It records spec §32.7.2's last structural gap closed:
`JobKind::MediaFetch`, the worker handler that runs guard → resolve → fetch →
classify → read bounded → fingerprint → record, and the enqueue in
`add_media_reference` — whose comment already promised "content hash computed
async by the pipeline". `record_fingerprint` now has a production caller.

**The decision worth reviewing.** Testing the handler against a loopback server
required touching the SSRF guard. My first cut branched *around* it, which is
wrong regardless of the fact that it also failed: a bypassed guard leaves a
second, untested path through the most security-sensitive function in the chain.
The working shape is `plan_fetch_allowing(url, allow, timeout)` with
`plan_fetch` delegating to it with an empty slice, so production has exactly one
path and the allowlist applies to both the literal-address and the DNS-resolution
check.

**Still not shipped:** `phash`/`whash`/`ahash` unimplemented;
`audio_fingerprint` unapplied to audio; the worker not exercised in the E2E
suite (these tests drive the handler directly against a local server, so a
Playwright test watching a reference go from `pending` to hashed is next); and
only the first availability link is tried.

Gate: clippy 0/0, fmt clean, **82 suites / 1785 tests, 0 failed**. No frontend
file was touched.

---

# Handoff — M32-07c: real image decoding, so a fetched image gets a real hash

Date: 2026-09-25. **The current, full handoff is
`docs/handoffs/2026-09-25T173000+0200-m32-07c-real-image-decoding-handoff.md`
— read that one.** It records spec §32.7.2's decoding half: `image = "=0.25.6"`
pinned because 0.25.7+ raises the workspace's declared `rust-version = "1.82"`,
`fingerprint_encoded` decoding a body to luma and producing both hashes, and the
pixel limit applied *to the decoder* via `ImageReader::limits` rather than
checked after the allocation has already happened.

**Two corrections to my own work in the same session.** My first "the same image,
re-encoded" fixture flipped a PNG filter byte that was already set, so the copy
was byte-identical — and flipping a filter without re-encoding the deltas
changes the *decoded* image anyway, so the premise was wrong as well as the
bytes. It now splices a `tEXt` chunk: different bytes, identical pixels, which
is the real shape of the problem. And the decompression-bomb fixture emitted
`width * height` zero bytes, making its own test 33 seconds while the decoder
refused correctly throughout; a bomb only needs its declared size with one real
row, and it is now 0.00s.

**Still not shipped, and it is now the only structural gap in the chain:** there
is no media job kind and no fetch loop, so nothing queues a fetch and drives
`plan_fetch → classify → decode → record_fingerprint` in sequence.
`record_fingerprint` has no production caller — only tests. That is the same "a
parser is not a feature" shape, and it is why the job wiring is the next
milestone rather than polish. `phash`/`whash`/`ahash` remain unimplemented and
`audio_fingerprint` is still unapplied to audio.

Gate: clippy 0/0, fmt clean, 81 suites / 1773 tests, 0 failed. An earlier run hit
the documented `milestone_2` rate-limit flake, which passes in isolation; the
clean re-run is the quoted number. No frontend file was touched.

---

# Handoff — M32-07b: perceptual hashes computed and stored; the default no longer lies

Date: 2026-09-25. **The current, full handoff is
`docs/handoffs/2026-09-25T164500+0200-m32-07b-perceptual-hashes-computed-and-stored-handoff.md`
— read that one.** It records spec §32.7.2's second half: a real 64-bit dHash
that resamples the frame to a fixed 9×8 grid, the SSRF guard on the media fetch
path, and the DB write that makes the `perceptual_hash` column real.

**M32-07a's handoff called the remaining half "blocked on a dependency". That
was wrong in the way that matters:** crates.io was reachable the whole time and
no decoder was in the lockfile. The dependency was *absent*, not *unavailable*.
Three bugs in my own new code were caught by tests written for them — a dHash
that read only the top five rows of an image, an `assert_eq!` where a Hamming
distance bound was the real property, and an SSRF check written against
`host_str()` that silently skipped every IPv6 literal including `[::1]`. The
`perceptual_hash_algorithm` default changed from `phash` to `dhash`, because
phash is not implemented and a stock instance was advertising a dedup it could
not perform.

**Still not shipped, and the next step:** image *decoding* is still absent, so a
real fetched JPEG/PNG gets its exact content hash and a `NULL` perceptual hash
rather than a fabricated fingerprint, and there is no media job kind or fetch
loop driving any of it. `audio_fingerprint` is still unapplied to audio.

Gate: clippy 0/0, fmt clean, 1767 workspace tests passed, 0 failed. No frontend
file was touched, so the Svelte and E2E gates were not re-run.

---

# Handoff — M32-07a perceptual dedup implemented; the column nothing populates

Date: 2026-09-25 (tip `fd08762` + this docs commit). The previous full handoff
is
`docs/handoffs/2026-09-25T153000+0200-m32-07a-perceptual-dedup-implemented-handoff.md`
— read that one.** It records spec §32.7.2 perceptual deduplication: a
Hamming-distance search ordered closest first, the `[media_resilience]` config
table wired to TOML for the first time, per-match confidence and distance in
the response and on the admin media page, and three dead things the feature's
absence had been hiding — an uncalled domain validator, a config struct with no
TOML surface, and a config test helper that could read a stale file. Gate:
clippy 0/0, 1733 tests passed, svelte-check 0/0, 308 vitest passed. It is
explicit that the feature is correct on an empty column: nothing computes a
perceptual hash yet, which is filed as M32-07b and needs image decoding as a
new dependency.

Date: 2026-09-25 (tip `9cd3c71`). The previous full handoff is
`docs/handoffs/2026-09-25T142500+0200-false-clean-gate-three-defects-e2e-assertion-fixed-handoff.md`
— read that one.** It records a `cargo clippy` gate that reported 0 warnings
because it piped stderr to `/dev/null` and rustc reports warnings there; the 30
warnings that were actually present, three of them real defects (an ignored
`media_reference_id` whose doc comment promised a per-reference match, SQL
placeholders built by `if i == 0 { "" } else { "" }`, and a `max_distance`
silently ignored), a test that had never run, and an E2E test that asserted an
empty list on a shared account when it should have asserted its own row's
disappearance. Verification: clippy 0/0 with stderr captured, 1775 tests passed
with one known rate-limit flake that passes in isolation, release build clean,
E2E 73/73.

Date: 2026-09-25 (tip `4ae4450`). The previous full handoff is
`docs/handoffs/2026-09-25T134127+0200-metadata-exchange-specified-docs-synced-handoff.md`,
which records the metadata-exchange specification landing (spec §0.3, §2.3.1,
§11.17, §15.17, §16.16.1, §19.14; 5 `planned` rows; M57 build order), what was
rejected and why, and the outstanding verification debt.

Date: 2026-09-25 (tip `5fb778e`). The previous full handoff is
`docs/handoffs/2026-09-25T110532+0200-m56-01-instance-access-mode-build-recovered-handoff.md`.

Date: 2026-09-24 (M52 tag `m52-rec-strategy`, export E2E fix). Previous handoff
(M51 + M12 + M7, v0.51.0+2) is archived at
`docs/handoffs/2026-09-23T112310+0200-m51-media-resilience-m12-mentions-m7-device-delivery-handoff.md`.

## What happened this session

A consolidated from-scratch specification was written (session copy:
`/tmp/ficarchive-from-scratch-spec.md`) by auditing both codebases — FicNexus
(`~/code/rust/ficnexus`, built 2026-07-25 → 09-09, plus `fanfic-scrapers`
and `fanfic-archivist-bot`) and Lorehaven. Decision (ADR 0024): **Lorehaven
is the base**; the from-scratch spec is its target state. Adapting FicNexus
would be architectural surgery (single-dialect Postgres + pgvector/tsvector,
load-bearing Redis, XP/levels to remove, ~150 write routes with opt-in
enforcement per its own REBUILD-NOTES). Adapting Lorehaven is mostly
completion. Estimate at observed pace (~11 requirements/day): 6–9 weeks
concentrated + 2–3 week backgroundable adapter tail.

## Files changed (all in ~/code-local/rust/lorehaven, uncommitted)

- `docs/adr/0024-lorehaven-as-base.md` — **new**. The decision, port sources,
  spec amendments, effort estimate.
- `docs/spec.md` — front-matter ADR-0024 note; **§16.1a** strategy registry
  (RecStrategy trait, RRF k=60, `rec.mode`, golden legacy-parity test);
  **§11.16** adapter porting backlog (fanfic-scrapers as port source,
  fixture-gated, `blocked-here` status); **§23.2** amended (bot port source
  fanfic-archivist-bot).
- `docs/plans/remaining-work.md` — **new**. The forward plan: M52 rec
  registry, M53 adapter batch 1, M54 bot port, M55 OpenAPI publication, M56
  M45/M47 residuals; verified current-state summary.
- `docs/requirements.csv` — repaired 9 malformed rows (unquoted-comma bug
  shifted columns; M33/M34/M35 series), added M52-01…M55-02 (16 planned
  rows). Now 255 rows, 0 malformed: 194 implemented (174 locally-tested,
  20 fully-tested), 57 planned, 4 unsupported.
- `README.md` — status section rewritten: was stale ("M0–M5 complete, M6
  partly built"); now the verified state, tags through v0.51.0, milestone-series
  table.
- `docs/plans/README.md`, `docs/plans/junior-implementation-plan.md` —
  superseded-status notes pointing at remaining-work.md.
- `docs/spec-gaps-ficnexus.md` — status header: resolved by ADR 0024 with
  the resolution map (kept as audit trail).

## Verified current state (evidence: requirements.csv + git log + tests)

194 of 255 rows implemented. 51 milestone test files (M0–M45),
migrations to 0071 in both dialects, 37 frontend routes, 11 scraper
adapters, tags through `v0.51.0`. Series built: platform core M0–M15,
marketplace/translation/API/admin M16–M26, forum M31–M35, directory/fork/
half-life/CTAs/ordering/roadmap-consensus/settings M39–M47, media
resilience M48–M51. Planned rows remaining: M45 (46, taste-arena
residuals), M47 (3, settings surfaces), plus the new M52–M55.

## Next steps (priority order)

1. **Commit this docs change** (docs-only; no code touched).
2. **M52 rec strategy registry** first — freezes neutral behavior before
   further influence work. Spec §16.1a; rows M52-01…08 already in the CSV.
   Golden legacy-parity test comes first: freeze `discovery::blend` output
   on fixtures before writing the registry.
3. **M53 adapter batch 1** (ffnet, ao3, royalroad, fictionpress, ficbook,
   syosetu → 10 verified adapters). Port parse logic only, from
   `~/code/rust/fanfic-scrapers` (read-only reference).
4. M54 bot port, M55 OpenAPI, M56 M45/M47 residuals.
5. Then: 3 known E2E failures (worker timing, download verification,
   subscription unread count) — confirmed after M52 commit: 2 remain flaky
   (both export/worker: EPUB ready detection + delete after). The media
   overflow is fixed. Tag v1.0.0, deploy to thinkcentre.
6. Run `scripts/seed_roadmap.py` after committing — the 16 new planned rows
   appear as `idea` cards (ADR 0023: the CSV is the board seed).

## What to pass along

- The from-scratch spec lives at `/tmp/ficarchive-from-scratch-spec.md`
  (session scratch, 24h-pruned — copy into the repo if wanted durable; it
  was deliberately not committed because spec.md + ADR 0024 now carry its
  content).
- FicNexus / fanfic-scrapers / fanfic-archivist-bot are **read-only port
  sources**. Never run git/cargo/npm under `~/code/rust/*` (SSHFS risk);
  copy files out to read them.
- The 9 repaired CSV rows were quoting bugs (unquoted commas in the
  requirement column shifting everything right). Validator: 7 columns, id
  matches `M\d+-\d+`, status in {planned, implemented-locally-tested,
  implemented-fully-tested, unsupported}.
- Carried from previous handoff: M6-10 preservation batches and M6-15
  aggregate mode remain deliberately unsupported; the scraper-bot adaptation
  gap is now M54 (planned) instead of "explored, not ported"; Obscura
  integration, the 40k rescrape and the webnovel-scraper port remain
  unstarted and unscheduled.

## M29 — recommendation transparency (§33.3): decided, not stubbed

**The design question, settled.** `GET /discovery/slots/{id}/explanation` was
unreachable and had to be either *persisted* or *recomputed*. Recomputation is
not available: `time_decay_strategy` scores with
`1.0 / (1.0 + julianday('now') - julianday(created_at))`, so a replay returns
different floats and would explain a ranking other than the one the reader
received. Slots are recorded at serve time instead, in `recommendation_slots`
(migration 0076), and the explanation is a read of that row.

**Five stubs became real code.** `explain_slot`, `get_attention_report`,
`propose_wrangling`, `list_wrangling_proposals`, `approve_wrangling` all
returned hardcoded strings and touched no database. They now do the work, and
two more doors were added: `PUT /me/attention-report` and
`POST /admin/tag-wrangling/proposals/{id}/revert`.

**The vocabulary is closed on purpose.** `SlotReason` has no operator-boost
variant, `TasteSignal` is three buckets rather than a float, and
`InstanceCuration` is two values with no magnitude. A `String` reason is how an
admin multiplier reaches a reader's explanation — not by accident, but because
someone eventually needs to say "the admin pushed this" and a `String` permits
it. A test walks every variant's prose against a forbidden-word list.

**Four bugs the tests found, all now pinned:**

1. **The merge retarget was wrong on SQLite.** Written as `UPDATE OR IGNORE`
   (SQLite) and `ON CONFLICT DO NOTHING` (PostgreSQL). Neither expresses the
   rule: they suppress a *constraint violation*, not a *row that would become a
   duplicate*. The SQLite form let the primary-key violation abort the
   statement — the merge moved nothing and reported success. A work already
   carrying the target also kept both spellings, which is the state a merge
   exists to end. Now handled per row, identical on both backends.
2. **An invalid transition was a 500.** `anyhow::bail!` on approving an
   already-approved proposal told the client to retry something that could never
   succeed. Now a typed `WrangleError` → 404 or 409.
3. **The routes passed an account where the FK says pseud.** A proposal is
   *authored* by a pseud; §19.1's trust gate is on the *account*. Both recorded.
4. **`set_weight` was not in the migration's CHECK constraint.** Invented the
   action after writing the schema, so every dedupe-path merge 500'd.

**A vacuous test, caught by mutation.** The end-to-end test
`a_served_recommendation_carries_a_slot_id_that_explains_itself` passed with
the recording disabled — it iterated an empty `items` list because the instance
had no published works. It now publishes three first. Verified by disabling the
recording and watching it fail on the missing `slot_id`.

**A latent injection pattern found on the way.** `rec_strategy.rs`
string-interpolates `account_id` into SQL at six sites. It is a UUID today, so
it is not exploitable, and it is unrelated to this work — but it is the exact
shape that becomes injectable the moment anything non-UUID reaches it. Worth
its own fix.

**Retention is a job, not a hope.** `purge_slots` joins `purge_exports` in
`MAINTENANCE_TASKS` and the worker's match arm; `jobs.slot_retention_days`
defaults to 3 and is clamped to a day..year. A test pins that the name is in
both places, because a task named in one list and not the other silently never
runs.

22 acceptance tests, green on both backends. 21 domain, 3 db unit.

### The taste profile was faked, and now is stored (migration 0077)

`PUT /operator/taste-profile` answered `{"status": "updated"}` and persisted
nothing. The dimensions were parsed out of the body, echoed back, and dropped on
the floor, so an operator was told the instance's taste model had changed and it
had not. The `// TODO: persist updates to config file` above it was not a
loose end so much as the whole story: a running instance does not rewrite its own
configuration, and the API has no business editing a file it may not be able to
write. Migration 0077 gives it a table.

`GET` falls back to the config when no row exists, which is the right default in
both directions — an untouched instance reports the axes it is *actually* running
on rather than an empty list that reads like a bug, and a configured instance
does not pretend to have a stored profile.

The amendment wants `{key, label, admin_target, weight}`; the config carries bare
names. So `TasteDimension` validates at the edge, and two of its rules are worth
knowing about:

- **Duplicate keys are refused, not resolved.** Two axes called "prose" would
  make a work's weight on that axis depend on which row a query read first.
- **A target outside 0..1 is refused.** It is not a position on the axis.
  Similarly, `diversity_injection_percent: 150` is a 422 rather than clamped to
  100 — clamping would make the instance maximally diverse because someone typed
  a number.

### Three defects the route inventory found, none of them mine originally

1. **`reject_wrangling` had no route.** The DB-layer function was written during
   the transparency work and never wired up. The route inventory caught the gap,
   which is precisely what it is for. It records the decision and the reason
   rather than deleting the row: "reviewed and declined" and "never reviewed"
   are different facts, and an un-reviewed queue is what an operator needs to find.
2. **`list_pending_wrangling` ignored a status filter.** A steward could only
   ever see what was waiting, never what had been decided. Now `list_wrangling`
   takes the status as a *bind*, pending by default.
3. **`approve_wrangling`'s PostgreSQL arm bound `&str` against `?::uuid`.** It
   was unreachable — nothing called it — which is exactly how a type error
   survives in a function only its own tests exercise.

Also a pre-existing table error, present since the stub days:
`/discovery/slots/{id}/explanation` was tabled as `Audience::Public` while the
handler takes `RequireSession`. Corrected. `public_wrangling_log` now takes
`MaybeSession` explicitly rather than being public by omission — the inventory
cannot tell an intentionally public route from one someone forgot to guard.

### A flake that is not a regression

`milestone_2::repeated_login_attempts_are_rate_limited` failed in a parallel
full-suite run and passes alone, and passes as a whole suite. The limiter is
process-global and `clear_buckets()` is called by several tests, so under
`--test-threads=2` another test refills the bucket between the clear and the
loop. It is a test-isolation problem, not a product one; run that suite on its
own when it matters. Do not "fix" it by widening the limits — the test exists to
trip them.

## Environment quirks (unchanged)

- **Work in local clone** `~/code-local/rust/lorehaven`. `~/code/rust/lorehaven` is SSHFS — never run git/cargo/npm through it.
- Daily sync: `lorehaven-sync.timer`, 09:00.
- Playwright E2E runs ON thinkcentre over SSH (`frontend/e2e/serve-scratch.sh`).
- Deployed instance: thinkcentre `127.0.0.1:8081`, admin credentials in `~/.hermes/.env`.
- **Frontend builds need to run ON thinkcentre** — the embedded bundle (`frontend/dist/`) is built into the binary with `rust-embed`.
- Build on thinkcentre: `pkill -9 cargo` first; binary swap needs `pkill -9 -f "lorehaven serve"`.
- Argon2 params: m_cost=19456, t_cost=2, p_cost=1.
- Rate limiter buckets are process-global (`GLOBAL_BUCKETS` in `limiter.rs`), keyed by IP.
- Lint false positive: the write_file/patch tool's linter runs rustc with Rust 2015 edition and reports `async fn` errors — ignore those; `cargo check` is the real gate.

## Gotchas (carried forward, still true)

- Spoilers routes passed `pseud_id` where DB FK'd `accounts(id)` — fixed with `RequirePseud { user, .. }` → `user.account_id`.
- Config `rate_limits` field has no top-level `burst`/`per_minute` — nested in each `Quota`.
- Comment POST returns **200** with `{id, receipt}`, not 201.
- `forum_categories` has no repo-level `create_category`; tests seed via raw SQL.
- **NewTopicForm** input ID must be `#topic-title` (tests expect this).
- **Work page** shows chapter titles in a list; clicking opens the reader.
- **Docs pages**: sections appear in both body and TOC — use `.first()` with `getByText`.
- Community category navigation: `/community/forums/<id>` requires waiting for the `Topics` heading.

## How to resume

Read ADR 0024, then `docs/plans/remaining-work.md` §M52 — it names the spec
section (§16.1a), the rows (M52-01…08), and the port source. Start with the
golden legacy-parity test.
