# Remaining work to the from-scratch target spec (ADR 0024)

Status: **the live plan for closing the gap between this repository and the
consolidated from-scratch specification.** Produced 2026-09-24 by auditing
`docs/requirements.csv`, `docs/verification.md`, the git log and the test
suite against the from-scratch spec. Supersedes nothing —
`junior-implementation-plan.md` §0 remains the historical map; this file is
the forward plan. Every new milestone below gets rows in
`docs/requirements.csv` before code starts (ADR 0023: the CSV is the
canonical feature inventory and the roadmap-board seed).

## Where the build actually stands (re-derived 2026-09-30)

**Re-derived from `docs/requirements.csv` and the tree, not carried forward.**
The previous header (2026-09-25) claimed 588 implemented / 75 planned / 667
total and "migrations to 0083"; all of those are now wrong, which is what the
note at the top of this file exists to prevent.

| | count |
|---|---|
| implemented (all three grades) | **611** |
| — fully tested | 289 |
| — locally tested | 206 |
| — verified end-to-end | 116 |
| planned | 60 |
| built (not graded) | 7 |
| deliberately unsupported | 4 |
| evaluated and rejected | 1 |
| **total rows** | **683** |

Migrations: **91 per dialect**, last `0094_retention_proposals` (both
`migrations/sqlite/` and `migrations/postgres/`, parity-tested by
`the_two_dialects_declare_the_same_columns_and_indexes`). App test files: **118**.

To re-derive, rather than believe:

```sh
python3 -c "import csv,collections; \
  r=list(csv.DictReader(open('docs/requirements.csv'))); \
  print(collections.Counter(x['status'] for x in r))"
ls migrations/sqlite/*.sql | wc -l
ls crates/app/tests/*.rs | wc -l
```

Built: the platform core (M0–M15), the forum series (M31–M35), media resilience
(M48–M51), roadmap consensus (M45), browse ordering (M43), export CTAs (M42), user
settings (M47), and **M59 crawling posture, retention governance and preservation
(phases A–F complete; 15 of 27 requirement rows implemented — see
the M59 section)**. Still to the from-scratch spec: the recommendation strategy
registry, most source adapters, the bot, and a published OpenAPI document.

## Milestones

### M59 — Crawling posture, retention governance, preservation (spec §5, §7.7, §19.15, §11.5, §11.15)

Phases A–F are built and verified on both dialects, and so is `M59-10` under its
own plan. The amendment is
`docs/spec-amendments/crawling-retention-and-preservation.md`; the plans are
`docs/plans/crawling-retention-preservation.md` and
`docs/plans/m59-10-reader-body-request.md`; the evidence is
`docs/verification.md`'s newest sections.

What shipped: a named robots posture with per-adapter overrides, a
class-specific UA token, body-audience baselines, the preservation credit loop
and recheck pass, and community retention governance — reader proposals,
ballots, an advisory/binding switch, operator respond and override, and a
settlement pass.

**All 27 requirement rows are now `implemented-locally-tested`, and that took two
kinds of correction that should not be conflated.**

**Twelve were `planned` when this section was last written. Eleven of them were
already built** — by phases C, C1 and D — and the ledger had never been updated.
They are now marked implemented, each naming the suite that pins it rather than
resting on this document's word:

| Row | Pinned by |
|---|---|
| M59-07 | `crates/app/tests/retention_routes.rs` — the setting, source overrides, refusals |
| M59-11 | `milestone_0.rs`, `retention_routes.rs` — the crosspost verification loop |
| M59-12 | `retention_routes.rs` — credits sized by threshold, decay and cap |
| M59-13 | `retention_routes.rs` — clawback when a destination dies |
| M59-14 | `retention_routes.rs` — the redistribution assertion gate |
| M59-15 | `retention_routes.rs` — badge and Top Preservers |
| M59-16 | `retention_routes.rs` — eligible-destination reporting |
| M59-24 | `retention_routes.rs` — body audience narrowing |
| M59-25 | `retention_routes.rs` — the rating ceiling, applied before audience |
| M59-26 | `retention_routes.rs` — the indistinguishable refusal |
| M59-27 | `retention_routes.rs` — the vanguard gate |

**The twelfth, `M59-10`, was a real gap** — the one M59 row no phase in
`crawling-retention-preservation.md` was assigned to, so it had neither code nor a
plan. It is now built: a reader's own copy of an external body (spec §11.15b),
under its own plan `docs/plans/m59-10-reader-body-request.md`, verified by
`crates/app/tests/reader_body_copies.rs` (the seven tests of the §6.4 acceptance
list), by the config tests including the §6.4.4 source scan, and by two
injection checks that a green run alone cannot give: the guard scan was shown to
fail and name the offending file, and the privacy test was shown to fail when the
account-id leak it caught is put back.

