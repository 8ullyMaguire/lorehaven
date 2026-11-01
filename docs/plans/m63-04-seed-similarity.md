# M63-04 — `dh rec --seed <work>`, `--author <pseud>`, `--exclude-read`

Milestone row (power-user): **`dh rec --seed <work>` (similar-to-this-work) and
`--author <pseud> --exclude-read`**. The row's own note says "tag/cooccurrence vectors already
power the strategies; attention records are the implicit negative set. No new ranking code, a new
entry point."

**That note is wrong on its premise, and this plan is built on the measurement instead.** Measured
on the live database 2026-11-01:

| what the note assumes | what is there |
|---|---|
| tag vectors cover the catalogue | `work_tags` joins **0** of the 1,247 rows that carry likes; all 7,626 tagged rows are `pawchive` with zero likes |
| attention records are the negative set | `reading_progress` holds **1 row** in the whole instance (1 account, 1 work) |
| "no new ranking code" | similarity over tags returns nothing for any row a reader can rank, so the similarity has to come from the text |

So the entry point is new *and* it needs a real ranking function. What that function may use is
title, summary and author text — the only fields that exist on every row of every source.

## What ships

Three flags, one code path, because they are all "narrow the candidate set before the metric runs":

1. **`--seed <work>`** — resolve one catalogue row (by id, by source URL, or by exact title), score
   every other candidate by idf-weighted token overlap against its title and summary, and order by
   that. `--sort` becomes the tiebreak inside equal similarity. `--explain` prints the shared tokens.
2. **`--author <pseud>`** — exact, case-insensitive match on `author_text`. Not a substring: a
   substring match on "Oshha" also returns "Oshhalo", and a reader asking for one author's works is
   asking for one author.
3. **`--exclude-read`** — drop rows the calling account is reading (`reading_progress.subject_type =
   'work'` matched through `library_items.work_id`). Needs a session; the public route refuses the
   flag without one rather than silently ignoring it.

