# Remaining work to the from-scratch target spec (ADR 0024)

Status: **the live plan for closing the gap between this repository and the
consolidated from-scratch specification.** Produced 2026-09-24 by auditing
`docs/requirements.csv`, `docs/verification.md`, the git log and the test
suite against the from-scratch spec. Supersedes nothing —
`junior-implementation-plan.md` §0 remains the historical map; this file is
the forward plan. Every new milestone below gets rows in
`docs/requirements.csv` before code starts (ADR 0023: the CSV is the
canonical feature inventory and the roadmap-board seed).

## Where the build actually stands (2026-09-24, commit 97b24b0)

The README's "M0–M5 complete, M6 partly built" note is stale. Verified state:
**177 of 230 original requirement rows implemented** (172 locally-tested, 5
fully-tested), **49 planned** (M45 taste-arena residuals + M47 settings
surfaces), **4 deliberately unsupported**, 51 milestone test files through M45,
migrations to 0071, 37 frontend routes, tags through `v0.51.0`. The
platform core (M0–M15), the forum series (M31–M35), media resilience
(M48–M51), roadmap consensus (M45), browse ordering (M43), export CTAs
(M42), user settings (M47 complete, 9/9 requirements implemented-fully-tested) are built. What remains to the
from-scratch spec is: the recommendation strategy registry, most source
adapters, the bot, published OpenAPI, and the M45/M47 planned rows.

## Milestones

### M52 — Recommendation strategy registry (spec §16.1a, new)

Port the FicNexus rec-platform design onto `discovery`.

- `RecStrategy` trait + registry + `ScoredRec`; RRF blend (k=60) over
  enabled strategies; failing/empty strategy skipped.
- `rec.mode = legacy | pluggable` config (default legacy).
- Golden legacy-parity test: registry path in legacy mode reproduces
  `discovery::blend` exactly on a frozen fixture set.
- Strategies: cooccur, time-decay, tag graph, author graph, sequential,
  completion weighted, curator prior, bandit, external sidecar.
  Embeddings/MF feature-gated on AI provider presence.
- Recipes (§16.3) compose over strategies; taste gravity / diversity /
  ordering contract apply after the blend, in both modes.
- Rows: M52-01…M52-08. Depends on: nothing open. ~1 week.
  M52-01…07 are built (8 strategies, RRF blend, recipe composition, both
  modes wired). M52-08 (shadow-mode evaluation) is open.

### M52-09 — Per-user recommendation engine preference (spec §16.1b, new)

A reader may choose a recommendation engine, and that choice is remembered and
honoured everywhere a recommendation is produced. The instance default remains
the fallback; the user's choice overrides it, and an operator may pin an engine
that overrides both.

- One preference per **pseud**, not per account: a reader who wears two faces
  may want different recommendations from each, and a choice attached to the
  account would silently apply to both.
- Persisted in the per-pseud `search_settings` store under the
  `discovery.rec_engine` key, not as a column: the setting is one of several
  per-pseud preferences, and a column would have to be re-migrated for each.
  (The plan's earlier mention of a generic `user_settings` table was wrong —
  migration 0070 explicitly rejects a JSONB mega-table in favour of per-domain
  settings, and `discovery.*` keys already live in `search_settings`.)
- Validated against the enabled strategy set at write time. An unavailable
  engine is refused with the accepted values named, never a silent fallback —
  the same reasoning as §0.4.7's instance mode. The code is `422`,
  `AppError::Validation`'s existing status here, not a bespoke one.
- Every surface that produces a recommendation reads the preference through
  one resolver: discovery, feeds, search-as-you-type, the arena, and the
  author's own "more like this".
- Empties are represented as "instance default", not as a null engine.

**Note on an earlier attempt:** commits `72ee3e3`–`827e25a` drafted this but
removed `rec_enabled_strategies` from `DiscoveryConfig` while
`rec_engine.rs` still read it, which did not compile. That draft was reverted
in `d86d081`'s push; the spec intent survives here and the implementation
starts from a green tree.

### M53 — Adapter porting, batch 1 (spec §11.16, new)

Port the top-demand sources from `fanfic-scrapers` onto the safe-fetcher
`SiteAdapter` trait, fixture-gated per §11.16: ffnet, ao3, royalroad,
fictionpress, ficbook, syosetu — parse logic only, fetch path rewritten.
Batch 1 target: 10 adapters verified (11 exist today counting the existing
ones; the count is an outcome, not a claim). Login/adult-gated adapters
behind the credential vault only. ~1–2 weeks; further batches (M56+)
backgroundable, one batch per plan row.

### M54 — Companion bot port (spec §23.2, amended)

Port `fanfic-archivist-bot`'s platform-neutral core against `/api/v1`:
Discord, Telegram, Matrix first. Link flow per §23.2 (Lorehaven-side scope
confirmation, revocable tokens, no passwords). Companion binary in this
workspace sharing an API-client crate; the bot never touches the DB.
~1 week.

### M55 — Public API publication (spec §23.1)

OpenAPI generated from the router, published docs page, scoped-token
issuance UI, deprecation policy. The API itself exists (M18 built the
surface); this milestone is the *publication contract*. ~3–4 days.

### M56 — Instance posture and the remaining planned rows

M56-01 — **instance accessibility mode** (`public` | `walled_garden` |
`private`, spec §0.4.7) is **implemented and fully tested**. The mode is
translated once into an `AccessPolicy` rather than checked per handler, so
every content surface honours it by construction. An unrecognised value stops
startup.

The rest of M56 is the 49 planned rows: taste-arena residuals (M45) and the
settings surfaces (M47-02/05/07). These predate ADR 0024 and stay first-class.
~1 week.

### M45 gap — category governance voting is unreachable

Found while clearing pre-existing `svelte-check` errors: the proposal *voting*
half of §45 was never built, so the two halves do not meet.

What exists: stewards can create a category proposal
(`POST /directory/categories/governance/proposals`), and the database layer
has `list_open_proposals`, `vote_on_proposal`, `veto_proposal`,
`list_entry_mod_proposals` and `vote_entry_mod`. Steward-gated vote and veto
handlers exist in `DirectoryGovernance.svelte`.

What is missing: there is no `GET .../proposals` route, and the component never
renders a proposal list. So a steward creates a proposal and then has no way to
see, vote on, or veto it — the API can be driven by hand but the UI cannot.

The dead handlers were removed rather than left to rot. Restoring this means:

1. `GET /directory/categories/governance/proposals` (steward-gated) backed by
   the existing `list_open_proposals`, and an equivalent for entry-moderation
   proposals.
2. A proposal list in `DirectoryGovernance.svelte` with yes/no controls, a
   quorum indicator (`yes_votes` / `quorum_needed`), and the operator-only veto
   box.
3. The same for entry moderation: `handleProposeEntryMod` is wired to a button,
   so an entry action can be proposed and then never voted on.
4. Tests: the endpoint, and that a vote moves the tally and a veto is
   operator-only.

~1 day. Small, but §45 is not usable without it.

### Then — hardening and release

E2E green (73/73 as of `d04a54a`), deploy to thinkcentre production (build
there, never through SSHFS). A `v1.0.0` tag is deliberately **not** cut: the
release decision is the operator's, and the remaining M45/M47/M53/M54 rows are
open.

## Order and rationale

M52 first (freezes neutral rec behavior before any further influence work),
then M53 batch 1 (imports are the highest-leverage reader feature), M54/M55
in parallel after, M56 interleaved. At the observed pace (~11
requirements/day agent-driven), the concentrated estimate is 6–9 weeks;
adapter batches beyond batch 1 stretch the tail by 2–3 weeks and can run in
the background.