Two of the three things a reader might call gaps are decisions, recorded as such
in the amendment's build-status section: a retention ballot grants no trust, XP,
badge or placement (deliberately — the plan's own reasoning applied against its
own convenience), and the `[preservation]` config block is named
`[media_resilience]`.

**The lesson is worth keeping, because it nearly went the other way.** A ledger
that says "planned" for built work invites either a duplicate implementation or a
report that M59 is a third unfinished. The check that settles it in one command is
to probe for the code and the tests that own each row, rather than reading the
requirement text against the source — a requirement whose subject matter has a
whole passing suite is implemented whatever the CSV says.

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
- Rows: M52-01…M52-08. Depends on: nothing open.
  M52-01…08 are built. M52-08 (shadow-mode evaluation) landed: a third
  `rec.mode` value that serves `legacy` and records the comparison against
  `pluggable`, with an operator report at `GET /api/v1/operator/rec/shadow`.
  M52 is closed.

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

### M57 — Metadata exchange (spec §11.17, §15.17, §19.14, §2.3.1, new)

The community metadata exchange, adopted 2026-09-25 after triaging an external
proposal (`docs/plans/metadata-exchange-triage.md`). Not the proposal's diff —
its section citations were substantially wrong, and the corrections are recorded
in the triage. The spec text is landed; these rows are the build order.

Ordered by dependency, not by appeal:

1. **`lore-metadata` crate** (§2.3.1) — serde-only wire types, version
   negotiation, no IO. Nothing else depends on it, so it is genuinely
   order-independent and can be written first or last. Publishing it early is
   what keeps a compatible third-party implementation possible.
2. **Signal submission** (§11.17) — the schema prohibition is the deliverable.
   Fields outside `WorkSignal` must be refused *by name*, not silently trimmed,
   and that needs a test that sends one and reads the error.
3. **Auto-created canonical entities** (§15.17) — the unverified/curated split.
   The load-bearing claim is that an unverified entity is usable immediately and
   visibly so; the risk is shipping it as hidden-until-quorum, which stalls
   organic growth.
4. **Trust bars** (§19.14) — TL1 to submit, TL3 to curate, neither configurable.
   Small, and it belongs with the curation surface rather than ahead of it.
5. **Latent demand** (§16.16.1) — the one piece that touches ranking, so it
   comes after the exchange itself works end to end.

Build-order note: this goes **after M54/M55**, not beside them. The exchange is
an API surface, and an API surface shaped before any external consumer exists is
shaped by imagination. The bot is the first real client.

**What is deliberately not in scope**, with the reason: no credits for
submission (§0.3, §19.2 — a reward attached to a contribution is how a
contribution becomes influence, and it is a spam vector); no reading data, ever
(the field does not exist in the type, so no config widens it); no content or
excerpts; no automatic application of received metadata (it feeds the existing
tag-proposal queue, §15.11).

### M57 — Metadata exchange (spec §11.17, §15.17, §19.14, §2.3.1, new)

The community metadata exchange, adopted 2026-09-25 after triaging an external
proposal (`docs/plans/metadata-exchange-triage.md`). Not the proposal's diff —
its section citations were substantially wrong, and the corrections are recorded
in the triage. The spec text is landed; these rows are the build order.

Ordered by dependency, not by appeal:

1. **`lore-metadata` crate** (§2.3.1) — serde-only wire types, version
   negotiation, no IO. Nothing else depends on it, so it is genuinely
   order-independent and can be written first or last. Publishing it early is
   what keeps a compatible third-party implementation possible.
2. **Signal submission** (§11.17) — the schema prohibition is the deliverable.
   Fields outside `WorkSignal` must be refused *by name*, not silently trimmed,
   and that needs a test that sends one and reads the error.
3. **Auto-created canonical entities** (§15.17) — the unverified/curated split.
   The load-bearing claim is that an unverified entity is usable immediately and
   visibly so; the risk is shipping it as hidden-until-quorum, which stalls
   organic growth.
4. **Trust bars** (§19.14) — TL1 to submit, TL3 to curate, neither configurable.
   Small, and it belongs with the curation surface rather than ahead of it.
5. **Latent demand** (§16.16.1) — the one piece that touches ranking, so it
   comes after the exchange itself works end to end.

Build-order note: this goes **after M54/M55**, not beside them. The exchange is
an API surface, and an API surface shaped before any external consumer exists is
shaped by imagination. The bot is the first real client.

