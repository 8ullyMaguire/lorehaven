# Handoff — three surfaces, and a backslash that ate itself

## The short version

Search now speaks one language on three surfaces: works, forum, and users.
`/api/v1/users/search` is new in this slice; the `ESCAPE` bug it exposed is not.

That bug is the reason this handoff exists. It is silent on both backends in
different ways, it was in code written in an earlier slice, and it took an hour
to find because the source reads correctly. It is written up in full at the
bottom under **The backslash that ate itself** — read that part even if you
skip the rest.

## What a reader can type today

    works:10000..50000          inclusive at both ends
    works:..50000               an open lower bound
    replies:>50 category:meta   forum
    active:>2026-01-15          a date, not a number
    user:nightowl               a pseudonym
    fandoms:"Good Omens"        a pseudonym who has written in a fandom
    works:>10                   how much a pseudonym has published
    joined:<2026-01-01          when they arrived

Every bound is inclusive. `10000..5000` is refused with a reason.

An empty query is an empty page on the forum and user surfaces. The works
search treats it as a browse; putting every pseudonym on an instance into one
response is a phone book and a denial of service at the same time.

## The four things this slice found

**A date comparison rendered as a number.** `render_comparison` assumed every
comparison's value was an integer. `published:>=2026-08-01` parsed and then
failed to render — and because both media doors validate a query by running it
through `render_query` first, `published:2026-08-01..2026-08-01` went from 200
to 422 for every reader. The range arm handled dates; the comparison arm did
not, and the range arm is only reachable *through* the comparison arm.

**`works:10` was a syntax error on the user surface.** A numeric field with no
operator is equality, and the works renderer already treated it that way. The
user renderer did not, so `works:0` was a 422 while `words:5000` worked. A
reader who counted to ten and got an error has no way to guess they should have
written `works:=10`.

**A fandom spelled as an alias found nobody.** `taxonomy_nodes.canonical` is
the spelling the instance chose; `taxonomy_aliases` holds what readers type.
Matching only the canonical form means a variant spelling returns an empty page,
and a reader concludes nobody writes in that fandom.

**`ESCAPE ''`.** The subject of its own section below.

## The backslash that ate itself

No `LIKE` in any renderer declared an `ESCAPE` character, while all three
escaped `%`, `_` and `\` in the bound pattern. PostgreSQL treats backslash as
the default `LIKE` escape. SQLite has no default. So `100%` matched correctly on
one backend and nothing on the other, with no error on either.

The working form has **two** backslashes in the Rust source:

    "… LIKE LOWER(?) ESCAPE '\\'"

The file on disk had one, which Rust reads as `\'` — an escaped apostrophe — so
the compiled SQL was:

    ESCAPE ''

and not the one backslash the source appeared to ask for.

    SQLite     → ERROR: ESCAPE expression must be a single character
    PostgreSQL → accepts '' as an empty escape, which matches every row

One backend errors, the other returns everything, and the source reads
correctly. It survived `cargo fmt`, survived review, and passed 20 renderer
tests, because every one of them asserted behaviour on SQLite only.

**What to do when a SQL edit appears to do nothing:** print the *rendered*
fragment, not the source. A throwaway test that `panic!`s the SQL finds it in
one run. Text-level `replace` calls write a different number of backslashes than
the Rust lexer wants, and that is how the doubled form became a single one.

`a_like_pattern_declares_its_escape_character` now asserts the emitted clause
byte for byte, because `contains("ESCAPE")` also passes against `ESCAPE ''`.

## A schema divergence still worth a migration

`pseuds.id` is `UUID` on PostgreSQL and `TEXT` on SQLite, and ten tables carry a
pseud FK that is `TEXT` in *both* migrations: `comments.author_pseud`,
`forum_topics.author_pseud`, `forum_posts.author_pseud`, `forum_karma.pseud`,
`forum_votes.pseud`, `critique_participants.pseud`, `work_reactions.pseud`,
`prompt_posts.winner_pseud`, and one more.

Joining one of those to `pseuds.id` without a cast is accepted by SQLite and
rejected by PostgreSQL with `42883: uuid = text`. The three new statements use
`CAST(... AS TEXT)`, which is ANSI and needs no dialect split. **The real fix
is a migration retyping those columns to `UUID`**, deliberately not done here.
The column-name parity test compares names and not types, so it will not catch
a regression.

Note that *binding* one of those columns is fine on both dialects. Only the join
to `pseuds.id` breaks.

## Still open

1. **Bookmark and directory renderers.** Two surfaces left of five.
2. **Shared taste gravity and the meta-ranker across surfaces.** One ranking
   architecture, so signal weighting is not re-implemented per entity.
3. **The bookmark and directory filter UIs**, and a user one. Works and forum
   have one each; the user surface has none yet.
4. **E2E coverage for the new search behaviour** at the browser level. This is
   the largest remaining gap and it is not small.
