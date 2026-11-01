# Idea sweep — power users (2026-11-01)

Priority axis: **Power Users — speed, tools, recognition for the engine.**
Companion to `docs/ideas/2026-10-31-idea-sweep-north-star.md` (breadth across all axes);
this sweep goes deep on one axis only.

The anchor ask, in the operator's words: a CLI for recommendations where the user chooses the
time frame, the fandom, the recommendation engine and the quality metric — *"naruto fics that
updated last week, sorted by metric X, run by algorithm Y"*. Everything below is that idea, or
a power-user surface that makes it more useful.

Grounded in what exists: `dh` CLI stub + offline cache card (in progress), `rec.mode` config
with legacy|rrf and the strategy registry (shipped), `/api/v1/catalogue?source=&sort=quality`,
`/api/v1/fics?host=&sort=quality` on ascend, saved-search and search-operator cards (discussion),
scoped API tokens (shipped issuance UI), `requirements.csv` as the roadmap ledger.

Effort tags: **S** ≤2 days, **M** ≤5, **L** >5.

## The flagship: `dh rec`

- **[M] `dh rec` with per-call engine selection and `--explain`** — `dh rec --fandom naruto --updated 7d --engine rrf --sort wilson`. Engine is currently instance config (`rec.mode`), so comparing legacy against rrf needs a restart and an operator; a per-call override makes it an experiment a user can run at the terminal, and `--explain` prints each strategy's contribution so the difference is legible.
- **[S] `--updated 7d|24h|2026-10-01..2026-11-01` as a first-class filter** — mapped to `works.updated_at` with the same window grammar everywhere it appears (rec, q, watch, digest), so "updated last week" is a phrase the whole tool understands once.
- **[S] `--sort` accepts any registered metric by name** — `wilson`, `kudos`, `kudos_per_1k_views`, `kudos_per_hour`, `completion`, `updater_reliability`, `hidden_gem`, `freshness_weighted_quality`. The metric registry that already backs analytics becomes the single place a sort name is defined, so a new metric is not a new route.
- **[M] `--seed <work>` for similar-to-this-work** — tag plus cooccurrence vectors already power the rec strategies; exposing the seed as a CLI argument turns "more like this one" into a pipeable query instead of a button.
- **[S] `--author <pseud> --exclude-read`** — backfill an author's catalogue minus what the reader has already finished, using attention records as an implicit negative set rather than a separate flag per surface.
- **[S] `--min-chapters`, `--max-words`, `--completion complete|wip`, `--status`** — structural filters pushed into SQL. Client-side filtering of a 20-row page is a lie about what the engine ranked; these belong in the query.
- **[S] `--not-tags tag,tag` and `--require-tags`** — hard exclusions and hard requirements kept separate from taste scoring, so a reader who will not touch a trope can say so once and keep it in a saved query.
- **[S] `--novelty 0.7`** — per-call mapping onto the diversity budget / MMR knob that already applies after the blend.
- **[S] `--why` prints three contributing reasons per row** — one line each ("tags Naruto+TimeTravel match 0.82", "author you finished 3 works by", "co-readers of X"), reusing the slot explanation strings the discovery surface already produces.
- **[S] `--jsonl --fields id,title,kudos,updated_at`** — field projection so `dh rec ... | jq -r .title` is exact and cheap, and so a script is not broken by a new column.
- **[M] `--save naruto-weekly` persists the query server-side** — a saved query is shareable (URL), schedulable (`dh cron install`), and openable in the browser with one command; it also makes `--digest-diff` meaningful.
- **[S] `--digest-diff` prints only rows new since this saved query last ran** — the actual recurring need behind a weekly ritual, without re-reading 20 known rows.
- **[S] `--table|--wide|--oneline --cols`** — terminal output is a contract: default is one line per fic with title/kudos/updated/words, `--wide` adds columns, `--cols` picks them, and nothing wraps without `--wrap`.
- **[S] `--link md|epub|url`** — emits ready-to-paste markdown or a URL list for a reading queue, because the next step after choosing is always moving the list somewhere.
- **[S] `--dry-run` prints the resolved SQL, the parameter set and the engine plan** — before any network call, so a surprising result set is diagnosable without reading code.
- **[S] `--fresh` / `--cache-ttl 60`** — explicit cache control per call. A recommendation you cannot force-recompute is a recommendation you cannot debug.

