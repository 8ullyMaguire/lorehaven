# Taste-Gravitational System — Implementation Plan v2

**Date:** 2026-09-22
**Source:** docs/spec-amendments/taste-gravitational-system.md + user streak refinement
**Status:** Active
**Supersedes:** docs/plans/taste-gravitational-system.md

---

## Design Principle: Signal Purity

The taste-gravitational system's core goal is to amplify *taste-aligned* signal. Every design decision is evaluated against one question: **does this make the taste signal stronger or weaker?**

A streak multiplier on all actions makes it *weaker* — it amplifies noise (off-target bookmarks, random kudos) as much as signal. A flat streak bonus or a login-only multiplier keeps the per-action economy clean while still rewarding the habit of showing up.

This is why the spec dismisses streak multipliers — not because of an abstract "episodic, never cumulative" principle, but because **multiplying all engagement equally dilutes the taste signal you're trying to amplify**.

---

## Phase 1: Foundation (ship together)

These three are the foundation — without all three, nothing else works.

### 1.1 Multi-dimensional Taste Profile (§0.4)

**Goal:** Replace scalar taste with a multi-dimensional model.

**Changes:**
- `crates/domain/src/taste_vector.rs` — already exists, extend with:
  - `TasteProfile` struct: `dimensions: Vec<TasteDimension>`, `exemplars: Vec<WorkId>`, `anti_examples: Vec<WorkId>`
  - `TasteDimension` struct: `key: String`, `label: String`, `admin_target: f64`, `weight: f64`
  - `TasteVector` struct: `dimensions: Vec<f64>` (user's position on each axis)
  - Functions: `compute_resonance(profile, user_vector) -> f64`, `distance(a, b) -> f64`
- `crates/app/src/config.rs` — add `TasteConfig` struct with dimensions, exemplars, anti_examples
- Database: `taste_vectors` table already exists (from earlier work), add `taste_profile` table for admin
- API: `GET /api/v1/meta` returns `taste_enabled: true/false`

**Tests:**
- TasteProfile serialization/deserialization
- Resonance computation with known inputs
- Distance metric correctness

### 1.2 Taste Resonance Score (§16.17)

**Goal:** Per-user alignment metric that powers everything else.

**Changes:**
- `crates/domain/src/taste_vector.rs` — add:
  - `ResonanceComponents` struct: `bookmark_overlap: f64`, `rating_correlation: f64`, `completion_alignment: f64`, `reading_time_ratio: f64`
  - `compute_resonance(components, weights) -> f64`
  - `qualitative_label(score) -> &'static str` ("Aligned: Strong" for >0.7, etc.)
- `crates/db/src/taste_vectors.rs` — already exists, extend with:
  - `fetch_user_bookmarks(account_id) -> Vec<WorkId>`
  - `fetch_admin_exemplars() -> Vec<WorkId>`
  - `compute_bookmark_overlap(user, admin) -> f64` (Jaccard)
  - `compute_rating_correlation(user, admin) -> f64`
  - `compute_completion_alignment(user, admin) -> f64`
  - `compute_reading_time_ratio(user, admin) -> f64`
  - `update_resonance_score(account_id, score)` — incremental update
  - `full_recompute_resonance(account_id)` — full recalculation
- Database: add `resonance_score` column to `accounts` table
- API: `GET /api/v1/me/taste-resonance` returns qualitative label only
- Admin API: `GET /api/v1/admin/users?sort=resonance` for admin panel

**Tests:**
- Jaccard similarity computation
- Resonance score bounds (0.0–1.0)
- Qualitative label mapping
- Cold start (insufficient data → 0.0)

### 1.3 Taste-Weighted Signals (§9.7.3)

**Goal:** Every user action carries a signal weight equal to their resonance.

**Changes:**
- `crates/domain/src/discovery.rs` — extend `Candidate` with `taste_signal: f64` (already done in earlier work)
- `crates/domain/src/discovery.rs` — add:
  - `apply_taste_gravity(candidates, strength, mode, admin_weight)` — already done in earlier work
  - `inject_diversity_by_taste(candidates, final_count, injection_percent)` — already done in earlier work
- `crates/app/src/config.rs` — add `SignalWeightMode` enum, `signal_weight_mode: String`
- `crates/app/src/routes/discovery.rs` — wire taste gravity into discovery pipeline
- Database: `work_signals` table for aggregated signal scores

**Tests:**
- Signal weight application (egalitarian, taste_weighted, admin_only)
- Diversity injection reserves correct portion
- Zero strength returns unchanged

---

## Phase 2: Health Layer

### 2.1 Anti-Echo-Chamber Valve (§0.4.1)

**Goal:** Reserve X% of discovery slots for taste-distant content.

**Changes:**
- `crates/domain/src/discovery.rs` — `inject_diversity_by_taste` already implements this
- Config: `diversity_injection_percent: f64` (already in TasteConfig)
- Wire into all discovery surfaces (home, search, tag pages, fandom pages)

**Tests:**
- Correct percentage reserved
- Quality threshold enforcement (≥100 words, etc.)

### 2.2 Onboarding Taste Quiz (§0.4.2)

**Goal:** New users get immediate resonance signal from quiz.

**Changes:**
- `crates/app/src/routes/quiz.rs` — new route:
  - `GET /api/v1/quiz/works` — returns quiz works (admin-selected)
  - `POST /api/v1/quiz/answers` — accepts selections, computes initial vector
- `crates/db/src/taste_vectors.rs` — `store_quiz_vector(account_id, selections)`
- Database: `quiz_answers` table
- Frontend: quiz component on registration flow

**Tests:**
- Quiz vector computation
- Skippable quiz falls back to egalitarian

### 2.2a Taste Calibration Arena (§0.4.2a)

**Goal:** Complement the onboarding quiz (§2.2) with a forced-tradeoff interaction that identifies *which dimensions of taste matter most to this reader* — the weights the quiz and swipes can't separate.

**Why both:** Like/dislike answers "is this good?" (absolute, 1 bit, noisy). Arena answers "what matters most?" (relative, ~3.6 bits per round, reveals dimensional weights through forced tradeoff). Psychophysics: relative judgments are far more consistent than absolute scoring. The arena calibrates the lens; the swipe feeds the engine.

**Changes:**
- `crates/domain/src/taste_vector.rs` — new:
  - `generate_arena_round(profile, works, config) -> Vec<WorkCard>` — pick 4 works sharing ≥1 attribute (fandom/genre/length bracket), maximizing variance on 1-2 target dimensions, with 300-word excerpts
  - `apply_arena_ballot(weights, round, best, worst, reasons) -> DimensionalWeights` — Plackett-Luce update (Elo-compatible start)
- `crates/app/src/routes/arena.rs` — new route:
  - `GET /api/v1/arena/next` — next round of 4 cards (active learning: highest model uncertainty)
  - `POST /api/v1/arena/vote` — submit best/worst/reason tags, returns updated weight hint (never the raw weights — §0.3)
  - `POST /api/v1/arena/dismiss` — skip arena (falls back to quiz or egalitarian)
- `crates/db/src/taste_vectors.rs` — `store_arena_weights(account_id, weights)`, `record_arena_ballot(account_id, round, best, worst, reasons)`
- `migrations/sqlite/0065_taste_arena.sql` + `migrations/postgres/0065_taste_arena.sql` — `arena_ballots` table (account_id, best_work_id, worst_work_id, reason_tags, created_at)
- Frontend: ArenaCard component (4-up grid, excerpt, BEST/WORST tap), ReasonTagBar (optional one-tap tags), ArenaIntro (onboarding flow)
- Config: `[taste_profile.arena]` — enabled, rounds_onboard (25), rounds_monthly (8), cards_per_round (4), dimensions_per_round (2), excerpt_words (300), reason_tags, reason_tags_optional, model ("plackett_luce" | "elo")

**Card construction rules:**
- Same content rating, same completion state, similar word count (±50%), ≥1 shared tag or fandom
- Maximize variance on exactly 1-2 dimensions per round (active learning)
- Show 300-word passage from Chapter 1 (not just metadata — prose is the hardest dimension to judge from summaries)

**Model:** Plackett-Luce (generalizes Bradley-Terry to partial rankings). Each round updates latent dimensional weights. After 20-40 rounds → reliable weight vector. Start with Elo for simplicity; upgrade when data volume justifies. Monthly mini-rounds (5-10 comparisons) track drift.

**Tests:**
- Arena round generation respects card construction rules (shared attribute, variance maximization)
- Plackett-Luce converges to known weights from synthetic ballots
- Elo fallback produces sane rankings
- Reason tags accelerate convergence (compare rounds-to-convergence with/without)
- Arena weights feed into resonance computation (§16.17) correctly
- Dismiss falls back to quiz/egalitarian
- Weights never exposed in API responses (§0.3)

**Effort:** M (new domain module + route + frontend component; DB touch is one table)

### 2.3 Taste Probes (§16.19)

**Goal:** Prevent taste profile ossification.

**Changes:**
- `crates/domain/src/taste_vector.rs` — `generate_probes(profile, distance) -> Vec<WorkId>`
- `crates/app/src/routes/discovery.rs` — inject probes into admin's discover
- `crates/db/src/taste_vectors.rs` — `record_probe_engagement(account_id, work_id, engagement)`
- Config: `taste_probes.frequency`, `distance`, `max_probes_per_session`, `auto_expand`

**Tests:**
- Probe generation at correct distance
- Positive engagement expands profile
- Negative engagement suppresses direction

---

## Phase 3: Engagement Layer

### 3.1 Fic Lifecycle Incentives (§9.8)

**Goal:** Incentivize finishing fics, not just starting them.

**Changes:**
- `crates/app/src/routes/works.rs` — on chapter publish:
  - Check if final chapter of multi-chapter work (≥3 chapters) → completion bonus
  - Check if work was stale (≥180 days) → resurrection reward
- `crates/db/src/credits.rs` — add lifecycle credit functions
- Config: `lifecycle.completion_multiplier`, `resurrection_multiplier`, `completion_boost`

**Tests:**
- Completion bonus fires only on final chapter of ≥3 chapter work
- Resurrection reward requires ≥180 day staleness
- Anti-gaming: single-chapter works don't trigger

### 3.2 Taste-Weighted Notifications (§9.9)

**Goal:** Notify aligned users when matching content is published.

**Changes:**
- `crates/app/src/routes/notifications.rs` — on work publish:
  - Score work against taste profile
  - If score > threshold, queue notifications for aligned users
- `crates/db/src/notifications.rs` — `queue_taste_notification(work_id, user_id)`
- Config: `taste_threshold`, `taste_batch_window`, `taste_max_per_day`, `popularity_bypass`

**Tests:**
- Notification triggered only above threshold
- Batching prevents spam
- Max per day enforced
- Popularity bypass works

### 3.3 Standing Bounties (§20.3.1)

**Goal:** Auto-create bounties when works match admin criteria.

**Changes:**
- `crates/db/src/bounties.rs` — `check_standing_bounties(work_id)` on work publish/rate
- `crates/app/src/routes/bounties.rs` — admin CRUD for bounty rules
- Config: `bounty_rules` array

**Tests:**
- Rule matching on tags + word count + rating
- Auto-fulfill creates bounty with correct status
- Vanguard provisional flow (3+ vanguard bookmarks → pending)

---

## Phase 4: Community Layer

### 4.1 Flexible Bounties (§20.3.2)

**Goal:** Crowdfunded, reverse, and collaborative bounty types.

**Changes:**
- Database: `bounties` table gets `type` column
- `crates/db/src/bounties.rs` — `create_crowdfunded_bounty`, `create_reverse_bounty`
- `crates/app/src/routes/bounties.rs` — new endpoints for each type
- Config: `bounties.allowed_types`

**Tests:**
- Each bounty type creation and fulfillment
- Crowdfunded activates when fully funded

### 4.2 Taste Vanguard Role (§16.18)

**Goal:** Top-aligned users become curators.

**Changes:**
- `crates/db/src/roles.rs` — `select_vanguards(method, threshold)`
- `crates/app/src/routes/vanguard.rs` — vanguard-specific endpoints
- Database: `vanguard_roles` table, `vanguard_pins` table
- Weekly batch job for selection
- Config: `vanguard.method`, `threshold_percent`, permissions

**Tests:**
- Selection by resonance_threshold, admin_appointment, contribution_volume
- Vanguard permissions (pin, nominate, create_clubs)
- Badge visibility

### 4.3 Referral System (§20.3.3)

**Goal:** Growth with taste alignment.

**Changes:**
- `crates/db/src/referrals.rs` — `create_referral_link`, `track_referral`, `process_tier_rewards`
- `crates/app/src/routes/referrals.rs` — `GET /api/v1/me/referrals`
- Database: `referrals` table
- Config: tier amounts, taste_threshold

**Tests:**
- Tier 1-4 reward triggers
- Taste bonus computation
- Anti-gaming: same-IP clusters flagged

### 4.4 Trust × Taste Coupling (§19.x)

**Goal:** Optional taste gate per trust level.

**Changes:**
- `crates/db/src/trust.rs` — `check_trust_advancement(account_id, level)` checks resonance
- `crates/app/src/routes/trust.rs` — advancement endpoint
- Config: `taste_coupling` array

**Tests:**
- User stalls at level if resonance insufficient
- TL6 bypasses coupling
- Never disclosed to user

---

## Phase 5: Incremental Improvements

### 5.1 Dynamic Tag Gravity (§0.4.6)

**Goal:** Tags gain/lose gravity based on admin engagement.

**Changes:**
- `crates/db/src/tags.rs` — `update_tag_gravity(tag, engagement)`
- `crates/domain/src/discovery.rs` — `apply_dynamic_tag_gravity(candidates, tag_scores)`
- Config: `dynamic_gravity`, `gravity_decay_days`, `gravity_max_boost`, `gravity_max_suppress`

### 5.2 Instance Presets (§0.6)

**Goal:** Bundled config defaults.

**Changes:**
- `crates/app/src/config.rs` — `Preset` enum, `apply_preset(preset)` function
- Config: `instance.preset`

### 5.3 Author Matchmaking (§14.5)

**Goal:** Hint authors about understaffed areas.

**Changes:**
- `crates/app/src/routes/works.rs` — `GET /api/v1/works/matchmaking-hints`
- `crates/db/src/tags.rs` — `find_understaffed_tags(demand_threshold)`

### 5.4 Reading Clubs (§17.6)

**Goal:** Manual gravity override for spotlight works.

**Changes:**
- `crates/db/src/clubs.rs` — CRUD for reading clubs
- `crates/app/src/routes/clubs.rs` — club endpoints
- Database: `reading_clubs` table

### 5.5 Streak Flat Bonuses (§9.7.1)

**Goal:** Reward daily presence without diluting the taste signal.

**Design rationale:** A streak multiplier on all actions amplifies noise (off-target bookmarks, random kudos) as much as signal, diluting the taste-weighted credit system. A flat milestone bonus rewards the habit of showing up without distorting per-action economics. Taste-weighted engagement already rewards aligned users more per action — a highly-aligned user logging in daily earns more than a misaligned user logging in daily, without needing a streak multiplier.

**Changes:**
- `crates/db/src/credits.rs` — `award_streak_milestone(account_id, days)`
- Config: `streak.milestone_7d` (default 5), `streak.milestone_30d` (default 15)
- Optional narrow alternative: `streak.login_multiplier` (default 1.0 = off; 1.5 = login credits only). This gives the dopamine hit of a growing streak without inflating the value of bookmarks, reviews, and kudos — which are the signals the taste algorithm actually depends on.
- Streak freeze remains at 5 credits
- Milestone bonuses are one-time, non-cumulative, logged as "streak milestone" category

**Tests:**
- 7-day milestone fires once
- 30-day milestone fires once
- Login multiplier (if enabled) applies only to login credits, not bookmarks/kudos/reviews
- Streak freeze works

---

## Phase 6: Algorithmic Meta-Ranking & Marketplace

### 6.1 Meta-Ranker with Built-in Strategies (§9.10.5)

**Goal:** Self-optimizing recommendation via Thompson Sampling over a pool of built-in strategies.

**Changes:**
- `crates/domain/src/meta_ranker.rs` — new module:
  - `Strategy` enum/trait (taste_gravity, popularity, recency, collaborative, completion_boosted, diversity, vanguard_consensus, dynamic_tag_gravity, lifecycle, taste_probe)
  - `BetaDistribution` struct: alpha, beta fields, sample() method, update(success, failure)
  - `MetaRanker` struct: strategy pool with Beta distributions
  - `select_strategy(exploration_percent)` — samples from Beta distributions, returns ranked strategy list
  - `record_outcome(strategy_idx, success)` — updates Beta distribution
  - `rebalance()` — recomputes exploration/exploitation split based on current distributions
- `crates/db/src/meta_ranker.rs` — persist strategy performance:
  - `strategy_impressions` table (strategy_id, impressions, successes, last_rebalanced)
  - `record_impression(strategy_id)` and `record_success(strategy_id)`
- Config: `meta_ranking.exploration_percent`, `exploitation_percent`, `min_impressions_per_strategy`, `rebalance_frequency`, `max_active_strategies`, `success_metric`, `auto_disable_threshold`, `candidate_exploration_bonus`
- Wire into discovery pipeline: before serving discovery results, ask meta-ranker for strategy allocation, fill slots accordingly

**Tests:**
- Beta distribution sampling converges after enough data
- Thompson Sampling gives more exploration to uncertain strategies
- Top-performing strategy gets exploitation budget
- Auto-disable fires after threshold breaches
- Config values respected

### 6.2 Admin Dashboard (§9.10.10)

**Goal:** Admin observability into meta-ranker performance.

**Changes:**
- `crates/app/src/routes/admin.rs` — `GET /admin/meta-ranking`
- Returns JSON: strategy rankings, confidence intervals, impressions, success rates, trending
- Admin can POST to lock/unlock/disable strategies

### 6.3 WASM Sandbox & Algorithm API (§9.10.2)

**Goal:** Extensible, safe execution environment for marketplace algorithms.

**Changes:**
- New crate: `crates/wasm-host/` (or similar)
- Use a WASM runtime (wasmtime or wasmer)
- Implement host function imports (get_work_metadata, get_user_public_signals, etc.)
- Tiered resource limits (base/author/curator) enforced per-invocation
- `crates/domain/src/marketplace.rs` — types for RecommendationContext, WorkCandidate, ScoredWork

### 6.4 Marketplace Registry & Incentives (§9.10.1, §9.10.7)

**Goal:** Community-contributed algorithm ecosystem.

**Changes:**
- `crates/db/src/marketplace.rs` — `installed_extensions`, `extension_reviews`, `creator_reputation`
- API endpoints: install, uninstall, review, publish
- Credit rewards tied to marketplace events (publication, install, performance, dominance)
- Badge awards for adoption milestones

### 6.5 User-Selectable Algorithms (§9.10.8)

**Goal:** Let users pick their preferred algorithm for personal feeds.

**Changes:**
- `crates/app/src/routes/user_settings.rs` — algorithm preference endpoint
- Config: `marketplace.user_algorithm_selection`, `user_surfaces`, `admin_surfaces`

---

## Database Migrations

| Migration | Description |
|-----------|-------------|
| 0054_taste_profile.sql | Admin taste profile table |
| 0055_resonance_score.sql | Add resonance_score to accounts |
| 0056_work_signals.sql | Aggregated signal scores per work |
| 0057_quiz_answers.sql | Onboarding quiz responses |
| 0058_bounty_rules.sql | Standing bounty configuration |
| 0059_vanguard_roles.sql | Vanguard role assignments |
| 0060_referrals.sql | Referral tracking |
| 0061_reading_clubs.sql | Reading club data |
| 0062_tag_gravity.sql | Dynamic tag gravity scores |
| 0063_strategy_impressions.sql | Meta-ranker strategy performance tracking |
| 0064_installed_extensions.sql | Marketplace installed extensions |
| 0065_extension_reviews.sql | Marketplace extension reviews |

---

## Config Additions

```toml
[instance]
preset = "curated_boutique"

[site.taste_profile]
enabled = true
dimensions = [
  { key = "angst", label = "Angst", admin_target = 0.8, weight = 0.3 },
  { key = "pacing", label = "Pacing", admin_target = 0.6, weight = 0.2 },
  { key = "prose_density", label = "Prose Density", admin_target = 0.7, weight = 0.2 },
  { key = "canon_compliance", label = "Canon Compliance", admin_target = 0.5, weight = 0.15 },
  { key = "trope_diversity", label = "Trope Diversity", admin_target = 0.6, weight = 0.15 },
]
exemplars = ["work_id_1", "work_id_2"]
anti_examples = ["work_id_3"]

[site.taste_profile.quiz]
enabled = true
min_works_shown = 10
min_works_picked = 3
skippable = true

[signals]
mode = "taste_weighted"
diversity_injection_percent = 10

[notifications]
taste_threshold = 0.5
taste_batch_window = "3h"
taste_max_per_day = 3
popularity_bypass = 0.9

[lifecycle]
completion_multiplier = 2.0
resurrection_multiplier = 3.0
completion_boost = 1.2

[taste_probes]
frequency = "weekly"
distance = 0.2
max_probes_per_session = 3
auto_expand = true

[vanguard]
method = "resonance_threshold"
threshold_percent = 10
can_pin = true
can_nominate = true
picks_shelf = true
can_create_clubs = true
public_badge = true
bounty_discount = 0.5

[trust]
taste_coupling = [
  { level = 1, min_resonance = 0.0 },
  { level = 2, min_resonance = 0.0 },
  { level = 3, min_resonance = 0.0 },
  { level = 4, min_resonance = 0.3 },
  { level = 5, min_resonance = 0.5 },
  { level = 6, min_resonance = 0.0 },
]

[bounties]
allowed_types = ["standard", "crowdfunded", "reverse"]

[referrals]
enabled = true
tier1_signup = 10
tier2_first_post = 10
tier2_taste_bonus = 30
tier3_retention = 50
tier3_taste_bonus = 50
tier4_high_resonance = 100
taste_threshold = 0.5

[theme]
dynamic_gravity = true
gravity_decay_days = 90
gravity_max_boost = 1.5
gravity_max_suppress = 0.5

[matchmaking]
enabled = true
subtlety = "hint"
min_readers_affected = 5
cooldown = "7d"

[club]
max_duration_days = 14

[streak]
milestone_7d = 5
milestone_30d = 15
login_multiplier = 1.0          # 1.0 = off; 1.5 = login credits only (narrow alternative)
freeze_cost = 5

[marketplace]
enabled = true
registry_url = "https://marketplace.lorehaven.org"
min_trust = "community_reviewed"
auto_update = false
user_algorithm_selection = true
user_surfaces = ["home_feed"]

[marketplace.algo_limits.base]
timeout_ms = 25
max_size_mb = 5
max_api_calls = 500

[marketplace.algo_limits.author]
timeout_ms = 50
max_size_mb = 10
max_api_calls = 1000

[marketplace.algo_limits.curator]
timeout_ms = 100
max_size_mb = 20
max_api_calls = 2000

[meta_ranking]
enabled = true
exploration_percent = 15
exploitation_percent = 85
min_impressions_per_strategy = 50
rebalance_frequency = "daily"
max_active_strategies = 20
success_metric = "admin_aligned"
auto_disable_threshold = 0.3
candidate_exploration_bonus = 2.0
candidate_promotion_impressions = 500
```

---

## Implementation Order

1. **Phase 1.1** (Multi-dimensional Taste Profile) — foundation
2. **Phase 1.2** (Taste Resonance Score) — foundation
3. **Phase 1.3** (Taste-Weighted Signals) — foundation
4. **Phase 2.1** (Anti-Echo-Chamber Valve) — health
5. **Phase 3.1** (Fic Lifecycle Incentives) — engagement
6. **Phase 3.2** (Taste-Weighted Notifications) — engagement
7. **Phase 4.2** (Taste Vanguard Role) — community
8. **Phase 5.5** (Streak Flat Bonuses) — trivial
9. **Phase 5.1** (Dynamic Tag Gravity) — incremental
10. **Phase 2.2** (Onboarding Taste Quiz) — health
11. **Phase 2.3** (Taste Probes) — health
12. **Phase 3.3** (Standing Bounties) — engagement
13. **Phase 4.1** (Flexible Bounties) — community
14. **Phase 4.3** (Referral System) — community
15. **Phase 4.4** (Trust × Taste Coupling) — community
16. **Phase 5.2** (Instance Presets) — incremental
17. **Phase 5.3** (Author Matchmaking) — incremental
18. **Phase 5.4** (Reading Clubs) — incremental
19. **Phase 6.1** (Meta-Ranker with Built-in Strategies) — self-correcting recommendation
20. **Phase 6.2** (Admin Dashboard) — observability
21. **Phase 6.3** (WASM Sandbox & Algorithm API) — extensibility
22. **Phase 6.4** (Marketplace Registry & Incentives) — ecosystem
23. **Phase 6.5** (User-Selectable Algorithms) — personalization

---

## Success Criteria

- All domain unit tests pass
- `cargo check` clean for all crates
- Taste gravity can be enabled/disabled per instance
- Resonance score is never exposed as raw number
- Diversity injection reserves correct percentage
- All config parameters are individually overridable
- Streak bonuses are flat (not multiplicative) by default
- Login-only multiplier (if enabled) does not inflate bookmark/kudos/review credits
