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

### M56 — Close M45 + M47 planned rows

The 49 planned rows: taste-arena residuals (M45) and the settings surfaces
(M47-02/05/07). These predate ADR 0024 and stay first-class. ~1 week.

### Then — hardening and release

E2E green (3 known failures per handoff), tag v1.0.0, deploy to thinkcentre
production (build there, never through SSHFS).

## Order and rationale

M52 first (freezes neutral rec behavior before any further influence work),
then M53 batch 1 (imports are the highest-leverage reader feature), M54/M55
in parallel after, M56 interleaved. At the observed pace (~11
requirements/day agent-driven), the concentrated estimate is 6–9 weeks;
adapter batches beyond batch 1 stretch the tail by 2–3 weeks and can run in
the background.
