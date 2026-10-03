# Lorehaven — remaining work, 2026-10-03

Written at the start of the session that continues M45-57 and M45-22. Base commit
`74554ff`. Two named plans are in flight with steps already partly landed; this
file is the authoritative list of what is left, in the order it will be done.

## Environment this session needed before anything ran

| Fact | Value |
|---|---|
| Toolchain | `export PATH="$HOME/.cargo/bin:$PATH"` first. System `rustc` is broken (partial Arch upgrade). |
| Docker daemon | Was stopped. `sudo systemctl start docker` — needed before any `sudo docker`. |
| PostgreSQL | Container `lh-pg-test` on **127.0.0.1:55433**, user/db `lorehaven`/`postgres`, password `lorehaven`. Was stopped; `sudo docker start lh-pg-test`. |
| SQLite | Default. `data/lorehaven.sqlite`. |
| Test URL | `LOREHAVEN_TEST_PG_URL='postgres://lorehaven:lorehaven@127.0.0.1:55433/postgres'` |
| btrfs | Metadata DUP pool was at 12.63/13.00 GiB. Builds are SLOW, not stalled — a test binary burning 15 CPU-seconds per 15 s is compiling, not stuck. Check `ps -eo pid,stat,etime,comm \| awk '$2 ~ /^D/'` before believing a hang. |

## Step 0 — M45-57 step 7: HTTP routes (uncommitted, compiles, unverified)

`crates/app/src/routes/source_adapters.rs` exists (188 lines, 5 routes incl. the
`{id}` read), registered in `server.rs`, and its `ROUTE_TABLE` rows are in
`route_inventory.rs`. `cargo check -p lorehaven-app --tests` is clean. What does
**not** exist yet is the test file the plan names: `source_adapter_routes.rs`
covering §55.8's 401 and 403 lines.

## The list

| # | Work | Plan step | Done when |
|---|---|---|---|
| 1 | M45-57 route tests, both engines | 7 | `source_adapter_routes.rs` green on SQLite and PostgreSQL |
| 2 | M45-57 §55.5 automated check | 8 | `declarative_check.rs` green, report names the manifest version |
| 3 | M45-57 tracker + docs | 10 | M45-57 row says "Path A shipped, Path B gated", not "done" |
| 4 | M45-22 migration 0114, both dialects | 2 | 114/114 apply on both engines |
| 5 | M45-22 domain `concierge.rs` | 3 | 7 named tests, each seen red |
| 6 | M45-22 store | 4 | green on both engines, privacy test proven red |
| 7 | M45-22 routes | 5 | green on both engines, `ROUTE_TABLE` complete |
| 8 | M45-22 WIP notify | 6 | 6 named tests, the consume-the-watch test proven red |
| 9 | M45-22 frontend | 7 | `Concierge.svelte` + tests, build before test |
| 10 | M45-22 tracker + docs | 8 | row updated with evidence |

After items 1–10 the tracker has **27** `planned` rows left (M45-23 north-star,
M45-25 … M45-55) and 6 `specified` (M46-05 search). Those are each multi-week
specs of their own and are out of scope here; they are listed so the number is
honest rather than implied.

## Not doing, on purpose

- **Path B (WASM), §55.4.** §55.6 gates it. `scripts/check-wasm-gate.py` fails
  the build if a WASM runtime is adopted. `wasmi 0.4` does not compile on this
  toolchain at all, so this is not a one-line change away.
- **§36.11 mood journal.** §54 reuses §15.8's mood *vocabulary*; the journal is
  separate work and the plan says so.
- **The pre-existing CI reds.** `scripts/check-sqlite-migration-syntax.py
  --self-test` and `scripts/check-pg-uuid-casts.py` both reproduce on a clean
  stash, so they are not caused by anything here. Named in the final report
  rather than quietly fixed or ignored.