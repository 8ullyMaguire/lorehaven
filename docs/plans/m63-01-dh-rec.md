# M63-01 — `dh rec`: the anchor verb

**Decision recorded elsewhere, honoured here:** `docs/plans/cli-placement.md` puts the CLI in this
workspace as `crates/cli`, binary `dh`, an **HTTP client of `/api/v1`** that may not depend on
`crates/app` or `crates/db` at runtime. This plan builds the first verb, and therefore the crate.

The anchor ask, from `docs/ideas/2026-11-01-idea-sweep-power-users.md`:

```
dh rec --fandom naruto --updated 7d --engine rrf --sort wilson
```

"Fics in this fandom, changed in this window, ranked by this metric, using this engine — and show me
why the order is what it is." Today every part of that sentence requires either a browser or a
restart: the window grammar exists only in a URL, and `rec.mode` is instance config.

## What this milestone does NOT do

- No TUI (`dh tui`), no saved queries, no `dh q`, no snapshot cache. Those are M63-05 and later.
- `--sort` accepts the catalogue's five metrics (likes/quality, aggregate, views, words, updated).
  The analytics metric registry (`wilson`, `hidden_gem`, …) is M63-03 and is a different store; when
  it lands, `--sort` resolves against it and this parser keeps the five as the fallback.
- `--seed <work>` / `--author` are M63-04.
- `--jsonl --fields` is M63-05. `--json` here prints the server's page verbatim.

## Facts this plan is built on, measured — not assumed

```
$ sudo -u postgres psql -tA -F'|' -d darkhold -c "SELECT ce.canonical, count(DISTINCT li.id) \
  FROM library_items li JOIN work_tags wt ON wt.work_id = li.work_id \
  JOIN canonical_entities ce ON ce.id = wt.node_id WHERE ce.kind='fandom' GROUP BY 1 ORDER BY 2 DESC LIMIT 3"
Harry Potter|2311
Marvel|349
A Song of Ice and Fire - George R. R. Martin|338
Naruto|265
```

- **233 fandom nodes**, 30,982 `work_tags` rows, all of which join a real node.
- **7,626 of 141,317 catalogue rows carry a fandom tag** — 5.4 %. The other 94.6 % are untagged and
  `--fandom` must exclude them *and say so*, or the user reads "3 results" as a ranking failure.
- The table is `canonical_entities` (`kind` ∈ tag 3,103 / character 1,014 / relationship 410 /
  fandom 233), joined through `work_tags.node_id = canonical_entities.id`. `taxonomy_nodes` appears
  only in older test comments; it is the pre-rename name.
- `norm` is the lowercased canonical, so a case-insensitive exact match is `ce.norm = lower(trim($1))`.
- Window parsing already exists and is shared: `darkhold_domain::window::parse` (M63-02), the same
  function the route uses. The CLI does **not** reimplement it — it sends the raw text and lets the
  one parser decide, so a window typed at the terminal cannot mean something else than in a URL.

## Design

### 1. `GET /api/v1/catalogue?…&fandom=naruto` (new param)

`CatalogueQuery` gains `fandom: Option<String>`; the predicate is an `EXISTS`, not a join, so a work
with two matching fandoms cannot duplicate rows and the page's `limit` stays honest:

```sql
AND EXISTS (
    SELECT 1 FROM work_tags wt
      JOIN canonical_entities ce ON ce.id = wt.node_id
     WHERE wt.work_id = library_items.work_id
       AND ce.kind = 'fandom'
       AND ce.norm = ?
)
```

`library_items.work_id IS NOT NULL` is implied: a NULL never satisfies `EXISTS`.

The route resolves the name **before** running the page query, so an unknown fandom is a `422` that
names the field and suggests the closest known one, rather than an empty page that reads as "no
matches":

```
422 fandom: no fandom called "narut"; did you mean "Naruto"?  (233 fandoms known)
```

`GET /api/v1/catalogue/fandoms` lists them with item counts (top 500 by count), which is also what
the suggestion is drawn from — one query, two uses.

### 2. `GET /api/v1/rec/run` (new, session-authed)

The catalogue is public; the recommendation engines are not, because every strategy reads the
account's own history (`seen_work_ids`) and several read other readers' co-occurrence. So the
engine lives behind a session.

```
GET /api/v1/rec/run?fandom=naruto&updated=7d&engine=rrf&sort=likes&limit=20&explain=1
```

| param | meaning |
|---|---|
| `fandom`, `updated`, `refreshed`, `added`, `source`, `min_likes`, `q` | the same filters as `/catalogue`, from the same struct |
| `sort` | the catalogue metric, used to order what the engine does not rank |
| `engine` | `catalogue` (no engine — metric order), `legacy`, `rrf`, or a registered strategy name; default is `config.discovery.rec_mode` |
| `explain` | include per-strategy contributions |

Ordering rule, stated so the UI can explain itself: **the blend decides the head, the metric decides
the tail.** Ids the engine ranked come first in blend order; the candidates it did not rank follow in
metric order. This is what makes `--engine rrf` comparable with `--engine catalogue`: same candidate
set, same tail, only the head differs.