5. **The migration** retyping the ten TEXT pseud FKs.
6. **Nine Rust warnings on the ThinkCentre build**, including
   `unused variable: user` in `crates/app/src/routes/media_resilience.rs:180`.

## Verified

- 17 user-renderer, 20 forum-renderer, 567 domain unit tests
- 20 user DB+route, 26 works-comparison, 11 forum DB, 8 forum route,
  17 media-field tests — each on SQLite **and** PostgreSQL
- 0 clippy warnings workspace-wide
- Commits `0e6ecc1` (forum search end to end), `9277b57` (the date fix),
  `0942094` (user surface + ESCAPE), all pushed

## The one trap in this codebase

`cargo fmt` reformats long SQL strings and will reflow them, which silently
reverts a cast you added to a line above if the line got re-wrapped. Twice in
this slice a fix "came back" after a `fmt`. If a change seems not to apply, run
`git diff` before assuming the test is lying.

# Handoff — the forum search speaks the same language as the works search

## The short version

The unified-search goal is now two surfaces, not one. `..` ranges landed, dates
became comparable values, and `/api/v1/forum-search` was rebuilt on the shared
query language: `replies:>50`, `category:meta`, `author:nightowl`,
`active:>2026-01-15`, `locked:true` and `replies:10..100` all work there, and a
field from another surface is a 422 naming it rather than an empty page.

The old `crates/db/src/forum_search.rs` is deleted. It had two hand-written
dialect arms and no callers once the route moved.

**Four bugs surfaced while building this, and three of them were in code the
previous slice had just written.** They are listed below because each one is a
*silent* failure — a search that returns the wrong rows, or none, and looks like
a legitimate empty result.

## What a reader can type today

Works (`/api/v1/search?q=`) and forum (`/api/v1/forum-search?q=`) share one
parser, one operator set, one 422 behaviour.

    tag:"enemies to lovers" fandom:"Good Omens" words:>10000
    words:10000..50000          inclusive at both ends
    words:..50000               an open lower bound, inclusive upper
    replies:>50 category:meta   forum
    active:>2026-01-15          a date, not a number
    locked:true
    a NOT b                     a AND NOT b
    "a quoted phrase"           matches exactly, in post bodies only

Every bound is inclusive. `10000..5000` is refused with a reason, not silently
empty.

## The three bugs worth knowing about

**A numeric bound needs `CAST(? AS BIGINT)`.** The bound is a `String`, and
SQLite leaves it as text on the right of an integer, so `3 > '1'` is *false*.
Without the cast every `replies:>N` matched nothing and read as "this category
is empty". There is a test for it — and a test for the opposite mistake, because
casting a *date* bound compares `2026-01-15` as the number 2026.

**`bounds_descend` compared lexicographically.** `"10000" < "5000"` as strings,
so `10000..5000` passed the backwards-range check and then matched nothing.
Numbers compare numerically; dates compare lexicographically. The caller passes
the kind, because it is the only layer that knows the column's type.

**A `forum_posts` join multiplies rows.** The result set is topics, so the
free-text, phrase and `author:` arms each reach `forum_posts` through a
correlated `EXISTS` instead. A thread with three replies all saying "winter"
comes back once. A join would have returned it three times with the same title,
and a reader cannot tell that from three different threads.

## A schema divergence worth knowing about

`pseuds.id` is `UUID` on PostgreSQL and `TEXT` on SQLite, but ten tables carry a
pseud FK that is `TEXT` in *both* migrations (`comments.author_pseud`,
`forum_topics.author_pseud`, `forum_posts.author_pseud`, `forum_karma.pseud`,
`forum_votes.pseud`, `critique_participants.pseud`, `work_reactions.pseud`, and
three more). Joining one of those to `pseuds.id` without a cast is accepted by
SQLite and rejected by PostgreSQL with `42883: uuid = text`.

`community.rs` splits its arms for this. The new forum statement uses
`CAST(pseuds.id AS TEXT) = forum_topics.author_pseud` instead, which is ANSI —
uuid-to-text on PostgreSQL, text-to-text on SQLite — so the statement is a
single shared string with no `Dialect` parameter at all.

**The real fix is a migration** retyping those ten columns to `UUID` to match
`pseuds`. That is worth doing and was deliberately not done here; the
column-name parity test does not compare types, so it will not catch a
regression.

Note that *binding* one of those columns is fine on both dialects. Only the
join to `pseuds.id` breaks. `works.owner_pseud_id` and
`collaboration_invites.invited_pseud_id` are already `UUID` and correct as
written.

## What the UI does now

- The category dropdown is populated from `/forums`. It used to be hardcoded to
  `general`, `fanworks`, `discussion`, `help` — **none of which is a category a
  fresh instance creates**, so anyone who picked one got an empty page and no
  way to know the dropdown was the problem. A failed fetch omits the filter
  entirely; the query box still works.
- A min-replies box was added, and an empty query can search on filters alone
  (`category:meta` is a complete query).
