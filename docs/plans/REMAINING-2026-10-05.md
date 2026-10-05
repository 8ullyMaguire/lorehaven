# Lorehaven — what's left, 2026-10-05 (measured against a running instance)

Base: **`312b41e`**, working tree clean. Everything below was either re-derived from this
tree today or explicitly attributed to the file it came from. Nothing is carried forward
on the strength of a plan's own claim.

Two complaints opened this pass — *the header is crowded* and *some pages say "That did
not work" when I open them* — and both turned out to be measurable. Driving a browser
over the instance found four more defects hiding behind the second one.

## Where this file sits, so the chain does not grow again

| file | owns |
|---|---|
| `docs/requirements.csv` | the canonical inventory — 703 rows: 672 implemented, 26 `planned`, 4 `unsupported`, 1 `evaluated-and-rejected` |
| `docs/plans/100-ideas-remaining.md` | the ideas queue — Tier 1 remainder, the 26 planned rows, the ~72 unassessed |
| `docs/plans/TRACKER-2026-10-05.md` | what the Rust gates measured, per commit |
| **this file** | what is left *as observed on a running instance today*, and pointers to the three above |

Four plan files have already superseded each other in a chain, and
`100-ideas-remaining.md` §1 says the chain itself was the defect. So this file does
**not** restate the queue and does not supersede anything: it carries the defects found
today and defers every feature question to the CSV.

## How it was measured — the harness, so it can be repeated

```bash
# release binary + a scratch SQLite instance with the embedded frontend
cd ~/code-local/rust/lorehaven/frontend
BIN=$HOME/.cargo-target/lorehaven/release/lorehaven
SCRATCH=$(pwd)/test-results/probe
$BIN --storage-root "$SCRATCH/storage" \
     --database-url "sqlite://$SCRATCH/lorehaven.db?mode=rwc" \
     --port 8174 --config "$(pwd)/e2e/scratch-config.toml" serve --with-worker &
# then, twice — once anonymous, once after POST /api/v1/auth/register —
# Playwright visited 35 routes and recorded every response >= 400, every console
# error, and whether an <h3>That did not work</h3> was on screen.
```

Result: **35 routes × 2 states = 70 page loads**. 15 of them showed the red error panel.
The scan scripts are in `/tmp/lhprobe/` (this session, not committed); the findings
below are what survives re-checking each one against the source at `312b41e`.

**Provenance caveat, stated rather than hidden.** The release binary under test is
stamped `0.1.0+1f5ce1a.dirty`, 13 commits behind HEAD (it was built at 01:17 from a
working tree whose changes were committed later as `5759616`). That matters only if a
route file changed underneath: `arena.rs`, `vanguard.rs`, `media_health.rs`,
`community.rs` and `author_media.rs` are **byte-identical** between `1f5ce1a` and HEAD
(`git diff --name-only 1f5ce1a..HEAD -- <those five>` is empty). Every runtime finding
below therefore holds at `312b41e`, and every frontend finding was read from the clean
working tree directly.

## 1. The header — crowded, and the guard test structurally cannot see it

Measured with Chromium at 1280 / 1440 / 1680 CSS px, signed in, on `/`. The bar is a
width-capped container: **1088px content box at every viewport**, so a wider window does
not help.

| element | width | note |
|---|---|---|
| brand | 143px | |
| `.desktop` nav | **310px available, 544px of content** | over by **234px — 43% of the row is behind the scrollbar** |
| `.controls` | **543px** | `Writing as @handle` 154 + `Settings` 57 + `Account` 57 + `Sign out` 83 + appearance `<select>` 143 |

The controls are half the bar. The nav has less room than the controls do, and it is the
nav that scrolls.

Anonymous, for contrast: the row **fits exactly** at ≥1280 (581/581) and overflows at
1024 (544/517). Header height 65px, one row, never wrapped, at every width tested — the
fixed-height guarantee in `App.svelte:590-617` still holds.

