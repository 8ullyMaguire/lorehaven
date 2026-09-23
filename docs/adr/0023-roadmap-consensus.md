# ADR 0023: Roadmap consensus — Elo-ranked feature board

- Status: accepted
- Date: 2026-09-23

## Context

Feature direction on Lorehaven has been operator-decided alone. The community governs moderation
(§19.13) but has no say in what gets built. FicHub solved this with an Elo/MaxDiff consensus
engine (`fichub-consensus`: pairwise best/worst choices → Elo, kanban stages, trust-gated voting)
that has been running in production there. Lorehaven has nothing equivalent.

## Decision

Port the FicHub design as spec §44 / Milestone 45:

- Cards seeded from `docs/requirements.csv` (every requirement row is a card) plus user
  suggestions, deduplicated by normalized title.
- MaxDiff arena: ballot of 4 `idea`-stage cards, pick best + worst, translated to virtual 1v1
  Elo matches (K=32). Only `idea` cards are arena-eligible; other stages are frozen.
- Voting and suggesting gated at TL ≥ 1 via the existing trust ladder (`governance::trust_for`).
- Stage moves are operator-only and land in a public changelog with reasons.
- The board is anonymously readable — transparency is the point.
- Ballots log pre-match Elo per card, so historical rankings are reconstructable and offline
  evaluation is unbiased (uniform ballot propensity).

Division of authority: **Elo is the community's, stage is the operator's.** The operator may move
a card regardless of its Elo; the community cannot move stages. This keeps §0.3 intact — taste
curation stays invisible, prioritization is public.

## Consequences

- The pure Elo math is copied from `fichub-consensus` (MIT-licensed, same author) rather than
  reimplemented; storage is rewritten for the dual SQLite/Postgres `Database`.
- Seeding is idempotent and never downgrades a `shipped` card.
- `docs/requirements.csv` becomes the canonical feature inventory: implemented = shipped cards,
  planned = idea/up_next cards, unsupported = rejected cards. The seed script keeps the DB in
  sync with the CSV, not the other way round.
- No embeddings: FicHub clusters suggestions by vector similarity via Ollama; Lorehaven's volume
  does not justify the dependency yet. Normalized-text matching only. Revisit if suggestion
  volume makes duplicates a problem.