- A 422's reason is shown verbatim instead of "Search failed".
- One line under the box lists what the language accepts. A reader cannot learn
  the operators except by being told.
- The works search box emits `words:10000..50000` as one `..` term, and drops a
  backwards range before sending it.

## Still open

The bulk of the feature, unchanged in shape:

1. **User, bookmark and directory renderers.** The field registry knows the
   fields; nothing renders them.
2. **Shared taste gravity and the meta-ranker across surfaces.** One ranking
   architecture so signal weighting is not re-implemented per entity.
3. **The other three specialized filter UIs** (user, bookmark, directory).
   Works and forum have one each now.
4. **E2E coverage for the new search behaviour** at the browser level.
5. **The migration** that retypes the ten TEXT pseud FKs to `UUID`.

## Verified

- 563 domain unit tests, 20 forum-renderer tests
- 11 forum DB integration tests, 8 forum route tests, 26 works-comparison
  integration tests — all on SQLite **and** PostgreSQL
- 326 frontend tests, 0 `svelte-check` errors, 0 clippy warnings workspace-wide
- Commits `8afa138` (`..`), `b2d235d` (forum renderer), `0e6ecc1` (forum search
  end to end), all pushed to `master`

## The one trap in this codebase

`cargo fmt` reformats long SQL strings and will reflow them, which silently
reverts a `::uuid` cast you added to a line above if the line got re-wrapped.
Twice in this slice a cast "came back" after a `fmt`. If a fix seems to not
apply, run `git diff` before assuming the test is lying.

# Handoff — the query language grows a comparison, and two of its bugs surface

## The short version

The unified-search work has a foundation and a first user-visible slice:
comparison operators (`>`, `>=`, `<`, `<=`) on the live parser, a cross-entity
field registry, a 422 instead of a 500 for a bad query, and a word-count range
in the search UI.

Two of the fixes are pre-existing bugs that the new tests exposed rather than
introduced. One of them was silent and total: **`NOT spoiler` matched nothing
at all**, for any work with a NULL summary or an unindexed body.

**Still open, and it is the bulk of the feature:** `..` range syntax is not
implemented; the forum, user, bookmark and directory renderers do not exist yet;
per-entity backends and shared taste-gravity ranking are not started; and four of
the five specialized filter UIs are not written. What exists today is the
language plus one surface.

## What the query language supports now

`crates/domain/src/query.rs` is the parser, `crates/domain/src/query_sql.rs` the
renderer. Both are the live path — they are what `search_works_ast` calls.

- `field:value` and `field:"quoted phrase"`, case-normalized
- Comparison operators on numeric fields: `words:>10000`, `kudos:>=50`
- `AND`, `OR`, `NOT`, parentheses
- Trailing `NOT` in an implicit conjunction: `a NOT b` means `a AND NOT b`
- Cross-entity fields are *recognized* by the parser and carry their owning
  entity, so the parser can tell `replies:>50` (forum) from `words:>10000`
  (works)

Two deliberate design choices, both load-bearing:

**`QueryAst::Comparison` is its own variant, not an overload of `Fielded`.** A
renderer that inferred the operator by inspecting a value string would have to
re-parse `">10000"` and guess. Keeping the operator in the AST means no renderer
ever guesses.

**A field on the wrong surface is an error, not an empty result.** Rendering
`replies:>50` against the works table returns a `QueryError` naming the surface
the field belongs to. Silently ignoring it would have produced "no results" for a
query the reader could see was well-formed — the worst possible failure, because
it looks like a correct answer.

## The two bugs

**A reader's typo was a 500.** `search_works_ast_impl` raised parse and render
failures with `anyhow!`, so the route mapped them all to `AppError::Internal` —
a server fault, with the message masked by design. Typing `replies:>50` into the
works search produced something that was neither the reader's error nor the
server's. There is now a `SearchError` carrying a `QueryProblem` (parse, with an
offset, or render), and the route downcasts to it: 422 with the reason, and
everything else still 500.

`anyhow`'s blanket `impl<E: StdError> From<E> for anyhow::Error` is what carries
the concrete type through, so there is deliberately **no** manual `From` impl in
`query_error.rs`. Adding one is a conflicting-impl error, which is noted in the
file so the next reader does not "fix" it.

**`NOT spoiler` matched nothing, silently.** The free-text predicate is
`title LIKE ? OR summary LIKE ? OR body_text LIKE ?`. `works.summary` is
nullable, and `works_index.body_text` is NULL for any work the indexer has not
reached. `NOT (false OR NULL OR NULL)` evaluates to NULL — not true — so a
negated free-text term excluded *every* such work, with no error and nothing the
reader could see. Confirmed against PostgreSQL directly before fixing: the
coalesced form returns true, the bare form returns NULL.

`render_query` now emits `NOT COALESCE(<inner>, false)`, and only where the inner
fragment can be NULL. A taxonomy term is an `EXISTS`, which is a definite false
and does not need the guard — the test suite pins both directions, so the
coalesce cannot creep onto a subquery that does not need it.

