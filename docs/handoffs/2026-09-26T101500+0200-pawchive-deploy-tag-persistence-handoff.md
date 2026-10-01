---
title: "Handoff — deploy the Pawchive tag-persistence fix (uncommitted correction in working tree)"
date: 2026-09-26
status: needs-deploy
branch: fix/pawchive-tag-and-author-parsing
---

# Handoff — deploy the Pawchive tag-persistence fix

Written by the agent that repaired the Pawchive import. Another agent is now
working in this repo, so this session stopped short of building, deploying or
merging. **There is an uncommitted correction in the working tree that must go
in before anything is deployed** — see §1.

## TL;DR

The data repair is finished and verified in production. The application fix is
written and tested but **not deployed**. A fresh import still drops the tags it
parses. One uncommitted commit is outstanding, then: build, restart, verify,
merge.

## 1. READ FIRST — uncommitted working-tree change

`git status` shows 5 modified files. They are a **correction to commit
`a605585`**, which was wrong and did not compile cleanly under clippy:

- `a605585` added `use std::str::FromStr;` and `.ok()` to calls like
  `WarningType::from_str(&t)`. Those are **inherent methods on the enum
  returning `Option<Self>`**, not the `std::str::FromStr` trait. So the import
  was unnecessary and `.ok()` did not exist on an `Option` — clippy reported
  `E0599` on six call sites across four files.

The working tree removes the needless imports and the `.ok()` calls, leaving
`ok_or_else` matching the `Option` it already expected. Clippy is clean.

```
crates/app/src/imports.rs                        |  6 +-----
crates/app/src/routes/moderation.rs              |  7 ++----
crates/app/src/routes/spoilers.rs                |  9 +++-----
crates/app/src/routes/thread_modes.rs            |  3 +--
crates/db/src/spoilers.rs                        | 15 ++++----------
5 files changed, 12 insertions(+), 28 deletions(-)
```

Commit it as a fixup to `a605585` before deploying:

```bash
git add crates/app/src/imports.rs crates/app/src/routes/{moderation,spoilers,thread_modes}.rs crates/db/src/spoilers.rs
git commit -m "fix: from_str on the spoiler enums is inherent, not FromStr"
```

**Do not run `cargo fmt --all` before committing.** It reformats 15 unrelated
files (`rec_strategy.rs`, `settings.rs`, `milestone_10.rs`, …) that are other
people's work in progress. Run `cargo fmt` on the specific files, or skip it —
clippy is the gate that matters here.

## 2. What is already done and verified in production

| Repair | Result |
|---|---|
| Summaries that were body dumps | 1,664 → **0** |
| Fandom tags | 0% → **74.7%** (24,784 of 33,167) |
| Character tags | 0% → **89%** (29,504) |
| Works with no tags at all | 33,167 → **908 (2.7%)** |
| Several posts merged into one chapter | 1,095 → **83** |

1,095 merged works were split into 3,024 chapters, character-exact (verified: all
5,106 characters of a sampled original preserved across the split). The
original merged chapters are **soft-deleted**, never deleted.

Every repair is idempotent — a re-run writes 0 rows. Verified this session:
`summary repair re-run: wrote 0 of 703 still-damaged`.

## 3. What is NOT done — the actual remaining work

### 3a. Deploy (the blocker)

**Production is running code from before all three commits.** The running
binary was built **Sep 25 11:52**; the commits are **Sep 26 08:51+**.

Consequence: the tag-persistence fix is not live. A fresh import parses tags
and discards them. The 33,167 works look tagged only because the Python
backfill wrote straight to the database.

It runs as a **systemd user service**, not a bare process:

```
unit:          lorehaven.service  (systemctl --user)
ExecStart:     /home/alvaro/.cargo-target/lorehaven/release/lorehaven serve --with-worker
WorkingDir:    /personal/documents/code/rust/lorehaven
Environment:   LOREHAVEN_ENV=development
               LOREHAVEN_CONFIG=/personal/documents/code/rust/lorehaven/lorehaven.toml
```

Deploy:

```bash
# on thinkcentre
cd /personal/documents/code/rust/lorehaven
git pull                      # or fetch the branch
CARGO_TARGET_DIR=/home/alvaro/.cargo-target/lorehaven cargo build --release
export XDG_RUNTIME_DIR=/run/user/$(id -u)
systemctl --user restart lorehaven
systemctl --user is-active lorehaven        # expect: active
curl -s -o /dev/null -w '%{http_code}\n' https://lorehaven.polarisocial.xyz/api/health   # expect: 200
```

**`CARGO_TARGET_DIR` must be `/home/alvaro/.cargo-target/lorehaven`.** That is
where `ExecStart` points. The repo also has a stale
`target/release/lorehaven` from Sep 24 that is *not* what runs — do not build
there and assume it deployed.

Note the working directory is `/personal/documents/code/rust/lorehaven` while
the local clone is at `/home/alvaro/code/rust/lorehaven` (a mount of the same
tree). Confirm which you are on before pulling.

### 3b. Merge

25 commits ahead of `master`, 0 behind. The branch is on Forgejo
(`origin`) but **not mirrored to `github`**. Note that `f959531` (the Pawchive
adapter fix) and the M47 work ride along in those 25, so merging is a larger
action than just the tag fix — check with Alvaro if that is intended.

The release branch is `master`, not `main`.

