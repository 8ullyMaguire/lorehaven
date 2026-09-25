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
