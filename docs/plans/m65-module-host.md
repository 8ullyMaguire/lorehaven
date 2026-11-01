# M65 — the module host: a small core, and every surface as a switchable module

**Milestone:** M65. **Status:** Phase 1 and 2 in this milestone; Phase 3 is the frontend, Phase 4 the third-party
loader (Milestone 15, unchanged).
**Spec:** §59 (`docs/spec.md`). **Closes:** the first-party half of §6.3's slot promise.
**Measured 2026-11-01** before any code was written.

## 0 — The incumbent, found before writing any code

`/api/v1/extensions` **already exists**: `marketplace.rs` serves a third-party extension catalogue
(`GET /extensions`, `/extensions/{slug}`, grant/revoke, `/me/extension-grants`) with
`extension_submissions` behind it (migration 0113). The first draft of this plan mounted a second
`GET /extensions` in `routes/extensions.rs` and the assembled router panicked with
`Overlapping method route. Handler for GET /extensions already exists` — the incumbent announcing
itself at test time, one merge away from a live 500.

That panic is why the whole surface is called **module** on the wire, in config and in the CLI:
an extension is a package you install, a module is a surface the build compiles. §59.1 argues it.

## 1 — The premise, measured

| fact | value | why it decides the design |
|---|---|---|
| route modules on disk | **82** | one registry row per module, not per handler |
| mounted in `server.rs` | **79** across **90** merge sites | the sweep is mechanical, and a script can do all of it |
| `config.rs` | **6,160 lines**, **49** top-level sections | a new section is the cheapest possible config surface; the file already carries 49 |
| marketplace tables (`packages`, `installations`, `manifests`) | **absent** | §6.3 promised slots, not tables; nothing to fill, nothing to duplicate |
| `docs/design/0004-module-marketplace-architecture.md` | **absent** | it is referenced by the spec's design index and does not exist — recorded, not fixed here |
| `module_submissions` | **exists**, owned by source adapters (0113) | the word "module" already means *importer manifest* in this tree; the new surface must not collide (ids are `ext.<area>.<name>` in prose, plain snake case on the wire) |