## 4. Known pre-existing failure — not yours, do not chase it

```
test result: FAILED. 26 passed; 1 failed
    repeated_login_attempts_are_rate_limited
```

Reproduced in a clean `git worktree` at `16ad679` **with none of this work's
commits present** — it fails identically there. Two tests in
`crates/app/tests/milestone_2.rs` both call `lorehaven_app::limiter::clear_buckets()`
on process-global state (lines 1308 and 1358). The test passes alone and fails
when its neighbour runs. Fixing it means serialising those two tests or giving
each its own limiter instance. Left alone deliberately.

## 5. The SmolLM2-135M decision — do not revisit

`Fu01978/SmolLM2-135M-Instruct-AO3` was evaluated and **rejected**. Re-verified
this session with a fresh run rather than trusting the earlier verdict:

- Zero-shot, greedy, no repetition penalty (most favourable settings): emits
  prose continuations, not tags. 86% / 75% / 47% duplicate tokens across three
  real posts.
- Few-shot greedy: right *shape* (comma-separated), but every tag copied
  verbatim from the demonstrations — `Elena Martinez, Mass Effect` for a post
  about pirates.
- Few-shot sampled: repetition drops to ~12%, and output is fluent, unrelated
  prose. Varied wrong answers are worse than a visible loop.

**No tag in the dataset came from this model.** Everything is deterministic:
source-supplied tags, title-convention parsing, or a lexicon built from
Lorehaven's own AO3 taxonomy.

Do not add it. There is no ONNX runtime, no `candle`, no `ort`, and no feature
flags in this repo. More importantly, a tagger writing into `taxonomy_nodes`
writes to *shared vocabulary* — a hallucinated node outlives the work it was
attached to and pollutes every fandom filter built on it. If AI tagging is
wanted later it should be a separate opt-in job writing to a staging table, and
should be an embedding or classifier model, not a 135M generative one.

## 6. Reference: the repair scripts

`/home/alvaro/code-local/research/pawchive/` on the local machine, and copied
to `/tmp/` on the ThinkCentre. Run against production Postgres with
`/tmp/pw-venv/bin/python`.

| Script | Purpose |
|---|---|
| `backfill_pawchive.py` | three passes: per-author fetch, stored-summary repair, taxonomy |
| `pawchive_meta.py` | `prose_summary`, `derived_summary`, title/lexicon parsing |
| `resplit_merged_posts.py` | split works holding several posts (on `-VB-`) |
| `resplit_containers.py` | split whole-archive containers (titles `Pawchive user <uid>`) |
| `audit_summaries.py`, `final_state.py` | measure the outcome — **read the checks before trusting the percentages** |

DB URL comes from `/personal/documents/code/rust/lorehaven/lorehaven.toml`
under `[database]`.

### Traps that cost hours here — do not re-introduce them

1. **A scalar parameter wrapped in a list.** `[WS_CLASS]` instead of
   `WS_CLASS` binds a one-element `text[]` where the statement expects `text`.
   Postgres matches nothing and reports rowcount 0, so the run *looks* clean.
   Instrument `db.exec` to print `[type(x).__name__ for x in params]` and look
   for the odd one out. `Db.exec` must return `cur.rowcount`; discarding it
   hides this entirely.
2. **A regex class built from a pattern string.** `WS_CLASS = r"[\t-\r]"` is 8
   literal characters, not a tab. `WS.pattern` is safe; a hand-written literal
   is not. Check with `chr(92) + "u" in WS_CLASS`.
3. **Python/SQL whitespace mismatch.** `str.split()` and Postgres `\s` disagree
   (Postgres keeps U+00A0, U+202F, U+0085). Use one `WS` regex on both sides
   and pass `WS.pattern` to SQL.
4. **Ordering matters for summaries.** Stored bodies have **no newlines** — the
   importer flattens them. `derived_summary` therefore returns the run-together
   header as "prose". Call `prose_summary` (which cuts at the `-VB-` marker)
   **first**; `derived_summary(a) or prose_summary(b)` picks the bad one
   because it is non-None.
5. **Every UPDATE needs `IS DISTINCT FROM`.** A derived first sentence is often
   still a prefix of the body, so it is re-derived identically each run; without
   the guard each run bumps `version` forever.
6. **`taxonomy_nodes.kind` is data, not decoration.** Writing everything as
   `tag` leaves the data present but invisible to every fandom and character
   filter — the UI showed untagged works while 32,259 rows existed. Key nodes
   on `(kind, norm)`; join `work_tags` on **both** columns, or a generic tag
   steals a fandom's slot. Vocabulary: `fandom`, `ship`, `character`, `tag`,
   `warning`, `mood`.
7. **Verify a count against its definition.** "Merged into one chapter" keyed
   on chapter *length* counted 7,870 defects where 83 were real — the other
   7,851 were single legitimately-long posts. A `node_kind = 'relationship'`
   filter matched nothing because the vocabulary says `ship`, reporting 100%
   of works missing a pairing.

## 7. Suggested order

1. Commit the working-tree fixup (§1).
2. `cargo test --workspace` — expect 359 passed, 1 known failure (§4).
3. Build + restart + health check (§3a).
4. Smoke-test a real import and confirm `work_tags` gets rows.
5. Merge to `master`; mirror to `github` (§3b).
6. `.hermes/` is untracked in the repo — decide whether it belongs in
   `.gitignore`.