**Why this was never caught:** `frontend/e2e/shell.spec.ts:39` ("every destination is
reachable without scrolling the row") does `page.goto('/')` with no sign-in. It measures
the row that fits and never the row that does not. The test is correct about the state it
covers; the state it covers is the one nobody spends time in.

Constraints any fix must respect, all of them load-bearing:

- `shell.spec.ts:91` asserts **exactly 9** nav children; `:61` asserts one row;
  `:97` asserts opening a menu does not change header height (`--header-height` and
  `scroll-padding-top` depend on it).
- `App.svelte:104-111`: the group label is `Create`, **not** `Publish` — renaming it
  fails seven Playwright tests with `Republish not found` under strict mode.
- `.desktop` must not go back to `flex-wrap: wrap` (`App.svelte:590`) and the scrollbar
  must stay visible if the row ever scrolls again (`App.svelte:631`).
- Everything in `.controls` already has a drawer home at ≤51.99rem
  (`App.svelte:763-782`, drawer has `theme-select-mobile` and its own sign-out).

Direction that fits the arithmetic — **the nav is not the problem, the controls are**:

1. Collapse `Settings` + `Account` + `Sign out` into one account trigger. Gross **197px**
   (57 + 57 + 83), less whatever the trigger itself costs.
2. Drop the appearance `<select>` from the header (143px, net) — the drawer already carries
   the same control as `theme-select-mobile` (`App.svelte:515-521`), so nothing becomes
   unreachable. Neither `Settings.svelte` nor `Account.svelte` has a theme control, so the
   header and the drawer are the only two homes it has.
3. `Writing as @handle` (154px) stays only if the arithmetic still closes; otherwise it
   moves to the account trigger's label, which is what it is really labelling.

Target: controls ≤ 300px, so 143 brand + 544 nav + gaps + 300 controls ≈ 1088.
**Done when** `shell.spec.ts`'s fit assertion passes **signed in as well as signed out**,
at 1024 and 1440 — the signed-in case is the one to add, and it is currently absent.

## 2. "That did not work" — which pages, and why

The panel is `frontend/src/lib/components/ErrorSummary.svelte:25`. It is the right
component for a failed *action*; it is the wrong thing to open a page with. Both halves
of the complaint are below.

### 2a. Anonymous: nine pages open with a red error instead of an invitation

| route | first API call | code |
|---|---|---|
| `/blind-date` | `401 /api/v1/discovery/blind-date` | `AUTH_REQUIRED` |
| `/surprise-me` | `401 /api/v1/discovery/surprise-me` | `AUTH_REQUIRED` |
| `/arena` | `401 /api/v1/arena/next` | `AUTH_REQUIRED` |
| `/community` | `401 /api/v1/forums` | `AUTH_REQUIRED` |
| `/notifications` | `401 /api/v1/notifications` | `AUTH_REQUIRED` |
| `/concierge` | `401 /api/v1/me/concierge` | `AUTH_REQUIRED` |
| `/quiz` | `401 /api/v1/quiz/works` | `AUTH_REQUIRED` |
| `/vanguard` | `401 /api/v1/vanguard/status`, `/vanguards`, `/me/streak` | `AUTH_REQUIRED` |
| `/admin/economy/flows` | `401 /api/v1/admin/economy/flows` | `AUTH_REQUIRED` |

The reader is told "That did not work / authentication required" about a page that never
asked them to do anything. **Two working patterns already exist in the tree**, and neither
of them is a guess:

- **Guard before fetching.** `Library.svelte:407`, `History.svelte:75`,
  `Import.svelte:242`, `Exports.svelte:232` and `Jobs.svelte:102` all render
  `{#if !session.isSignedIn}` → a sign-in note, and only then touch the API.
  `ErrorSummary` stays reserved for real failures (`Library.svelte:528`,
  `Jobs.svelte:116`). Those five pages are therefore absent from the table above.
- **Else read the status.** `AnalyticsDashboard.svelte:40-49` catches `401 || 403` and
  sets a `signedOut` flag, with a comment that states the rule this pass rediscovered:
  *"A 401 on this door is not an error to report, it is the answer to 'is this page for
  me?' — and rendering it as a failure string sends a signed-out visitor to a message they
  can do nothing about."*

**Eight of the nine offending files contain no reference to `session` at all**
(`grep -c session` = 0 for `BlindDate`, `SurpriseMe`, `Arena`, `Notifications`,
`Concierge`, `Quiz`, `Vanguard`, `AdminEconomyFlows`; `Community` has 3 and still
renders the panel). Prefer pattern 1; pattern 2 is the fallback for the page where
fetching on mount is unavoidable, because it also catches the race where a page mounts
before `session.refresh()` has answered.

Decision to make first, not after: **is `/community` meant to be readable by a visitor?**
`get_forums` is `RequireSession` (`community.rs:268-271`), so today the answer is no by
construction — but a forum behind a login wall is a product call worth making
deliberately, because it is the one page in this table a stranger has any reason to want.

`/admin/economy/flows` is the odd one: it is **not linked from anywhere** (0 `href`
matches in `frontend/src`), so only a typed URL reaches it — and it still answers with a
red error panel rather than 404. The drawer already refuses to link the operator queue
for exactly this reason (`App.svelte:501-505`); the route should answer the same way.

### 2b. Signed in: four pages, four different root causes

| route | response | panel says | root cause |
|---|---|---|---|
| `/arena` | **500** `GET /api/v1/arena/next` | `INTERNAL` | §3.1 |
| `/vanguard` | **403** `GET /api/v1/vanguards` | `ACCESS_DENIED` | §3.2 |
| `/admin/media-health` | **403 ×6** (`overview`, `link-rot`, `curator-leaderboard`, `bounty-status`, `storage`, `providers`) | `ACCESS_DENIED` | §3.3 |
| `/author/media` | **404** `GET /api/v1/author/media-health` | `INTERNAL` — "Lorehaven returned an unexpected 404 response." | §3.4 |

## 3. The four defects behind 2b, each with its cause

### 3.1 `/arena` answers **500** to every new account

`arena.rs:136-141` maps `generate_arena_round(...).ok_or_else(...)` to
`AppError::Internal("not enough works for arena round")`, and the log confirms the shape:

```
ERROR request{path=/api/v1/arena/next}: lorehaven_app::http: request failed
      error=Internal(not enough works for arena round)  → status=500
```

**It is not an empty-instance artefact.** Reproduced twice: on a blank scratch database,
and again after `seed --development` loaded the full fixture set — a freshly registered
account still gets 500 from `Read → Arena`. An empty or too-small round is an ordinary
state for a reader who has rated nothing, not a server fault.

Fix shape, per the standing rule in `100-ideas-remaining.md` §3 (*"`{"works": []}`,
never null. An empty rail renders nothing"*): a typed empty answer — `200` with no round,
or a non-`INTERNAL` code the page renders as "not enough of your ratings yet" — never
`Internal`.

**Why nothing caught it:** `grep -rn 'arena/next' crates --include='*.rs'` matches only
the route definition and the `route_inventory` table — `crates/app/tests/arena.rs` (4
tests) never issues the HTTP call — and `grep -rn arena frontend/e2e/*.spec.ts` is empty:
**no Playwright spec visits `/arena` either.** The endpoint has no HTTP-level test at
all. **Done when** a fresh account on a seeded instance gets a page that explains itself,
and `GET /api/v1/arena/next` with an empty pool is asserted directly.

### 3.2 `/vanguard` is poisoned by one operator-only call inside a `Promise.all`

`Vanguard.svelte:36-43` awaits four requests together, one of which is `listVanguards()`
→ `GET /api/v1/vanguards`. That handler is `require_operator`
(`vanguard.rs:136-145`), so **every account that is not the operator** gets 403, and
because the four share one `error` slot the whole page renders the red panel — status,
pins and streak are discarded with it. `Promise.all` was the wrong combinator for four
independent panels; `Promise.allSettled`, or an error per panel, keeps an operator-only
list from taking the page down. **Done when** `/vanguard` renders for a normal account
with the "Current vanguards" section simply absent.

### 3.3 `/admin/media-health` answers **403**, and shows the panel to non-operators

`media_health.rs:26-39` returns `AppError::AccessDenied` (403) to anyone who is not the
operator. The house rule for operator surfaces is written down in
`REMAINING-2026-10-03.md` (north-star / flows): **404, not 403**, because for an
operator view the *existence* is the disclosure — 403 answers "yes, and you may not".
Two other operator modules already do this (`admin_discovery.rs:40-51`,
`decision_service.rs:153-164`), so this one is the outlier. Second half: the page still
renders `ErrorSummary` to a non-operator, where it should answer like every other
forbidden surface. **Done when** `GET /admin/media-health/overview` returns 404 for a
non-operator and that is asserted the way flows' 404 is.

### 3.4 Five author-media endpoints have been **unmounted since M47**

`routes::author_media::router()` is declared, tabulated, unit-tested — and **never merged
into the API router**:

```bash
for f in crates/app/src/routes/*.rs; do
  n=$(basename "$f" .rs)
  for fn in $(grep -oP 'pub fn \K\w+' "$f" | sort -u); do
    case "$fn" in
      *router*)
        if [ "$(grep -c "routes::$n::$fn" crates/app/src/server.rs)" -eq 0 ]; then
          echo "NOT-IN-server.rs: $n::$fn"
        fi ;;
    esac
  done
done
# → NOT-IN-server.rs: author_media::router     # the only miss in the tree
```

The history names the cause. `d5af6e0` (M46) added the merge; `1439bac` (M47, advanced
mirroring) **replaced** it rather than adding a second `.merge(...)` beside it:

```diff
         .merge(classified(
-            routes::author_media::router(),
+            routes::mirror_admin::router(),
             RouteClass::Default,
             &state,
         ))
```

Since that commit the five routes — `GET/PUT /author/media-preferences`,
`GET /author/media-health`, `POST /author/targeted-bounties`,
`POST /author/targeted-bounties/claim`, `GET /works/{work_id}/targeted-bounties` —
answer 404, and `/author/media` reports `INTERNAL` because the client turns an
unexpected 404 into "Lorehaven returned an unexpected 404 response".

`server.rs` is the only place routers are composed in this tree
(`grep -rn '::router()' crates/app/src --include='*.rs' | grep -v server.rs` is empty),
which is what makes the check below sufficient.

**Both guards point the wrong way**, which is why it survived:

- `crates/app/tests/route_inventory.rs:17` asserts *every registered route in
  `server.rs` has a table entry* — server → table. The reverse (every route source has a
  registration) is unchecked, and that is exactly the direction that failed.
- `crates/app/tests/author_media.rs:196` calls
  `media_resilience::author_media_health_report` — the **store**, not the HTTP route —
  and no E2E issues the request either.

Fix: restore the `.merge(...)`, then close the gate in the other direction — every
`pub fn …router()` in `crates/app/src/routes/*.rs` must appear at least once in
`server.rs` (the loop above found exactly one defect and needs no new dependency).
**Done when** `GET /api/v1/author/media-health` with a session returns 200 on a seeded
instance and the reverse-direction check is in CI.

## 4. Not defects — measured, explained, do not re-open

| observation | why it is correct |
|---|---|
| `GET /api/v1/continue-reading` → 404 on `/` for a signed-in reader | **404 means "nothing to continue"** by design — `continue_reading.rs:15-21`, and the client keys on `is_not_found`. The banner simply does not render. |
| `/works/<unknown>` shows the panel | a missing work is a real error, and it is labelled `NOT_FOUND` |
| `/admin/jobs` and `/admin/economy/flows` → 404 for a non-operator | the deliberate operator-404 rule (§3.3); the page staying quiet is the wanted behaviour |
| `401 /api/v1/auth/me` on every anonymous load | the session probe |
| the browser console echoing `Failed to load resource: 401` | the same 401, logged by Chromium, not by the application |

## 5. Gates, re-measured today (2026-10-05)

| gate | result |
|---|---|
| `vitest run` | **543/543, 75 files**, 12.4 s |
| `svelte-check` | **0 errors, 2 warnings** — `NavMenu.svelte:166` (a11y: listeners on a non-interactive `div`), `ForumSearch.svelte:161` (unused CSS `h2`) |
| `cargo fmt --all --check` | **clean** |
| Playwright E2E | **not re-run today**; last recorded figure is 126/126 from the earlier pass |
| Rust workspace, SQLite | last measured `651cc9b`: **4160 passed / 0 failed** |
| Rust workspace, PostgreSQL | `651cc9b`: **4126 passed / 3 failed**, and those 3 are the harness races fixed *by* `651cc9b` — **still open in `TRACKER-2026-10-05.md`: "in flight — the one gate that closes the session"** |

`git diff --stat 651cc9b..HEAD` is a single tracker file, so the Rust numbers carry to
`312b41e`; what this project still does not have is the PG run *at* that commit. Before
it: `scripts/pg-test-db.sh`, then export the URL it prints (4 GB shm is fixed at container
create time — `docker start` cannot repair it).

## 6. Carried forward, unchanged, with its source

From `100-ideas-remaining.md` — the ideas queue, not repeated here:

- **Tier 1 remainder, 5 open**: items **31** (private search history), **41**
  (reading-goal progress ring — §9.6 has the goal, nothing renders it), **43**
  (fandom-specific reaction labels), **75** ("New author" debut badge), **22-preset**
  (a named preset button over the existing `max_words`/`completion` search filters).
  All render work over columns that already exist.
- **M45-39** is next among the 26 `planned` rows — the first of the two
  external-dependency rows (ActivityPub). None of the 26 has a `docs/spec.md` section, so
  each needs spec + plan before code.
- **~72 ideas unassessed**, through the same measure-then-schedule loop.
- Refusals that stay refusals: item 2 (the AO3 archive scrape) and P2P beyond LAN.

From `TRACKER-2026-10-05.md`: the PG workspace run (§5), and the migration parser's
**9 column-set mismatches** against the live schema — lossy for `ALTER TABLE ADD COLUMN`
in later migrations, fallback path only, live path authoritative.

From `requirements.csv`: **4 `unsupported` rows** — `M2-06` block and mute primitives,
`M6-10` approved preservation batches, `M7-03` device delivery, `M6-15` instance body
retention.

## 7. Doc hygiene, two items

1. **`100-ideas-remaining.md` §2 row 6 still lists item 7 (Surprise Me) as open**, while
   its own §1 table says "shipped AND wired". The tree agrees with §1:
   `SurpriseMe.svelte:50` calls `fetchSurpriseMe()`, the route exists in `router.ts`, and
   `frontend/e2e/surprise-me.spec.ts` exists. Strike the row.
2. The §2 numbering jumps 3 → 5 (row 4 was removed when it closed). Harmless, but it is
   the kind of gap a reader assumes is a lost row.

## 8. The order of work

| # | work | done when |
|---|---|---|
| 1 | **Restore `author_media::router()` (§3.4) + the reverse-direction route gate** | five endpoints answer a session; CI fails if any `routes/*.rs` router is unmerged |
| 2 | **`/arena` empty round (§3.1)** | fresh account on a seeded instance sees an explanation, not a 500; the `None` branch is tested |
| 3 | **Sign-in guard on the nine anonymous pages (§2a)** | no route shows `That did not work` to a signed-out visitor; each of the eight files either guards on `session` or reads the 401 the way `AnalyticsDashboard` does; `/community`'s public-vs-login decision recorded |
| 4 | **`/vanguard` `Promise.all` (§3.2)** | the page renders for a non-operator, section absent |
| 5 | **`/admin/media-health` 403 → 404 (§3.3)** | matches the flows/north-star rule and is asserted like them |
| 6 | **Header re-layout + signed-in guard test (§1)** | `shell.spec.ts` fit assertion passes signed **in** at 1024 and 1440; header still one row, still 65px |
| 7 | **PG workspace run at HEAD (§5)** | the tracker's last open gate closes with a number |
| 8 | Playwright E2E re-run against HEAD | 126/126 (or the failures named) |
| 9 | Ideas queue — Tier 1 remainder, then M45-39 | per `100-ideas-remaining.md` §2 |

1–5 are defects, are each independently revertible, and each has a reproduction in this
file. 6 is the complaint that opened the pass and is deliberately last among the UI items
because its guard test has to be written before the layout is touched — a fix with no
test is the thing that let `1439bac` ship.

**Re-running the scan after each fix**: the harness in "How it was measured" takes about
four minutes and reports per-route status codes, console errors and whether the panel was
on screen. The 15 panel hits today are the budget; the target is 1 (`/works/<unknown>`).
