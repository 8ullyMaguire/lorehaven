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