`legacy` vs `rrf`: `legacy` is the pre-registry blend path (`rec_mode="legacy"`), `rrf` is the
registry blend. Both are produced by `RecRegistry::generate_traced` today (the legacy path is the
same computation the old blend performed), so the difference the operator sees is the strategy set
and the RRF constant, not two code paths that can drift.

Selection: `engine=<strategy>` narrows the registry to that one strategy via `RecRegistry::only`
(which returns `None` for a disabled strategy → `422 engine: <name> is not enabled on this instance`).

Response:

```json
{
  "engine": "rrf",
  "sort": "likes",
  "candidates": 265,
  "tagged_rows": 7626,
  "items": [{"id": "…", "title": "…", "author_text": "…", "source_key": "sb",
             "likes": 812, "word_count": 21044, "status": "complete",
             "source_updated_at": "2026-10-27T14:02:11Z", "source_url": "…",
             "rec_score": 0.0322, "rec_rank": 1}],
  "strategies": [{"name": "cooccurrence", "produced": 200, "matched": 37,
                  "top": [{"id": "…", "score": 0.0164}]}]
}
```

`rec_score` is the sum of each strategy's RRF contribution, recomputed from `per_strategy.ranked`
(the report already carries it), so the number the CLI prints is the number the server used.

### 3. `crates/cli` — binary `dh`

```
dh rec [OPTIONS]
  --fandom <NAME>          fandom, as the catalogue spells it (case-insensitive; suggest on miss)
  --updated <WINDOW>       7d | @2026-10-27 | 2026-01-01..2026-10-15 | >30d | <2w
  --refreshed <WINDOW>     when the crawler last rechecked it
  --added <WINDOW>         when it entered this catalogue
  --source <KEY>           sb, sv, qq, ao3, royalroad, …
  --min-likes <N>
  --q <TEXT>               substring over title and author
  --sort <METRIC>          likes|quality, aggregate, views, words, updated   [default: likes]
  --engine <E>             catalogue, legacy, rrf, or a strategy name       [default: configured]
  --explain                print what each engine contributed
  --limit <N>              1..=100                                          [default: 20]
  --json                   print the server's JSON verbatim
```

Dependencies are all in `[workspace.dependencies]` already, so this adds no download: `clap`,
`serde`, `serde_json`, `reqwest` (`rustls-tls`, no cookies), `tokio`, `anyhow`, `time` for nothing
(the window is never parsed client-side).

Output is a table with a stable column order and a fixed-width number column so `sort -n` works:

```
  1   812 likes   21.0k words  complete   2026-10-27  Naruto: A Quiet Sort        by Esama        [sb]
```

`--explain` appends:

```
engine rrf over 265 candidates (7,626 of 141,317 rows carry a fandom tag)

strategy          produced  matched  head
cooccurrence           200       37  0.0164  0.0148  …
time_decay             265       44  0.0161  …
```

Error mapping, which is the part users actually experience:

| condition | what `dh` prints | exit |
|---|---|---|
| `422` | the server's `message`, verbatim, plus `field` when present | 2 |
| `401` | `this engine needs a token: set DARKHOLD_TOKEN (engines read your history)` | 2 |
| connection refused | `nothing listening at <url> — is the server running? (--base-url)` | 1 |
| other `5xx` | status + first line of the body | 1 |

### 4. Tests

`crates/db/tests/catalogue_fandom.rs` — both engines:
- `a_fandom_filter_returns_only_that_fandom`
- `a_fandom_with_no_rows_returns_an_empty_page_not_an_error` (known fandom, zero dated rows)
- `two_fandoms_on_one_work_do_not_duplicate_its_row` (the `EXISTS` invariant; a `JOIN` would)
- `an_unknown_fandom_is_reported_by_the_route_not_the_store` — store returns empty; the 422 is the
  route's, tested in `crates/app/tests/m63_01_rec.rs`.

`crates/app/tests/m63_01_rec.rs`:
- `an_unknown_fandom_names_the_field_and_suggests` → 422, `message` contains `did you mean`
- `the_catalogue_route_filters_by_fandom` → seeded rows, `fandom=Naruto` returns only tagged ones
- `engine_catalogue_orders_by_the_metric`
- `engine_rrf_puts_ranked_ids_first_and_metric_order_after`
- `an_engine_that_is_not_enabled_is_a_422`
- `explain_reports_a_strategy_that_produced_nothing` — the finding shadow mode exists to surface

`crates/cli/tests/rec.rs` — the CLI's contract, against a router built in-process:
- `rec_parses_the_anchor_ask` (`--fandom naruto --updated 7d --engine rrf --sort wilson` parses;
  `wilson` is rejected today with a message naming M63-03, not silently mapped to likes)
- `rendering_is_stable_across_a_new_column` — a field the CLI does not know about does not shift
  the table (it renders from the known keys only)
- `an_unknown_fandom_exits_2_with_the_server_message`

## Steps, each with its verification