**What is deliberately not in scope**, with the reason: no credits for
submission (§0.3, §19.2 — a reward attached to a contribution is how a
contribution becomes influence, and it is a spam vector); no reading data, ever
(the field does not exist in the type, so no config widens it); no content or
excerpts; no automatic application of received metadata (it feeds the existing
tag-proposal queue, §15.11).

### M57 — Metadata exchange (spec §11.17, §15.17, §19.14, §2.3.1, new)

The community metadata exchange, adopted 2026-09-25 after triaging an external
proposal (`docs/plans/metadata-exchange-triage.md`). Not the proposal's diff —
its section citations were substantially wrong, and the corrections are recorded
in the triage. The spec text is landed; these rows are the build order.

Ordered by dependency, not by appeal:

1. **`lore-metadata` crate** (§2.3.1) — serde-only wire types, version
   negotiation, no IO. Nothing else depends on it, so it is genuinely
   order-independent and can be written first or last. Publishing it early is
   what keeps a compatible third-party implementation possible.
2. **Signal submission** (§11.17) — the schema prohibition is the deliverable.
   Fields outside `WorkSignal` must be refused *by name*, not silently trimmed,
   and that needs a test that sends one and reads the error.
3. **Auto-created canonical entities** (§15.17) — the unverified/curated split.
   The load-bearing claim is that an unverified entity is usable immediately and
   visibly so; the risk is shipping it as hidden-until-quorum, which stalls
   organic growth.
4. **Trust bars** (§19.14) — TL1 to submit, TL3 to curate, neither configurable.
   Small, and it belongs with the curation surface rather than ahead of it.
5. **Latent demand** (§16.16.1) — the one piece that touches ranking, so it
   comes after the exchange itself works end to end.

Build-order note: this goes **after M54/M55**, not beside them. The exchange is
an API surface, and an API surface shaped before any external consumer exists is
shaped by imagination. The bot is the first real client.

**What is deliberately not in scope**, with the reason: no credits for
submission (§0.3, §19.2 — a reward attached to a contribution is how a
contribution becomes influence, and it is a spam vector); no reading data, ever
(the field does not exist in the type, so no config widens it); no content or
excerpts; no automatic application of received metadata (it feeds the existing
tag-proposal queue, §15.11).

### M56 — Instance posture and the remaining planned rows

M56-01 — **instance accessibility mode** (`public` | `walled_garden` |
`private`, spec §0.4.7) is **implemented and fully tested**. The mode is
translated once into an `AccessPolicy` rather than checked per handler, so
every content surface honours it by construction. An unrecognised value stops
startup.

The rest of M56 is the **46 planned M45 rows** (taste-arena residuals and
category governance). These predate ADR 0024 and stay first-class. ~1 week.

The M47 settings surfaces the earlier draft of this paragraph referred to are
**no longer planned** — every M47 row is now `implemented-fully-tested`. The
count changed without this paragraph being updated, which is exactly the status
drift this file is supposed to prevent; the numbers above are re-derived from
`docs/requirements.csv`, not carried forward.

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

### M59 — Crawling posture, retention governance, and preservation (spec §11.5, §11.15, §11.12a, §11.15a, §11.15b, §19.15, amended)

Spec: `docs/spec-amendments/crawling-retention-and-preservation.md`.
Plan: `docs/plans/crawling-retention-preservation.md`. Rows: M59-01…M59-27,
all `planned`.

`imports.honour_robots` becomes `imports.robots_posture`
(`strict | metadata_only | permissive`) with every fetch declaring a class, and
§11.15's unbuilt `cache | aggregate` retention (**M6-15**, recorded
`unsupported` since 2026-09-11) is built as specified. Preservation targets
crosspost a work to a configured archive and earn credits and a badge for
**verified live** preservation, with a ledger reversal when a destination
dies. Retention may be decided by a trust-gated community vote in `advisory`
(default) or opt-in `binding` mode, with asymmetric quorum, a cooling period
and a recorded operator override.

Two things this deliberately does not build: §11.11 preservation *batches*
(M6-10) stay unbuilt, and crossposting an **imported** work is refused by name
because inherited `redistribution` permission needs the §23.7 transfer
manifest, which is deferred. Local works are the shippable slice.

**§11.10's cross-source identity is a prerequisite, and none of its four tables
exists.** `story_identities` appears in the §3 data model and is absent from
`crates/`, from `migrations/`, and from this CSV — so the roadmap seeder has
never seen it either. What exists is `library_items` (0006) keyed on
`UNIQUE(account_id, source_key, source_work_key)`, which means importing the
same fic from two sites produces two unrelated rows and no link. Phase A0
builds the minimum (`story_identities` + `story_identity_members`, one
`edition_relation` value, no inference), and a preservation target becomes a
*state column on an identity member* rather than a parallel table that a later
identity milestone has to reconcile. M59-21…M59-23.