## Query language for the terminal

- **[M] `dh q '<expr>'` with a real grammar** — `tag:naruto AND kudos>=100 AND updated>=-7d AND words>50000 AND completion:complete`. One parser shared with `--save`, watch, feeds and the web search box, so an expression typed in the terminal is the same object everywhere.
- **[S] `--from-file queries.txt`** — a file of statements runs as a sectioned report; a personal query library becomes a file you can diff, not browser history.
- **[M] `tag:naruto+` expands synonyms and implied tags** — the tag graph is already maintained by wrangling; the `+` operator is what makes it usable without a UI.
- **[S] `--explain` prints the plan and index used** — slow query diagnosis at the terminal, no `psql` needed.
- **[S] `--count` fast aggregate mode** — COUNT pushdown for corpus surveys ("how many complete Naruto fics over 50k words").
- **[S] `--group-by tag --top 20`** — histogram mode, e.g. which fandoms gained the most fics this week; the same aggregate engine that answers it on the web.
- **[S] `--csv|--tsv|--no-header`** — spreadsheet and `awk` pipelines both work, and neither needs a new flag later.
- **[M] `.darkhold/queries/*.q` in the repo** — a user's query library is versioned, reviewable and shareable, in the same shape as the instance's own config.
- **[S] `--watch 60s`** — re-runs and prints only deltas, a terminal radar for a launch, an exchange deadline or a fandom's busy week.
- **[S] `--format md|html|json`** — markdown table for a post, HTML for a page, JSON for everything else.
- **[M] `--all` streams every page through the cursor API** with `--limit`/`--offset` still available, so "export everything matching" does not need a bespoke script.
- **[S] Parse errors point at the token and exit 2** — usage errors (2) distinguished from runtime errors (1) so a wrapper script can tell "you typed it wrong" from "the instance is down".

## Library and bulk operations

- **[M] `dh lib sync --since`** — incremental pull of the library into a local sqlite mirror; the mirror is also what makes `--offline` and `dh q` fast.
- **[S] `dh lib diff`** — local vs remote added/removed/changed, printed before any push, so a bulk operation is reviewed before it is real.
- **[S] `dh lib push --dry-run` with `--yes` to apply** — destructive verbs default to preview; the flag that skips the preview is spelled out and logged.
- **[M] `dh lib tag --from-csv` with a per-row error report and a rollback file** — batch editorial work on 500 rows is the power user's actual job, and a partial failure must be resumable, not a mystery.
- **[S] `dh lib status --bulk '<query>' read`** — set reading status for a result set, one confirmation, one audit entry.
- **[M] `dh lib undo <txn>`** — reverses the last bulk operation by replaying its inverse from the operation log; the log exists for audits, so the undo is evidence-backed rather than best-effort.
- **[S] `dh lib stale --days 90 --mark dropped`** — finds library entries untouched for N days; the "I have 900 bookmarks and read none" cleanup, as one verb.
- **[M] `dh lib queue` with `next`/`pop`/`priority`** — a persistent read queue that survives sessions and is shared with the web reader.
- **[M] `dh lib merge <a> <b>`** — merges two library entries pointing at the same work, reusing the perceptual-hash and URL dedup that exists for media.
- **[M] `dh lib import --from ao3-csv|kudos.json|bookmarks.html` with a mapping file** — cold start for a power user migrating a decade of bookmarks, dry-run first.
- **[S] `dh lib stats --by fandom|author|year`** — the user's own library as a terminal dashboard; feeds the gamification surfaces without new queries.
- **[S] `dh lib export --format jsonl`** — lossless machine export, so the library can be processed by any tool without the API.