## A test of mine that passed on nothing

`a_comparison_composes_with_a_negated_term` asserted only that the result did not
contain `"Tiny"`, which is true when the result is *empty* — and it was empty,
for the reason above. It now asserts the un-negated term first (exactly one
match), then the negated term (everything else), so a regression to the empty
result fails it.

Worth carrying forward: an assertion of the form `assert!(!result.contains(x))`
is vacuous when `result` is empty. Assert the positive case first.

## Ordering is not a contract

Two integration assertions had to compare membership rather than order, because
the rows tie on `score` and the `updated_at DESC` tiebreak resolves differently
on each backend. This has now bitten twice in one file. Do not assert row order
where scores tie.

## Verification

- `cargo test -p lorehaven-domain --lib -- query` — 57 pass
- `cargo test -p lorehaven-app --test search_comparisons` — 19 pass, and all 19
  pass again on PostgreSQL
- `cargo clippy -p lorehaven-domain -p lorehaven-db --all-targets` — 0 warnings
- `npx vitest run` — 316 pass; `svelte-check` clean

PostgreSQL test container: `lh-m32e-pg` on port `55433`.

## Next, in order

1. `..` range syntax in the parser and renderer (`words:10000..50000`).
2. The forum renderer — `replies:>50`, `after:2026-01` are already parsed, they
   just have nowhere to go.
3. User, bookmark and directory renderers, same shape.
4. Per-entity indices, and the shared taste-gravity ranking the spec calls for.
   Right now ranking is works-only; the point of this work is that it is not.
5. The four remaining filter UIs. The works one now has a word-count range; the
   other four do not exist.
6. E2E coverage for search across surfaces, once there is more than one.

---

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

## The whole dual-backend claim rested on the forgiving engine

The headline finding of this stretch. The PostgreSQL suite had never been run to
completion, so the repository's central architectural claim -- that the same code
serves both engines -- was supported only by tests that had ever run against
SQLite.

Run for the first time, it was **30 failing tests across 11 suites**. Nearly all
of them were one thing: SQL that SQLite accepts and PostgreSQL rejects. SQLite is
dynamically typed, so each of these passes there and fails here:

| written            | SQLite      | PostgreSQL                    |
|--------------------|-------------|-------------------------------|
| `WHERE id = ?`     | fine        | syntax error at the next `AND` |
| `SELECT some_int4` | fine        | "driver reports the column as INTEGER" |
| `SELECT some_uuid` | fine        | "mismatched types; Rust String" |

The worst one was `vote_tx!` in `directory.rs`: a `macro_rules!` with a single
`?`-using body, written on the assumption that sqlx renumbers placeholders per
dialect. sqlx 0.8 does not. Every entry vote on a PostgreSQL instance was a 500,
and every test was green. Four more arms had the same defect across
`category_governance.rs` (36 placeholders, 6 arms), `settings.rs` (3), and
`work_discussion.rs`, where `linked_topic` cast `chapter_id::text` and left the
`work_id` sitting next to it uncast, so every work-thread read was a 500.

**Thirty fixed, and the two classes are now gated.**

### The gate: bound the arm by brace depth, then look inside it

`scripts/check-pg-backend-arm-placeholders.py` (blocks in CI, self-test 6/6).
The whole design is one idea: once you have the `Backend::Postgres => {` ... `}`
span, "is this literal inside it" is *exact* rather than heuristic.

This matters because four earlier positional attempts each reported 100-500
correct statements as faults -- the repo spells a dialect pair three legal ways
(`db.sql(a, b)`, `let sqlite = ...` / `let postgres = ...`, and a `format!` built
statement), a literal may sit on the line after its binding, and SQL fragments get
interpolated into callers' statements. A gate that cries wolf gets muted, which
is worse than no gate. Brace-depth bounding has no false positives and found all
eleven real sites, three of which had been green for a long time.

Two details:

- `?` is ambiguous *even inside a correct arm* -- PostgreSQL's JSONB containment
  operators are spelled `?`, `?|` and `?&`. Skip whitespace after the `?` and treat
  a following quote, `|`, or `&` as an operator. Do **not** treat a `?` that ends
  the line as an operator: `LIMIT ?` is an ordinary bind. I got this backwards
  first and it silently hid eight real sites.
- Number `$n` per *arm*, not per literal. An arm usually runs several statements
  each with its own `.bind(..)` chain; restarting at `$1` collides with the
  statement before it.

`scripts/fix-pg-arm-placeholders.py` is the migration, and it is worth keeping
next to the gate: fixing these by hand went wrong twice, because the two arms
hold adjacent, near-identical literals and `replace(.., 1)` cheerfully edits the
SQLite one. The gate caught both mistakes within minutes of being written.

### Four gates written, then deleted