Order: **A0 identity** → A posture+class → B UA token → C retention → D
preservation → E proposals. A and C can run in parallel; D and E share the
retention routes and serialise; D cannot start before A0 is green.

### Then — hardening and release

E2E green (73/73 as of `9cd3c71`, the commit that fixed the export-delete
assertion), deploy to thinkcentre production (build there, never through
SSHFS). A `v1.0.0` tag is deliberately **not** cut: the release decision is the
operator's, and the remaining M45/M47/M53/M54 rows are open.

## M32 perceptual dedup — the search is done, populating the column is not

`db::media_resilience::find_by_perceptual_hash` is a real Hamming-distance
search as of `3346144`, ordered closest first, and the `[media_resilience]`
config table it reads is wired to TOML for the first time. The route
`POST /api/v1/media/reverse-search` takes the operator's threshold and returns
`match_distance`, `match_confidence`, `match_kind` and `auto_attach` per match,
and the admin media page shows them.

**The remaining piece is M32-07b: nothing computes a perceptual hash.** The
column is only ever written by a test, so the search is correct on an empty
column and the feature is not reachable from the site. The blocker is real
rather than incidental:

- image decoding is not a dependency of this workspace, so applying
  `perceptual_hash_algorithm` to image bytes is a new crate, not wiring;
- audio needs the same treatment for `audio_fingerprint`;
- the two keys `require_curator_confirmation_below` and
  `require_curator_confirmation_above` are parsed and validated but no merge
  path reads them, because there is no merge path — the curator workflow
  §32.7.2 describes is unbuilt.

Order of work when M32-07b is picked up: decode and fingerprint in the fetch
path, then the merge door that consumes the two confirmation keys, then the
front-end affordance for it. Doing the merge door first would build a workflow
around a column nothing populates.

## Gate discipline — read before trusting any count in this file

A `cargo clippy` gate in this repo reported **0 warnings and 0 errors** for
several sessions while the tree held 30. The command was
`cargo clippy ... 2>/dev/null | grep -cE '^(warning|error)'`: rustc and clippy
write warnings to **stderr**, so the `2>/dev/null` threw them all away and the
`grep -c` counted an empty stdout. See
`docs/handoffs/2026-09-25T142500+0200-false-clean-gate-three-defects-e2e-assertion-fixed-handoff.md`.

When running any gate in this project:

- Redirect with `> /tmp/x.log 2>&1`, then read the file. Never pipe through
  `grep -c` without `2>&1`.
- If a gate has never once failed, suspect the gate before trusting it.
- A green E2E that asserts an empty state or a global count on a shared test
  account is a false signal too. The export-delete test asserted `"No exports
  yet"` on an account the export-download test above it also writes to, so the
  empty state could never appear — it tested the fixture, not the feature.

## Order and rationale

M52 first (freezes neutral rec behavior before any further influence work),
then M53 batch 1 (imports are the highest-leverage reader feature), M54/M55
in parallel after, M57 once there is a real API consumer, M56 interleaved. At
the observed pace (~11 requirements/day agent-driven), the concentrated estimate
is 6–9 weeks; adapter batches beyond batch 1 stretch the tail by 2–3 weeks and
can run in the background.

## Verification debt, stated plainly

**299 rows are not `implemented-fully-tested`** (183 locally-tested, 116
verified-e2e, of 660). That is the largest soft spot in a green board, and
it is why M57 is not simply next in line. A public write path built on
machinery that has never been exercised through the full stack is a different
risk from the one the external proposal named when it argued about sequencing —
its argument was that M10 taxonomy and M14 trust were unbuilt, and both are
built. The real constraint is verification depth, not milestone order.

The number is not a claim that 299 rows are broken. It is a claim that 299 rows
have not been held to the full-stack bar, and the honest way to find out which
those are is a sweep that promotes or downgrades each row on evidence — not a
re-read of this file. That sweep is unowned and unestimated; it is listed here so
it is not mistaken for finished. (Re-derived 2026-09-27; the previous figure of
172 of 253 predated the M54–M58 rows.)

One more thing worth stating plainly: a public write path is the first thing in
this spec that lets data leave the instance. Everything else here is either
local or an inbound fetch. The §0.3 signal prohibition is the reason that is
survivable, and it is worth reading once before the endpoint is written, not
after.