## Reading and output

- **[S] `dh read <work> --chapter N`** pages cached bodies through `$PAGER`; the instance already stores chapter revisions, so the terminal reader needs no new source.
- **[S] `dh read --offline`** never touches the network — train mode, and the honest test that the local mirror is complete.
- **[M] `dh read --epub -o fic.epub`** builds an EPUB on demand from cached chapters; the export pipeline exists, this is the CLI wrapper that makes one fic a one-liner.
- **[M] `dh read --follow`** watches a WIP and notifies on new chapters, reusing the watch daemon rather than a second polling loop.
- **[S] `dh read --progress`** prints position plus estimated remaining time from chapter word counts, and writes the position back so the web reader resumes in the same place.
- **[S] `dh read --highlight 'phrase'`** greps cached bodies locally with context windows — full-text search over what you have already downloaded.
- **[S] `dh read --stats`** prints words, reading time and chapter cadence before committing to a 400k-word fic.

## Automation, feeds and alerts

- **[M] `dh watch <saved-query> --exec 'cmd'`** — a daemon that polls a saved query, debounces, and runs a command on new hits; the difference between a saved search and a workflow.
- **[S] `dh watch --notify ntfy|desktop|webhook`** — pluggable sinks so the notification path is not tied to one provider.
- **[M] `dh cron install --every 6h --jitter 10m`** — writes a systemd user timer for a saved query or a recheck job; the instance already runs timers, this puts the same mechanism in a user's hands without them writing a unit file.
- **[M] Per-saved-query RSS/Atom feed** — any feed reader can subscribe to a taste query; the cheapest possible client for a power user who lives in one.
- **[S] Per-saved-query JSON Feed with ETag/If-None-Match** — conditional GET so polling a query costs a 304, not a query.
- **[M] `dh digest --weekly`** — a markdown/terminal digest per saved query with diffs and a stable ordering, so two consecutive digests can be compared with `diff`.
- **[M] `dh webhook add --event work.updated --filter 'tag:naruto'`** — filtered hooks evaluated server-side; the filter language is the same `dh q` grammar.
- **[S] `dh hooks test`** — sends a sample payload, prints the response and the retry schedule; webhooks debugged in a minute instead of an afternoon.
- **[S] `dh notify --stdin`** — turns any JSONL stream into deduped desktop notifications, so `dh watch` and ordinary shell pipelines share one sink.
- **[S] `dh status`** — one line: instance reachable, token scope, cache age, pending jobs, API version. The first command anyone runs when something feels wrong.

## Operator verbs (the power user who also operates)

- **[S] `dh admin migrate --dry-run`** — prints pending SQL and the schema diff before applying; a migration you can read is a migration you can refuse.
- **[S] `dh admin backup --to -`** — streams a consistent dump to stdout for `ssh host dh admin backup | zstd > backup.zst`.
- **[S] `dh admin health`** — machine-readable checks with one exit code per check, so monitoring is a cron job and not a dashboard.
- **[S] `dh admin reindex --tags`** — rebuilds derived tag aggregates with progress and ETA.
- **[S] `dh admin seed --from docs/requirements.csv --dry-run`** — wraps the existing seed script with a report, so the roadmap ledger has one entry point.
- **[S] `dh admin quota`** — per-token rate limits and current usage, visible before a batch fails at row 400.
- **[M] `dh admin explain-rec <work>`** — dumps the full feature vector and every strategy's score for one work: the operator's microscope for "why is this ranked first".
- **[S] `dh admin logs --follow --filter level=error`** — structured log tail with a filter expression, not `grep` over prose.

## Machine ergonomics