While fixing the above I wrote four scanner-shaped gates without first reading
`scripts/`. Three were duplicates of gates already in CI, and the fourth was
strictly worse:

    check-backend-pool-mismatch.py     ->  check-pg-arm-uses-sqlite-pool.py
    check-pg-int4-as-i64.py            ->  check-uncast-pg-placeholders.py
    check-pg-uuid-placeholder-casts.py ->  check-pg-uuid-casts.py
    check-pg-arm-divergence.py         ->  neither; 289 advisory findings

`check-uncast-pg-placeholders.py` reads 256 tables out of `migrations/postgres`
and resolves the column type per statement, which is the thing my name-matching
approach could not do -- the reason mine produced ~280 false positives where this
one produces zero. The lesson is not "write fewer gates", it is that reading the
directory is a precondition for adding to it.

### Running this yourself

    export CARGO_TARGET_DIR=$HOME/.cargo-target/lorehaven-pg6   # isolate, see below
    export LOREHAVEN_TEST_PG_URL='postgres://lorehaven:***@127.0.0.1:55433/postgres'
    cargo test --workspace --no-fail-fast

Give every checkout its own `CARGO_TARGET_DIR`. Two checkouts sharing the
default `lorehaven` target directory produce failures that belong to neither:
they compile against each other's stale artifacts. This cost a long detour and
is the single most confusing failure mode in this repo.

Two of the timing-sensitive tests need `-- --test-threads=2` on this machine, and
`repeated_login_attempts_...` takes 83 s alone. Treat `0 passed; N filtered out`
as a filter that matched nothing, not a pass -- two "baselines" earlier in this
document were verified that way and proved nothing.

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
write.

**The first version of the fix was itself a defect, and the shape of the data is
why.** I created `instance_taste_profile` with a `dimensions` JSONB column —
which is a *second* home for a concept that has had a home since M17:
`admin_taste_profile` (0054) already holds
`dimension_key, label, admin_target, weight`, and
`taste_health::save_admin_taste_profile` already writes it, with a roundtrip
test. Two stores for one concept means the one that loses is whichever a future
reader happens to find, and nothing in either schema says so.

So 0077 is now `instance_taste_settings` and holds only what config carried and
no table did: `gravity_strength`, `signal_weight_mode`, `admin_weight`,
`diversity_injection_percent`. The split is by *shape* — a set of rows versus one
row — which is also the property worth having: a PUT that touches only the
dimensions does not reset the knobs, and vice versa. A test pins both directions.

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

### A leak assertion that was itself the bug

`a_served_slot_explains_itself` asserted "no raw score reached the reader" as
`!body.to_string().contains("0.")`. That is a substring test, and a slot id, a
timestamp or a `blend_score` of 42 can all contain those two characters by
chance — so it passed on SQLite and failed on PostgreSQL, which served a slot
whose id happened to spell one. `blend_score` is *deliberately* reader-visible:
it is the reader's own rank, echoed from the discovery response, and §29.2 shows
a close call above it. The assertion is now field-level — it forbids
`weight`, `weights`, `operator_affinity`, `affinity`, `score_detail` by name and
range-checks `blend_score` as the 0–100 rank it is.

The general rule: an assertion that a *field* is absent must name the field.
`contains("0.")` is a claim about text, and text is not the thing under test.

### A flake that is not a regression

`milestone_2::repeated_login_attempts_are_rate_limited` failed in a parallel
full-suite run and passes alone, and passes as a whole suite. The limiter is
process-global and `clear_buckets()` is called by several tests, so under
`--test-threads=2` another test refills the bucket between the clear and the
loop. It is a test-isolation problem, not a product one; run that suite on its
own when it matters. Do not "fix" it by widening the limits — the test exists to
trip them.

### The taste knobs are versioned, and a rollback is a write (migration 0078)