1. **`CatalogueQuery.fandom` + the `EXISTS` predicate.**
   `cargo test -p darkhold-db --test catalogue_fandom` → `4 passed`.
2. **Route param + `422` with suggestion + `/catalogue/fandoms`.**
   `cargo test -p darkhold-app --test m63_01_rec` → the four catalogue cases pass;
   `curl -s -o /dev/null -w '%{http_code}' 'localhost:8081/api/v1/catalogue?fandom=narut'` → `422`.
3. **`/api/v1/rec/run`.**
   `cargo test -p darkhold-app --test m63_01_rec` → `6 passed`.
   Manual: `curl -s -H "Authorization: Bearer $T" 'localhost:8081/api/v1/rec/run?fandom=naruto&engine=rrf&explain=1' | jq '.candidates, (.strategies|length)'`.
4. **`crates/cli`.**
   `cargo test -p darkhold-cli` → parse + render + contract tests pass.
   `cargo run -p darkhold-cli -- rec --fandom naruto --updated 7d --limit 5` against the live
   instance → five rows, then `--explain` shows the strategy table.
5. **Docs:** README gains a `dh rec` example; this plan gains a "what landed" section with the live
   numbers; `docs/requirements.csv` M63-01 → `implemented-verified-e2e`.

## Risks, and what would make this plan wrong

- **Data coverage.** 5.4 % of the catalogue carries a fandom. If the user's real question is "all
  Naruto fics", the answer is "265 of them are tagged" and the fix is the tag backfill, not this
  verb. The CLI must print the coverage line whenever `--fandom` is used, and the plan files the
  gap as its own item rather than pretending the filter is complete.
- **`engine` on a public route is impossible.** If a token-less `dh rec --engine rrf` is expected to
  work for anonymous users, the design is wrong: strategies read per-account history. This plan says
  so in the error message instead of degrading silently to the catalogue order.
- **Schema drift on the live instance.** `migrations/postgres` creates 11 tables the live database
  does not have (`chapters`, `bodies`, `chapter_revisions`, `import_chapters`, `ship_participants`,
  the five pre-rename `taxonomy_*`). Everything this milestone touches is present, so the milestone
  is verifiable, but a reconciliation is a separate item and is filed as such.

## What landed (`f13597f3`)

| piece | where |
|---|---|
| fandom filter | `CatalogueQuery.fandom`, one `EXISTS` predicate, both dialects |
| coverage | `catalogue_fandom` + the page's `fandom_tagged_rows` / `catalogue_rows` |
| suggestion | `suggest()` in the catalogue route: prefix, then substring, then edit distance |
| fandom list | `GET /api/v1/catalogue/fandoms` (top 500 by item count) |
| engines | `GET /api/v1/rec/run`, session-authed, `catalogue`/`legacy`/`rrf`/`<strategy>` |
| CLI | `crates/cli`, binary `dh`, `dh rec` with every flag this plan listed |

Two things the plan said that the code does differently, both deliberate:

- **`--engine` defaults to the catalogue route, not to `rec.mode`.** `dh rec --fandom naruto`
  should not need a token, and a token-less request cannot reach an engine. So the flag's absence
  means "the public metric order", the flag's presence means "run this engine, with a session", and
  `--explain` without `--engine` asks for `rrf` — because there is nothing to explain about a plain
  metric ordering, and answering "no strategies ran" to a request that asked for an explanation is
  worse than running the default engine and showing it.
- **`explain=1` is parsed as text, not as a bool.** `#[serde(default)] explain: Option<bool>` makes
  axum answer `400 Failed to deserialize query string: explain: provided string was not true or
  false`, which reads as a syntax error. `1`/`true`/`yes`/`on`/bare are now all accepted.

### Verification

| claim | command | output |
|---|---|---|
| the filter works on both engines | `cargo test -p darkhold-db --test catalogue_fandom` | 5 passed on SQLite, 5 passed on PostgreSQL |
| the routes work on both engines | `cargo test -p darkhold-app --test m63_01_rec` | 9 passed on SQLite, 9 passed on PostgreSQL |
| the CLI's parsing and rendering | `cargo test -p darkhold-cli` | 10 passed |
| nothing else moved | `cargo test -p darkhold-db -p darkhold-app --lib` | 236 + 157 + 784 passed |
| clippy | `cargo clippy -p darkhold-db -p darkhold-app -p darkhold-cli --all-targets` | no new warning (the three in `work_ratings.rs` are pre-existing) |

Mutations, both reverted:

- bind the fandom name **upper-cased** → `a_fandom_filter_returns_only_that_fandom`,
  `one_matching_work_is_one_row` and `the_filter_is_case_insensitive…` fail on both engines. That is
  what proves the `norm` match is load-bearing rather than incidental.
- never **suggest** a near name (`suggest("zzzznomatchzzzz", …)`) → exactly one test fails, with the
  message a user would have seen: `fandom: no fandom called "narut". (1 fandoms known)`.

A first attempt at the second mutation removed the `suggest()` call outright; that fails the build on
`dead_code` (warnings are denied), which is not a verdict about the test. The mutation has to keep
the code compiling to say anything.