- **[M] Versioned JSON schema with `--api-version` and a compatibility table** — scripts break silently when output shape changes; an explicit version with a stated deprecation policy makes it a loud failure.
- **[S] Shell completions generated from the clap definition for bash/zsh/fish** — a subcommand nobody can discover may as well not exist; generation keeps completions honest by construction.
- **[S] Man pages generated from the same definition** — one source, no drift.
- **[S] Config profiles `--profile work`, `DARKHOLD_PROFILE`** — per-profile instance URL and token, so one user can sit on a personal and a project instance without exporting variables per shell.
- **[S] `dh config get/set` with dotted keys and validation, plus `--show-defaults`** — every knob discoverable and settable from the tool that reads it.
- **[M] NDJSON streaming for large result sets** — `dh q --all | jq` starts printing immediately instead of after the last page.
- **[S] Exit codes as a documented API** — 0 ok, 1 error, 2 usage, 3 not found, 4 auth, 5 rate-limited, tested as a contract.
- **[S] `dh upgrade` with checksum verification and `--dry-run`** — a self-updating binary that verifies what it downloaded.

## Speed

- **[L] Local sqlite mirror of the catalogue, refreshed incrementally** — sub-millisecond `dh q` and true `--offline`, at the cost of a sync protocol; the same download machinery a mirror instance needs anyway.
- **[M] Connection reuse, HTTP/2 and gzip on batch fetches with `--parallel N` capped** — the difference between 20 seconds and 3 for a 500-work import, without hammering a $5 VPS.
- **[S] `dh prefetch --top 20`** — warms the local cache for the top recommendations before a train ride.
- **[S] Query result cache keyed by (expression, params, catalogue version)** — with explicit invalidation when a work is updated, so freshness and speed are not a trade the user has to guess at.
- **[M] `dh bench`** — times the 20 canonical queries and prints a table, so a regression is visible from a user's machine, not only in CI.
- **[S] Adaptive page size** — raises the page size until latency crosses a threshold, so the default is fast on a LAN and polite on a phone tether.

## Recognition for the engine

- **[M] Contributor score for CLI users, from the operation log** — imports, tags applied, reports filed, all recorded already; shown on a private profile so the contribution is visible to the person who made it.
- **[M] Curator clout with per-capability thresholds earned by accepted contributions** — power users get faster lanes (auto-approved tags, larger import quotas), not louder votes; recognition expressed as capability is the only kind that does not corrupt ranking.
- **[S] `dh stats --by week` streak for first-comment-on-a-new-work** — the one behaviour worth encouraging in a fic site, surfaced in the tool power users actually run.
- **[S] Tooling badges: first webhook, first shared saved query, 10k works indexed** — cheap, honest, and impossible to farm without doing the thing.
- **[M] Opt-in public leaderboard of tag-wrangling throughput, with an anonymous mode** — curation labour is visible or it is invisible; anonymity keeps it safe.
- **[M] "Engineer" rank gated on usage breadth, unlocking beta engine flags** — recognition as access to new machinery, which is what this audience actually wants.
- **[S] `dh stats --year`** wrapped-style personal report rendered to markdown for sharing.
- **[S] Credits for accepted corpus contributions, spendable on export quota or bounty boosts, never cashable** — the faucet stays closed-loop, which is already the instance's stated rule.

## Guardrails that make power use safe

- **[M] Scoped tokens per command family, with the missing scope named on refusal** — `dh` never sends a request it knows will be refused, and the refusal says `token lacks library:write`, not 403.
- **[S] Rate-limit awareness: back off with jitter on 429 and print the retry window** — a batch that dies at row 400 because of a 429 is a tooling bug, not a user error.
- **[S] `--dry-run` is the default for every bulk verb; `--yes` applies** — the safe path is the short path.
- **[S] `dh redact`** — strips pseudonyms and emails from any JSON output for pasting into a bug report; the privacy rule applied at the tool boundary.
- **[S] `dh audit --since 7d`** — the account owner sees every write their tokens made; automation is accountable to the person it acts for.
- **[S] `dh doctor`** — diagnoses token scope, clock skew, TLS, DNS and API version drift, printing one suggested fix per problem.