Gaps review F1 ("taste as a versioned, blendable object") and F2 ("config
history and rollback") were both unsatisfiable while `instance_taste_settings`
was a singleton that overwrote: the value an operator just replaced was gone
before anyone could read it. 0078 appends every write to
`instance_taste_settings_history`, and
`POST /operator/taste-profile/history/{id}/rollback` restores one.

Three decisions worth carrying forward:

- **The history insert and the settings upsert are one transaction, in that
  order.** Two statements leave a window where a crash records a change with no
  record of what it replaced — and the history row is the only copy of the old
  values, so that window loses data permanently.
- **A rollback is a write, not an undo.** It goes through the same path as an
  ordinary save, so the trail records that a rollback happened and who asked for
  it. Two rollbacks therefore walk *backwards* through the history rather than
  toggling between two values. A test asserts the fourth entry exists and names
  its author.
- **Each row is the state a write _replaced_, with `replaced_version` null for
  the first write** — it replaced the config file, which is not version 0.

**The ordering is by `replaced_version`, not `changed_at`, and this bit me.**
`now_rfc3339()` has second precision, so two writes inside one test share a
timestamp, and a timestamp ordering returns the rows in whatever order the
storage engine put them. I wrote the obvious ordering first, the rollback test
failed with the config baseline instead of the expected value, and switching to
the version — unique and strictly increasing — fixed it. Verified by mutating the
order back and watching the test go red.

### Mutes and blocks were stored but never enforced (commit `fad9b1a`)

M2-06 was recorded as deferred to M12; M12 shipped the rows and not the effect.
`is_muted` had **zero callers** in the tree, `presence_visible_to` was dead code,
and `GET /presence/stream` returned every presence row unfiltered — so someone you
blocked still appeared as online, and a mute was a list item that did nothing.

- `hidden_accounts()` fetches both lists in two queries. Not `is_blocked` per row:
  the caller filters a whole list, and a per-row check turns one query into N.
- The stream routes the answer through `presence_visible_to`, so the decision
  lives in the domain layer where it was already written. The viewer's own row
  survives — muting someone does not hide you from yourself.
- **Presence was opt-in with no way to decline.** The `enabled` column existed, no
  route set it, and the stream passed `enabled = true` on every poll — so wiring a
  toggle through the existing upsert would have made the opt-out last exactly one
  poll. `PUT /me/presence` sets it; the stream now reads the flag back.

**The block test passed against code with block-filtering disabled.** The blocked
account had never polled the stream, so there was no row for a filter to remove
and the assertion was vacuous. Both presence tests now assert the account is
*present* before the block or mute is applied. Worth remembering: a negative
assertion ("X is absent") is only meaningful if a positive one ("X was there")
precedes it in the same test.

## M57A — The first real metric (`own.reading.basic`)

Commit `e1c68e8`. Seven tests in `crates/app/tests/analytics_reading.rs`, green
on SQLite and PostgreSQL.

The registry landed with 57 scopes and a hardcoded `"implemented": false` in
`one_capability`, so the dashboard was a list of definitions. `own.reading.basic`
is the first with a query behind it, and it answers §9.6: finished works,
chapters read, words read, capped reading seconds.

**Two spec sentences decide the whole shape, and both are load-bearing:**

- *§9.6: "Do not count opens as proof of reading."* So nothing derives from
  `reading_history_entry` or from `reading_progress.created_at` — an open writes
  both. Finished works come from `reading_status`, which records a decision.
  The test seeds `finished` / `reading` / `dropped` side by side so a query that
  forgets `status = 'finished'` fails.
- *§9.6: "Label estimates as such."* `reading_seconds` is an estimate, so it
  travels with `Method`, and the test asserts the 30-minute cap is *disclosed*
  rather than trusting it.

**The bug worth remembering.** My first query measured each `reading_progress`
row's own `created_at → updated_at` window. The method says "wall-clock between
two progress updates on the same chapter" — a gap *between consecutive rows*, so
it needs a self-join on `(account_id, subject_id)` with a strictly increasing
stamp. The single-row form answered 599 seconds against a seeded 72000-second
gap: a real number, a plausible one, and the wrong one. Six of the seven tests
were green throughout, including the one asserting `implemented`.

That is the failure class the registry cannot catch on its own — the shape of
the answer is right and the value is wrong. A plausible number is the hard case,
not a null.

**What changed structurally:** `implemented` is now derived from the scope
rather than hardcoded in the route. A hardcoded flag is a claim in two places,
and the failure is a dashboard reporting `not_implemented` for a capability the
instance can answer. Adding the 2nd..57th metric is now a `_ =>` arm plus a
query, and nothing else.

**Still open:** the other 56 report `not_implemented`, which is honest but
means the dashboard is still mostly definitions. Next is a real metric per
capability, starting with `own.reading.trend` (the same tables, bucketed by
ISO week) and `own.work.retention` (the per-chapter drop-off curve, which has
its own spec text). No browser E2E for the analytics page. The SQLite/PostgreSQL
question is also still open and unanswered — see the ADR conversation; my
recommendation is to drop SQLite, because the local product it served is not in
the roadmap and the SQLite-as-default-gate is a blind spot for Postgres-only
defects.

## M57 — Trust-gated analytics (in progress)

Spec: `docs/spec-amendments/trust-gated-analytics.md`. Five commits:
`9a8d924` registry, `f7bcf09` floor at the query, `cf55d5c` the HTTP gate,
`b529d8d` the dashboard.

**The shape.** Analytics are a capability registry, not a list of metrics.
`crates/domain/src/analytics.rs` has 57 named `Scope`s; a dashboard asks
`visible_to(trust, role, preset)` and renders what it is told. The reason is
failure mode, not elegance: with a per-route `if trust >= 3`, adding a metric
and forgetting the check produces a 200, and 57 hand-written checks is 57
chances to be the one that forgot.

**Three separate authorisations, never conflated:**

- `trust_level` from `trust_levels`
- `Role` from `operator_role` — a grant, not a score
- `Preset` from `config.instance.preset` — an *upper bound* that can only
  remove capabilities

### The traps in here

- **The floor was 5 and had to become 10.** `CREATOR_DASHBOARD_FLOOR` was
  justified as "the same order as the public rating aggregate" — but every
  count it bands (bookmarks, ratings, comments) counts *other people*, and
  §36.12 puts author-facing analytics at 10. There are two floors now:
  `K_SELF = 5`, `K_OTHERS = 10`. The threshold must move *stricter* as the
  subject gets less personal.
- **Banding in the route is too late.** `routes/dashboard.rs` still bands after
  the query. The rows were read either way. New metrics apply the floor where
  the aggregate is produced (`ReaderCount`), so a suppressed count is not
  representable as a number.
- **`stats::apply_k_anonymity` leaked.** It returned a coarsened *key* with
  the **true count** alongside. Now returns 0 below the floor.
- **`Scope::OwnWorkBasic` is `Subject::Other`.** It is your work, but its
  headline number counts your *readers*, and the subject of a number is what
  picks the floor.
- **Registration creates no `trust_levels` row.** A fixture that UPDATEs the
  level matches nothing and every reader looks like TL0 — a gate that refuses
  everyone is indistinguishable from a gate that works. Use
  `governance::set_trust` (an upsert).
- **`Config::development_defaults()` is `curated_boutique`** → `Preset::Gallery`
  → withholds every `community.*`. Tests of the trust ladder must pin
  `open_library` or they are testing the wrong instance.
- **`basis` is `jsonb` on PostgreSQL**, `TEXT` on SQLite. `set_trust` takes
  `"{}"`.
- **`/users/search` had no session extractor** and was missing from
  `ROUTE_TABLE`; both fixed. `MaybeSession` is declared and unused on purpose
  — it is how an anonymous route states that it is anonymous.

### Testing it

```bash
# both backends
cargo test -p lorehaven-app --test analytics_gate --test analytics_k_anonymity
cargo test -p lorehaven-domain --test analytics_registry
cd frontend && npx vitest run src/routes/AnalyticsDashboard.test.ts
```

The PostgreSQL test container is `lh-m32e-pg` on port 55433. Set
`LOREHAVEN_TEST_PG_URL` to run the dual-backend path; without it those tests
silently run on SQLite only, which is how a `::uuid` cast bug survives.

**Not done:** only `own.work.basic` has a query behind it. The other 56
report `implemented: false`, which is honest but means the dashboard is
mostly a list of definitions. Next is a real metric per capability, starting
with the reading ones the §9.6 dashboard already promises.

## M58 — Vote decay in the directory (done)

Spec: `docs/spec-amendments/vote-decay.md`. Four commits: `5ac9d1b` the curve,
`dfb8c69` the integer exponent, `971bb1e` the computed score, `3db5aa0` refresh
and withdraw, `707f060` the list, `543ca2c` the UI.

**The shape.** A vote is a *current statement*, not a ballot. One row per
`(entry_id, account_id)`; re-voting rewrites `base_weight` and `voted_at` on
that row. A vote of age `d` is worth `base × (1 - clamp(0, d/cutoff))^exponent`,
and the score is `SUM` of those, computed on read. Below `min_votes` rows the
entry is exempt entirely.

**Defaults:** enabled, 60-day cutoff, `min_votes` 20, integer exponent 2.
A fresh vote is 1.0; day 7 about 0.78; day 30 exactly 0.25; day 60 exactly 0.

### The traps in here

- **sqlx's bundled SQLite has no `POWER`, `EXP`, `LN` or `SQRT`.** Probed, not
  assumed. A fractional exponent is computable in Rust and unreachable in SQL,
  so the two would disagree on the score with nothing to report it. The
  exponent is `u32` and the SQL is a product of repeated factors.
- **Counting *live* votes for the threshold is a cliff.** With 20 votes at 59
  days the live count is 20, so the entry decays and scores ~0.0006; a day
  later every vote is dead, the count is 0, the entry is exempt, and it jumps
  to full weight. It counts *rows* now. See `vote_decay::should_decay`.
- **`voted_at` is TEXT in both schemas** and the decay expression read it
  unqualified inside a subquery that aliases the table. SQLite resolved that
  against the outer query; PostgreSQL could not resolve it and reported it by
  returning the *undecayed* sum. A silent wrong answer on one engine.
- **The two fixture arms disagreed about the sign of an age.** SQLite's
  `datetime` modifier takes the signed offset; PostgreSQL's
  `NOW() - (N * INTERVAL)` subtracts it. A shared negative number wrote votes
  dated 200 days in the *future*, which clamps to 1.0 — so the suite reported
  "decay does nothing" while passing most of the time.
- **A correlated subquery given `'e.id'` compares every row to the literal
  string "e.id".** Both engines accept it silently and the threshold sees the
  same count for the whole list.
- **`vote_count_sql` has two shapes.** Parenthesised for `CASE WHEN <here> >= n`
  (a bare SELECT is a syntax error there) and bare as a statement in its own
  right (a leading `(` is a syntax error on SQLite). The `wrapped` flag exists
  so the caller states which, rather than stripping a character afterwards.
- **A `sqlx::Query` is generic over its database.** One built for the SQLite
  pool cannot execute on the PostgreSQL one, and the error names the pool
  rather than the cause. Statement and binds are built inside each arm.
- **`TestDb::connect_with_dir` names the PostgreSQL database from the tag and
  ignores the directory, but the SQLite branch opens a fixed
  `lorehaven.sqlite` with `mode=rwc`** — which reuses whatever is there. A fixed
  scratch directory means a failed run's rows survive into the next, and the
  next run fails on a UNIQUE constraint against data it thinks it created.
  Put the pid in the path.
- **`match` arms returning `QueryResult<Sqlite, _>` and
  `QueryResult<Postgres, _>` are different types.** `.map(|_| ())` before the
  `?` makes both arms `anyhow::Result<()>`.
- **`bad_request` in the directory routes is `AppError::Validation`, which is
  422**, not 400.

### Testing it

```bash
cargo test -p lorehaven-domain --test vote_decay
cargo test -p lorehaven-app --test vote_decay_score --test vote_decay_parity \
                           --test vote_refresh --test vote_unvote \
                           --test vote_list_ranking --test milestone_39
# then again with LOREHAVEN_TEST_PG_URL set for the dual-backend path
cd frontend && npx vitest run src/routes/Directory.test.ts
```

**The two tests that catch the most:** a decay test that forgets to clear
`min_votes` is measuring the exemption and reporting it as "decay is broken";
and the list test that asserts the *same* expression ranks and displays, since
those are written out twice because PostgreSQL will not accept a SELECT alias
in ORDER BY when it wraps a subquery.

**Left open deliberately, not forgotten:**

- **The denormalised `directory_entries.score` is still written** by the vote
  transaction, undecayed. Nothing reads it any more and the list computes its
  own, so the next step is a migration that drops the column rather than
  another reader to keep in sync. `list_entries` (the undecayed variant) is
  kept only because `vote_list_ranking` compares the two.
- **No browser E2E.** The component tests cover the withdraw control and the
  three decay notes, but nothing drives the real page. The Playwright
  directory suite is the next thing to add.
- **No production deployment or smoke test** for any of this. Nothing since
  `3d4ba0f` has been restarted on thinkcentre.
- **The `Top` sort is the only one affected.** `New` is `created_at` and never
  looked at the score, so it is untouched.
- **No per-entry decay in the single-entry `GET`.** `get_entry` returns
  `my_vote` but not `decay`, so the entry page has the same information gap
  the list had.

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

## M57B — The analytics page is reachable; a status needed a subject

`/analytics` existed as a component with twenty unit tests and was not in the
router. A component that is not in `FIXED_ROUTES` is invisible, and a test that
renders it directly cannot tell an unreachable page from a reachable one, so all
twenty were green while no reader could open the page. Four coordinated edits
plus a nav entry; the nav entry matters, because a page reachable only by typing
its URL is not a page a reader has.

### A data-integrity bug this found

`PUT /library/items/{id}/status` trusted `id`. Any UUID in the instance got a
200 and left a `reading_status` row for a subject that does not exist. The
reader's own dashboard then counted nothing, for reasons visible nowhere on the
page. Two problems, one predicate: the subject must exist, and it must be
*this reader's* item — without `account_id` in the check, a reader who learned
another reader's item id could write a status against it. `library_item_exists`
does both; `add_item_tag` had the identical hole and gets the same guard. Four
tests, including a happy path, because a gate that refuses everyone is
indistinguishable from a gate that works.

Refusals are 404 rather than 403 on purpose: a 403 confirms the item exists,
which is a small leak across accounts.

### A layout bug that predates the analytics work

The desktop nav had `flex-wrap: wrap` and sixteen items, so at 1280px it wrapped
to three rows and a `position: sticky` header measured **177px** — a quarter of
a 720px viewport — covering everything scrolled to underneath it. Found because
Playwright could not click a control the header sat on. Fixed by pinning the nav
to one row with `flex-wrap: nowrap` and horizontal scroll, which is what makes
`--header-height` a true constant instead of a function of how many items fit.
Header is now 65px. The E2E asserts `scroll-padding-top >= header height`, so a
drift in the token fails a test rather than quietly clipping a keyboard reader's
focus ring.

The whole analytics E2E file went from 4 minutes (three tests timing out at 180s)
to 14 seconds once the header was fixed.

### Still open, deliberately

`library_items` is created in exactly one place: `imports::upsert_library_item`,
called by the import runner. A work published on this instance never gets a
library row, so a reader who finishes one has no subject to mark and
`own.reading.basic` shows them a permanent zero. `library_items.work_id` exists
and is nullable precisely for the materialised case, so the model anticipated
this and the write path was never built. Tracked as M57A-09.

The E2E asserts the honest 404 rather than working around it, and the comment
says why — a workaround is what hid this in the first place.