Every response carries the gate: `ranked N of M` (`N` = candidates that share at least one token
with the seed, `M` = the filter's total) plus `scanned` = the rows the ranker actually saw. That
last number is what stops "similar to X" reading as a claim about the whole catalogue when the
candidate cap is 5,000.

## Files

| file | change |
|---|---|
| `crates/domain/src/similar.rs` | **new** — tokenizer, idf, similarity, ranking. No database, no clock. |
| `crates/domain/src/lib.rs` | `pub mod similar;` |
| `crates/db/src/catalogue.rs` | `CatalogueQuery` + `author`, `tokens`, `exclude_account`; `filters_sql` emits them; `order_by` unchanged |
| `crates/db/src/metrics.rs` | `CandidateRow` + `title`, `summary`, `author_text`; new `similar()` |
| `crates/app/src/routes/catalogue.rs` | `CatalogueParams` + `author`, `seed`, `exclude_read`; `search()` gains a seed branch; `MaybeSession` |
| `crates/cli/src/rec.rs` | `--seed`, `--author`, `--exclude-read` |
| `crates/db/tests/catalogue_similar.rs` | **new** — SQLite + PostgreSQL |
| `crates/app/tests/m63_04_seed.rs` | **new** — route contract |

## The maths (domain, tested without a database)

```rust
pub fn tokens(text: &str) -> Vec<String>      // lower, split on non-alnum, len >= 3, stopwords out, dedup
pub struct Seed { id, title_tokens, body_tokens, author }   // title weight 3.0, body weight 1.0
pub struct Idf { df: HashMap<String, u32>, n: u32 }          // built from the candidate population
pub fn similarity(seed: &Seed, other: &Seed, idf: &Idf) -> (f64, Vec<String>)
```

- cosine over idf-weighted vectors (`weight = ln((n + 1) / (df + 1)) + 1`, so a token in every
  document contributes almost nothing and a rare shared token contributes most);
- `+0.15` for the same author, clamped to `1.0`;
- rounded to 4 dp so ordering is stable across runs;
- `shared` is the intersection ordered by descending contribution — the `--explain` line and the
  reason a reader can argue with.

Why cosine over idf rather than raw overlap: on this catalogue "gamer" and "system" appear in
hundreds of summaries, so raw overlap returns every LitRPG in the catalogue at similarity 0.9 and
the ranking stops discriminating. The idf term is what makes a shared *rare* token mean something.

## Steps, each with its verification

**Step 1 — domain.** `cargo test -p darkhold-domain similar` → `6 passed`.

**Step 2 — store.** `filters_sql` gains three arms in a fixed bind order (author, tokens, exclude),
because `counts()` and `candidates()` share the predicate and a bind out of order is a silent
mismatch on SQLite and a type error on PostgreSQL.

```bash
cargo test -p darkhold-db --test catalogue_similar                       # sqlite, 5 passed
DARKHOLD_TEST_PG_URL=postgres://... cargo test -p darkhold-db --test catalogue_similar   # pg, 5 passed
```

**Step 3 — route.**

```bash
curl -s '127.0.0.1:8081/api/v1/catalogue?seed=<id>&limit=5'          # 200, sort=similarity
curl -s '127.0.0.1:8081/api/v1/catalogue?seed=nope'                  # 404, "no catalogue row matches"
curl -s '127.0.0.1:8081/api/v1/catalogue?seed=worm&limit=3'          # 422 listing the candidate titles when ambiguous
curl -s '127.0.0.1:8081/api/v1/catalogue?exclude_read=1'             # 422, names the field, says it needs a session
curl -s '127.0.0.1:8081/api/v1/catalogue?author=Oshha&limit=50'      # every item's author_text == "Oshha"
```

**Step 4 — CLI.**

```bash
dh rec --seed 'Ghost in the City' --limit 5 --explain
dh rec --author Oshha --limit 5
dh rec --exclude-read --limit 5        # exit 2: "--exclude-read needs --token"
```

**Step 5 — gates.**

```bash
cargo test -p darkhold-domain -p darkhold-db -p darkhold-app --lib
cargo test -p darkhold-app --test m63_04_seed
cargo clippy -p darkhold-domain -p darkhold-db -p darkhold-app -p darkhold-cli --all-targets
```

Three pre-existing warnings in `crates/db/src/work_ratings.rs` are expected and untouched.

**Step 6 — mutations that must be caught** (each reverted after):

| mutation | test that must fail |
|---|---|
| title weight `3.0 → 1.0` | `a_shared_title_token_outweighs_a_shared_summary_token` |
| drop the `+0.15` author bonus | `the_same_author_breaks_a_similarity_tie` |
| `lower(trim(author_text)) = ?` (drop one side's `lower`) | `the_author_filter_is_case_insensitive` |
| replace the exclusion arm's `NOT EXISTS` with `1=1` | `an_accounts_reading_is_excluded_from_the_page` |
| route accepts `exclude_read` and drops it | `excluding_read_without_a_session_is_refused` (it would answer 200) |

**Step 7 — deploy** per `darkhold-thinkcentre-deploy`: sync the host tree, build app + cli, stop,
backup, `migrate --status`, start, then the four live curls above and `dh rec --seed` against
`127.0.0.1:8081`.

**Step 8 — docs.** `requirements.csv` row `M63-04` (the power-user one — there are two rows with
that id, see below), handoff, session note.

## Deferred, with reason

- **`--seed` over tags** — waiting on M63-32 (tag backfill). The plan for M63-04's follow-up is to
  add the tag arm to `similarity()` once `work_tags` covers rows that carry likes; the function
  already takes a `Seed` whose fields can grow.
- **`--exclude-read` at scale** — the predicate is correct and tested, but the live instance holds
  one `reading_progress` row, so the live proof can only show the route refusing the flag without a
  session. Reported as measured, not claimed.

## What landed — commit `05981e80`

Seven unit tests in `crates/domain/src/similar.rs`, six store tests in
`crates/db/tests/catalogue_similar.rs` (6 SQLite, 6 PostgreSQL), six route tests in
`crates/app/tests/m63_04_seed.rs`. Five mutations, each caught by the test that names it:

| mutation | caught by |
|---|---|
| `TITLE_WEIGHT` 3.0 → 1.0 | `a_shared_title_token_outweighs_a_shared_summary_token` |
| `AUTHOR_BONUS` 0.15 → 0.0 | `the_same_author_breaks_a_similarity_tie` |
| `lower(trim(author_text))` → `author_text` | `the_author_filter_is_case_insensitive` |
| `rp.account_id = ?` → `rp.account_id = rp.account_id` | `an_accounts_reading_is_excluded_and_nobody_elses_is` |
| route: `if false && exclude_read` | `excluding_read_without_a_session_is_refused_by_name` |

### Live proof changed the resolver, and the change is `0b1d37d9`

The first deploy (`05981e80`) answered correct JSON for a UUID seed and wrong rows
for a name. Measured against the running instance:

| spec | first version | after `0b1d37d9` |
|---|---|---|
| `Ghost in the City` | `422`, "2 rows match" and both were titled `Ghost in the City` — two `archive` rows by the same author with **0 likes** | resolves to the `sb` row with **602,176** likes, `Ghost in the City. Cyberpunk Gamer SI` |
| `With This Ring` | resolved to an `archive` stub (0 likes); neighbours shared only the token `ring` | resolves to the `qq` thread with **483,770** likes |
| `Worm` | resolved to a `pawchive` row with 0 likes | **refuses**, naming the 20 rows: 431,719 / 113,511 / 81,506 … — `Worm` is a fandom, not a title, and nothing here is 10× the next |
| `Twin Story` (a fixture tie) | — | `422` naming both rows with archive, likes and id |

The lesson generalises: **on this catalogue the exact title is usually the dead
copy**, because the estates import carries the canonical title with no endorsement
and the boards carry the readership under a longer one. A resolver that ranks by
title affinity and refuses on ties sends every reader to a stub. The rule that
replaced it — same story across archives, or a leader with >0 likes and 10× the
runner-up — is documented on `resolve_seed` and has its own test
(`a_short_name_resolves_to_the_copy_people_read`), with the 10× margin mutated to
prove the test notices.

Three things diverged from this plan, all recorded here rather than silently:

1. **`--sort` with `--seed` is refused, not a tiebreak.** The plan said the metric
   would break ties inside equal similarity. Doing that means running the metric
   ranker over the same population and threading its scores into the similarity
   sort — for a tiebreak nobody can see. A `422` that says "a seed already decides
   the order" is the honest version.
2. **`exclude_read` is a text flag, not a `bool`.** `Option<bool>` answers
   `400 Failed to deserialize query string` for `exclude_read=1`, which is exactly
   what a shell types. `rec_run` had already learned this for `explain=1`; the
   route now accepts `1|true|yes|on|0|false|no|off` and refuses anything else by
   name.
3. **`total` counts the preselection, not just the user's other filters.** The
   token `LIKE` arm is a filter, so `ranked 1 of 2` on a five-row fixture is
   correct: two rows carry the seed's words. `scanned` is the same number here
   because the cap (5,000) was not reached, and the two diverge only on a
   catalogue-wide seed.

## A defect this row surfaced: two rows share the id `M63-04`

`docs/requirements.csv` holds **two** rows per id for `M63-03`, `M63-04` and `M63-05` — one under
`area=power-user`, one under `area=catalogue`, because the catalogue sweep appended fresh rows
without checking the id space. `M63-03` was therefore closed twice for two different pieces of work.
Renumbering milestones is an operator decision, so this plan does not renumber: it disambiguates by
`area` in every reference and adds the observation to the handoff.
