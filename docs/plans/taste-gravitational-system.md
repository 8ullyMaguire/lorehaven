# Taste-Gravitational System — Implementation Plan

**Date:** 2026-09-22
**Source:** docs/spec-amendments/taste-gravitational-system.md
**Status:** Active

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

**Goal:** Flat milestone bonuses without diluting signal.

**Changes:**
- `crates/db/src/credits.rs` — `award_streak_milestone(account_id, days)`
- Config: streak milestone amounts

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

---

## Success Criteria

- All domain unit tests pass
- `cargo check` clean for all crates
- Taste gravity can be enabled/disabled per instance
- Resonance score is never exposed as raw number
- Diversity injection reserves correct percentage
- All config parameters are individually overridable