So this is not a greenfield design: §6.3 already fixed the contract ("the slot contracts do so
first-party features use the same mechanism third-party modules will") and no code ever
existed for it. The refactor fills that promise instead of inventing a fifth.

## 2 — What the core is

`meta`, `health`, `auth`, `settings`, `openapi`, plus the request plumbing and the host itself.
**Everything else is an module whose compiled default is on.** `works`, `search`, `library`
and `reading` are modules too: an instance that cannot turn off its own catalogue is not
configurable, and a surface that can be turned off is a surface whose absence gets tested.

## 3 — The registry row

```rust
pub struct Extension {
    pub id: &'static str,
    pub name: &'static str,
    pub summary: &'static str,
    pub default_on: bool,
    pub stability: Stability,   // Stable | Beta | Experimental | Planned
    pub toggle: Toggle,         // Fixed | Operator | PerUser
    pub requires: &'static [&'static str],
    pub milestone: &'static str,
    pub spec: &'static str,
    pub nav: &'static [NavEntry],
}
```

Routes are **not** in the row: they are mounted through `ext::mount(...)` at the call site, so the
registry cannot drift from `server.rs` without the door test failing. `Stability::Planned` rows
have no routes at all, by construction.

## 4 — Resolution

```text
compiled default → darkhold.toml [modules] → account override → resolved
```

- Instance layer: `[modules] disabled = ["quiz"]`, `enabled = [...]`, `allow_user_overrides = true`.
  Load-time validation refuses an unknown id (naming the nearest), a `fixed` id in either list, an
  id in both lists (disabled wins, and the load says so), and a `requires` cycle.
- A user override narrows and never widens: `resolved = instance_on && !(user_off)`.
- `fixed` rows refuse both layers at load time with §59.4 quoted.

## 5 — The refusal

`ext::mount` wraps the module's router in a middleware that answers
`404 {"error":{"code":"module_disabled","module":"<id>", …}}` naming the config line that
would enable it. 404 because the surface is absent, not refused. The body is the operator's index
into the config file, which is what keeps a 404 from being a mystery.

## 6 — The sweep: 74 modules, mapped

Generated from `server.rs` by parsing the 90 merge sites; a module missing from this table fails
`m65_module_host::every_route_module_is_registered`.

| id | mounted fns | classes | toggle |
|---|---|---|---|
| `account_permissions` | router | Write | fixed |
| `admin` | router | Default | fixed |
| `admin_discovery` | router | Default | operator |
| `analytics` | router | Default | operator |
| `arena` | router | — | per_user |
| `audience` | router | Default | operator |
| `author_media` | router | Default | operator |
| `browse` | router | Default | operator |
| `catalogue` | router | Search | operator |
| `collaborators` | router | Write | operator |
| `community` | router | Write | operator |
| `concierge` | router | Default | per_user |
| `continue_reading` | router | Default | operator |
| `cta` | router | Write | operator |
| `curator` | router | Default | operator |
| `dashboard` | router | Default | operator |
| `decision_service` | router | Default | operator |
| `derivative` | router | Write | operator |
| `directory` | router | Default | operator |
| `discovery` | router | Default | operator |
| `dnf` | router | Search | operator |
| `draw` | router | Default | per_user |
| `economy` | router | Default | operator |
| `erasure` | router | Write | fixed |
| `events` | router | Write | operator |
| `exchange` | router | Write | operator |
| `exports` | authed_router, router | Default, Export | operator |
| `external` | router | Default | operator |
| `federation` |  | — | operator |
| `feedback` | router | Write | operator |
| `flows` | router | Default | operator |
| `governance` | router | Write | operator |
| `imports` | admin_router, router | Write | fixed |
| `jobs` | admin_router, router | Write | fixed |
| `lending` | router | Write | operator |
| `library` | router | Write | operator |
| `marketplace` | router | Default | operator |
| `media` | router, write_router | Default, Write | operator |
| `media_health` | router | Default | operator |
| `media_resilience` | router | Default | operator |
| `mirror_admin` | router | Default | operator |
| `moderation` | router | Write | fixed |
| `monetization` | gifts_router, read_router, router, transparency_router | Default, Write | operator |
| `narration` | router | Write | per_user |
| `north_star` | router | Default | operator |
| `notifications` | router | Write | operator |
| `preread` | router | Write | operator |
| `pseuds` | router | Write | operator |
| `quiz` | router | Default | per_user |
| `rating_integrity` | router | Default | operator |
| `reader_body_copies` | router | Write | fixed |
| `reader_surface` | router | Default | operator |
| `reading` | authed_router, router | Write | operator |
| `rec_run` | router | Search | operator |
| `recommendation_transparency` | router | Default | operator |
| `retention` | router | Write | operator |
| `retention_proposal_admin` | operator_router | Write | operator |
| `retention_proposals` | router | Write | operator |
| `retention_settle` | router | Write | fixed |
| `roadmap` | admin_router, read_router | Default, Write | operator |
| `search` | router | Default | operator |
| `source_adapters` | router | Default | operator |
| `spoilers` | router | Write | per_user |
| `subscriptions` | ai_training_router, router | Write | operator |
| `supply` | router | Write | operator |
| `tasting` | router | Write | per_user |
| `taxonomy` | router | Write | operator |
| `taxonomy_export` | router | Default | operator |
| `thread_modes` | router | Write | operator |
| `translation` | router | Default | operator |
| `typed_votes` | router | Write | operator |
| `vanguard` | router | Default | operator |
| `work_discussion` | router | Write | operator |
| `works` | read_router, router | Default, Write | operator |

Core (not gated): `meta`, `health`, `auth`, `settings`, `openapi` — 5 modules.

## 7 — Phases

### Phase 1 — the host (this milestone)

1. `crates/app/src/ext/mod.rs` — `Extension`, `Stability`, `Toggle`, `NavEntry`, `REGISTRY`,
   `find`, `instance_state()`.
2. `crates/app/src/ext/registry.rs` — the 74 rows above + planned rows.
3. `crates/app/src/ext/config.rs` — `ExtensionsConfig { disabled, enabled, allow_user_overrides }`
   with `validate(&self) -> Result<(), String>`; wired as `Config::modules`.
4. `crates/app/src/ext/gate.rs` — `mount(state, id, router, class)`; `AppState::modules`.
5. `crates/app/src/server.rs` — every `.merge(classified(routes::x::f(), RouteClass::C, &state))`
   becomes `.merge(ext::mount(&state, "x", routes::x::f(), RouteClass::C))`.
6. `crates/app/src/routes/modules.rs` — `GET /api/v1/moduless`.
7. `crates/cli/src/ext.rs` — `dh ext list [--json]`, `dh ext show <id>`.

**Gate:** `cargo test -p darkhold-app --test m65_module_host` green (7 tests: disabled 404 names the
id, the same route is not 404 when enabled, fixed cannot be disabled, unknown id refused with a
suggestion, enabled-by-default is all-on, planned refuses enable, every module registered);
`cargo test -p darkhold-app --lib` green; live: `curl /api/v1/quiz/...` → 404 `module_disabled`
with `quiz` in `disabled`, then 401 with it removed.

### Phase 2 — per-user overrides

1. `migrations/{sqlite,postgres}/0134_module_user_state.sql` —
   `(account_id, module_id, enabled, updated_at)`, PK `(account_id, module_id)`, FK to `accounts`
   `ON DELETE CASCADE` (§51: this is personal data and the cascade reaches it).
2. `crates/db/src/modules.rs` — `set_override`, `read_overrides`, `off_accounts(id)`.
3. `crates/app/src/ext/gate.rs` — when the module is `per_user` **and** any account holds an
   override for it, resolve the session and refuse for that account. Zero cost when nobody has
   toggled anything, which is the common case.
4. `crates/app/src/routes/modules.rs` — `PATCH /api/v1/moduless/{id}` with `{"enabled":bool}`.
5. `docs/snapshot-column-policy.json` — the table's `account_id` needs a decision (`drop_table`).

**Gate:** `cargo test -p darkhold-db --test module_state` 4 tests both engines; a route test
where `alice` toggles `quiz` off and gets 404 while `bob` gets 401; `PATCH` on a `fixed` row → 403
naming §59.4.

### Phase 3 — planned catalogue and the operator's verbs (SHIPPED as M65-05)

`dh ext list --planned` prints the roadmap's unbuilt surfaces from the registry; `dh ext disable`
refuses a planned row with its milestone. Rows come from `docs/requirements.csv` grouped by area,
one row per *surface the reader would see*, not per ticket.

### Phase 4 — the frontend consumes it

`/api/v1/moduless` is what the nav hides its links with; §6.3's slots are declared in `nav` and
rendered from the same payload. Not this milestone.

## 8 — What this does not do

- No `dlopen`, no ABI, no untrusted code. §59.1 argues why.
- No module can be enabled that has no compiled routes; a `planned` row refuses with its
  milestone rather than pretending a flag is a build.
- No per-pseud toggles (§59.6: one row per account, because a matrix of toggles is a settings page
  nobody finishes).

---

## What Phase 3 landed, and the two rules it cost

`scripts/ideas_catalogue.py` reads eight sources and writes `crates/domain/src/modules/ideas.rs`
(191 rows) plus `docs/ideas-catalogue.json`; `the_idea_catalogue_matches_its_json` is the drift
gate. Relevance (§59.10) orders the queue and, for built surfaces, decides the default: six are
opt-in by design and off (`arena`, `quiz`, `tasting`, `concierge`, `narration`, `vanguard`), so a
fresh instance is 68 on / 29 off, and `[modules] enabled` turns one back on — the m65 test drives
that path, which is the instance layer proven rather than described.

The first generator run produced **375 rows that were mostly furniture**: a spec amendment's
internal headings ("4 Acceptance", "Config", "Storage") became ideas, and the relevance score
sorted "5 Acceptance" above the brainstorm it came from. Two rules fixed it, and both are the kind
of thing that only shows up when you look at the output instead of the count:

- **A spec amendment or a design is one idea** — its title. Its headings are its sections.
- **A brainstorm is a list**, so its `###` headings are the ideas and its `##` tiers are not.

Concord's darkhold entry was checked: it is a forge seed (`slug`, `repo_url`) and contributes no
ideas. Saying so is the honest result; inventing a feature from it would have added a row to a
catalogue whose whole value is that every row can be traced.
