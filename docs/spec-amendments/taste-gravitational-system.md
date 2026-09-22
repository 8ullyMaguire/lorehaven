# Lorehaven Spec Amendment — Taste-Gravitational System

**Status:** Final plan
**Date:** 2026-09-22
**Supersedes:** Draft implementation plan (supersedes scattered thread)
**Source:** Consolidation of design thread + spec premise

---

## Overview

These amendments extend the existing taste infrastructure (demand multiplier, theme gravity, admin taste profile) into a full **taste-gravity engine**: a multi-dimensional taste model, weighted signal propagation, a curation layer, and instance-configurable presets. The operator's taste shapes every surface through silent algorithmic gravity, while the instance retains full control over how strongly, how visibly, and how democratically that gravity operates.

**Priority alignment:** These changes serve priority stack items 1 (customizable), 2 (maximize high-quality fiction), 4 (admin-aligned but never monothematic), and 5 (trust-level self-governance). They never violate foundational protections (admin taste invisible, no purchased ranking, honest verification only).

---

## §0.4 Instance Topics → §0.4 Instance Topics & Taste Profile

**Modification.** Instance Topics (already in spec) gain a companion structure: the **Instance Taste Profile**. Both are optional. An instance with neither is topic-agnostic and taste-neutral — the gravity engine defaults to disabled.

The Taste Profile replaces what the current spec calls the admin taste profile (`§16.2`) with a richer model:

- **`dimensions: [{key, label, admin_target, weight}]`** — each dimension is a named axis (e.g., `angst`, `pacing`, `prose_density`, `canon_compliance`, `trope_diversity`). `admin_target` is the admin's ideal position on that axis (0.0–1.0). `weight` determines how strongly the dimension influences recommendations. Admin defines which dimensions matter and how much.
- **`anti_examples: [work_id]`** — works the admin explicitly dislikes. Used as negative anchors. More informative than dimensions alone: "I hate this despite it scoring well" teaches the model fast.
- **`exemplars: [work_id]`** — works the admin loves. Positive anchors for calibration.
- Dimensions are **instance-defined** — no hardcoded taxonomy. An admin who only cares about angst and pacing defines two dimensions. An admin who cares about eight defines eight.

**Why multi-dimensional over scalar:** A single resonance score collapses "this user loves darkfic in one fandom and fluff in another" into noise. Dimensions let the system distinguish axes of alignment. Scalar resonance remains computable as a projection, but the underlying model is richer.

Config (`[site] taste_profile = { enabled, dimensions, exemplars, anti_examples }`). Reported on `/api/v1/meta` only as `taste_enabled: true/false` — never the dimensions, never the targets.

---

## §9.7 Gamification — Amendments

### §9.7.1 Episodic Credits → allow streak-linked flat bonuses

**Modification.** The current principle "episodic, never cumulative" blocks multipliers. Replace with: **credits for actions remain episodic and non-cumulative; a flat bonus may be granted for streak milestones (7-day, 30-day) but no persistent multiplier may attach to per-action credits.**

This allows:
- 7-day streak: +10 flat credits (one-time, not a multiplier)
- 30-day streak: +30 flat credits
- Streak freeze remains at 5 credits

The rationale: streak multipliers inflate *all* signal equally, diluting the taste-weighted credit system. Flat bonuses reward the habit without distorting per-action economics.

### §9.7.2 Quality Multiplier → unchanged

No change.

### §9.7.3 Demand Multiplier → Taste-Weighted Signals (major extension)

**Modification.** The existing demand multiplier (1.0×–1.5×, folds in admin taste, never disclosed) is expanded into a general **Taste-Weighted Signals** system that operates not just on credit earnings but on the entire recommendation algorithm.

Every user action (kudos, bookmark, comment, completion, reading time, re-read) carries a **signal weight** equal to that user's alignment with the instance taste profile. Signal weights feed into:

- **Recommendation scoring** on every surface (discover, search, fandom pages, tag pages, author pages, notifications)
- **Credit demand multiplier** (existing behavior, now derived from the same alignment score)
- **Community signal aggregation** — a bookmark from a 0.9-aligned user counts more toward a work's visibility than a bookmark from a 0.1-aligned user

**Alignment score** is the Taste Resonance Score (see §16.17 below). It is always hidden. The admin's own actions carry the maximum signal weight (1.0×).

**Instance config:**

```yaml
[signals]
mode = "taste_weighted"    # egalitarian | taste_weighted | admin_only
# egalitarian: all user signals count equally (demand multiplier only)
# taste_weighted: signals weighted by resonance (default when taste profile exists)
# admin_only: only admin actions influence recommendations
```

An instance without a taste profile defaults to `egalitarian`. An instance can always fall back.

**Never disclosed:** signal weights, resonance scores, and their influence on any ranking are invisible in any label, tooltip, breakdown, or API response. Only the behavior changes.

### §9.7.4 Daily/Monthly Caps → topic bonus integration

**No structural change.** Topic bonuses (§0.4) add rows within existing cap windows. Configurable whether topic bonuses count against caps (default: yes).

### §9.7.5 Leaderboards → per-topic and taste-aligned categories

**Modification.** Existing windowed leaderboards remain unchanged. Add:

- **Per-public-topic boards** when topics are configured (already spec'd in §0.4)
- **"Taste-Aligned" board** (optional, config: `taste_board_enabled`): top-20 by cumulative taste signal contribution — i.e., users whose actions most shaped aligned content visibility. This is **not** a resonance leaderboard (resonance is hidden). It measures contribution to the gravity engine. Opt-out available like other boards.

### §9.7.6 Badges → lifecycle and vanguard badges

**Add to milestone table:**

| Badge | Trigger | Count |
|-------|---------|-------|
| Completionist | Finish a multi-chapter work (final chapter posted) | ×5, ×20, ×50 |
| Resurrectionist | Finish a work whose last chapter was 180+ days stale before you posted a new one | ×1, ×5 |
| Streak of Strikes | Finish works on N consecutive days | ×3, ×7, ×14 |
| Tastemaker | Your bookmark was within the top-10% of taste signal for a work that later reached top-20 discover visibility | ×1, ×5 |
| Vanguard | Assigned Taste Vanguard role | one-time |
| Herald | Referred 5+ users who reached 30-day retention | one-time |
| Ambassador | Referred 25+ users who reached 30-day retention | one-time |

### §9.7.7 Streaks → allow flat milestone bonuses

**Modification.** As noted in §9.7.1 above. Streaks remain private by default, reset without guilt, freeze for 5 credits. Milestone flat bonuses are non-recurring and logged in transaction history as a separate category ("streak milestone").

### §9.7.8 Anti-gaming → add resonance gaming protection

**Add:** Resonance score is never exposed to any user via API, page, label, or notification. The taste-profile dimensions are never exposed. Only `taste_enabled: true/false` appears on `/api/v1/meta`. Users can infer broad alignment from behavioral signals (Vanguard role visible, taste-board presence) but cannot reverse-engineer the model. This is an extension of the foundational protection "admin taste invisible."

---

## §9.8 Fic Lifecycle Incentives (NEW)

**New section.** Directly addresses the completion problem that undermines admin enjoyment (finding a perfect fic abandoned at chapter 4).

| Incentive | Trigger | Reward | Config |
|-----------|---------|--------|--------|
| **Completion bonus** | Posting final chapter of a multi-chapter work (≥3 chapters) | 2× standard chapter credits for that chapter | `lifecycle.completion_multiplier: 1.5–3.0` |
| **Resurrection reward** | Posting a chapter on a work whose previous chapter was ≥180 days old | 3× standard chapter credits | `lifecycle.resurrection_multiplier: 2.0–5.0` |
| **Completion boost (algorithmic)** | Work completion state = completed | Permanent discovery/search score boost over WIPs at equal alignment | `lifecycle.completion_boost: 1.1–1.5` |
| **WIP visibility (neutral, not penal)** | Work completion state = WIP | No penalty, but no completion boost | — |
| **Series completion** | All works in a series completed | Per-work completion bonus for the series bonus row | — |

**Anti-gaming:** Completion bonus only fires on final chapter of a work with ≥3 chapters (prevents trivial "complete" single-chapter works from gaming it). Resurrection reward requires the work to have been genuinely stale (≥180 days) before the new chapter. Resurrection reward applies to the chapter author only.

**Rationale:** Fanfic archives are full of abandoned WIPs. This directly serves "maximize high-quality fiction" (priority 2) by incentivizing finish, not just start.

---

## §9.9 Taste-Weighted Notifications (NEW)

**New section.** The highest-leverage feature for driving reads of aligned content: put aligned fics directly in front of the right readers at the right time.

**Mechanism:** When a new work is published (or a work achieves a milestone like completing), the system scores it against the instance taste profile. If the score exceeds the notification threshold, taste-aligned users receive a notification. Notification batching prevents spam.

```yaml
[notifications]
taste_threshold = 0.5        # minimum alignment to trigger taste notifications
taste_batch_window = "3h"    # batch taste notifications to avoid spam
taste_max_per_day = 3        # max taste-driven notifications per user per day
popularity_bypass = 0.9      # works above this popularity score notify regardless of alignment
```

**For users without taste data (new accounts):** taste notifications are suppressed until resonance is computable (see §16.17 cold start). Default notifications (new chapters from followed authors, etc.) unaffected.

**Never disclosed:** The user sees "New fics you might like" — not "aligned with instance taste."

---

## §14.5 Author Taste Matchmaking (NEW)

**New section.** Shapes what gets *written*, not just what gets surfaced.

When an author opens the "new work" form or adds tags to an existing work, the system subtly surfaces understaffed areas of high instance demand:

- "Works tagged [X] are currently underrepresented in this instance"
- "Readers have been searching for [topic] but finding few matches"

These hints are drawn from the intersection of instance taste profile dimensions and tag coverage gaps. They are **hints, not requirements** — the author can dismiss them.

```yaml
[matchmaking]
enabled = true
subtlety = "hint"           # hint (subtle tooltip) | suggestion (inline card) | prompt (blocking, discouraged)
min_readers_affected = 5    # only hint if ≥5 taste-aligned readers searched this tag
cooldown = "7d"             # each author sees each hint at most once per 7 days
```

**No topics / no taste profile:** matchmaking disabled.

---

## §17.6 Reading Clubs / Spotlight Events (NEW)

**New section.** Manual gravity override: when the admin (or a Vanguard) finds a work they love, they can concentrate community attention on it.

- **Create a Reading Club:** admin or vanguard selects a work or theme. Front-page placement for the duration. Participants earn a flat bonus (configurable) for reading, commenting on, or bookmarking the spotlight work. Badge: "Reading Club Participant."
- **Duration:** 1–14 days (config: `club.max_duration_days`)
- **Frequency limit:** 1 active club per instance at a time (prevents spam)
- **Credits:** flat bonus only (not a multiplier, consistent with §9.7.1 amendment)
- **Trust coupling:** clubs are admin/vanguard-created, so trust level coupling is irrelevant — a trust TL4+ vanguard or admin only.

---

## §19.x Trust × Taste Coupling (extend existing)

**Modification.** Trust levels (TL0–TL6, behavior-only) gain an optional taste-alignment gate per level.

```yaml
[trust]
taste_coupling = [
  { level: 1, min_resonance: 0.0 },
  { level: 2, min_resonance: 0.0 },
  { level: 3, min_resonance: 0.0 },
  { level: 4, min_resonance: 0.3 },
  { level: 5, min_resonance: 0.5 },
  { level: 6, min_resonance: 0.0 },  # admin level, bypasses coupling
]
```

- `min_resonance: 0.0` = no taste requirement for that level
- A user with sufficient behavior records but insufficient resonance stalls at the highest level their resonance supports
- **Preset:** `taste_coupling = "none"` sets all to 0.0 (default)
- **Never disclosed:** The resonance requirement is never shown to the user. If they can't advance, the UI shows their normal behavior-based progress, which will appear "stuck" without explaining why. This preserves foundational protection "admin taste invisible."

**Config level:** per-instance, per-trust-level. A democratic instance sets all to 0.0. A curated boutique sets TL4+ high.

---

## §20.3 Economy — Standing Bounties, Flexible Bounties, Referrals

### §20.3.1 Standing Bounties (extend existing bounty infra)

Admin defines criteria; when a work matches, a bounty is auto-created and auto-fulfilled.

```yaml
[[bounty_rules]]
name = "Slow-burn with payoff"
criteria_tags = ["slow burn", "enemies to lovers"]
min_words = 20000
min_admin_rating = 4.0
reward_bp = 20000           # 200 credits in base points
enabled = true
```

**Trigger flow:**
1. Admin rates a work ≥ `min_admin_rating`
2. System checks work's tags/word count against all enabled rules
3. Matching rules create a bounty with `status = auto_fulfilled`, credited to work author
4. Admin receives notification of auto-payout (can audit)

**Vanguard provisional flow (config: `vanguard_provisional_bounties`):**
- If enabled, when 3+ Vanguard users bookmark or rate a work highly (threshold configurable), a **provisional** bounty is created with `status = pending_admin_review`
- Admin has 14 days to confirm (bounty fulfilled) or reject (bounty voided)
- No response: auto-reject (fail-closed)

**Anti-gaming:** `min_admin_rating` requires actual admin rating, not tag alone. If a rule has only tag criteria (no rating requirement), it can be gamed — admin should always include a rating floor or word count floor.

### §20.3.2 Flexible Bounty Types (extend existing bounty table)

Generalize beyond admin-funded bounties:

| Type | Funded by | Claimed by | Description |
|------|-----------|------------|-------------|
| **Standard** | Admin | Any user (author or reader who fulfills) | Current behavior |
| **Crowdfunded** | Any user(s), pooled | Any user | Multiple users contribute credits; bounty activates when fully funded |
| **Reverse** | Reader (pays credits) | Author who writes matching work | "I'll pay X credits if someone writes Y" |
| **Collaborative** | Admin or user | Multiple users (split payout) | Multi-part work where each claimant earns a share |

**Config:** `bounties.allowed_types = ["standard", "crowdfunded", "reverse"]` (collaborative deferred).

### §20.3.3 Referral System (new subsection)

```yaml
[referrals]
enabled = true
tier1_signup = 10                # inviter earns when invitee registers
tier2_first_post = 10            # inviter earns when invitee posts first work
tier2_taste_bonus = 30           # bonus if first post aligns ≥ taste_threshold
tier3_retention = 50             # inviter earns when invitee reaches 30-day retention
tier3_taste_bonus = 50           # bonus if invitee resonance ≥ taste_threshold at day 30
tier4_high_resonance = 100       # inviter earns when invitee resonance ≥ 0.7
taste_threshold = 0.5            # alignment needed for taste bonuses
```

**All tier amounts configurable. Taste bonuses are the key design:** base payouts are modest, taste-aligned bonuses are generous. This trains referrers to recruit aligned users and writers.

**Anti-gaming:** Tier 3+ requires 30-day retention with minimum activity threshold (≥3 works read, ≥1 comment posted). Same-IP referral clusters flagged for review. Tier 4 requires actual resonance computation (which requires the invitee to have genuine reading/rating history).

**Referral link:** `?ref=<code>` on registration endpoint. Cookie-based fallback.

**Analytics:** `GET /api/v1/me/referrals` returns referral link, counts, tier breakdown, credits earned. RequireSession.

---

## §16.17 Taste Resonance Score (NEW)

**New section.** The hidden per-user alignment metric that powers Taste-Weighted Signals, Vanguard selection, Taste notifications, and Trust × Taste coupling.

**Computation (per account):**

```
resonance = weighted_sum(
    bookmark_overlap,        # Jaccard similarity of user vs admin exemplars (weight: 0.30)
    rating_correlation,       # correlation of shared ratings (weight: 0.25)
    completion_alignment,     # fraction of admin-liked works this user finished (weight: 0.25)
    reading_time_ratio,       # time on admin-aligned works / total reading time (weight: 0.20)
)
```

Weights are instance-configurable. Clamped to 0.0–1.0.

**Update cadence:**
- **Incremental:** on each bookmark, rating, or completion event, recompute the affected component only and update a running delta (near-real-time)
- **Full recompute:** weekly batch job that recalculates all components from scratch (correction pass)
- New accounts with insufficient signal (< 5 bookmarks AND < 5 ratings): resonance = 0.0, signals fall back to `egalitarian` mode until threshold met (see §16.18 cold start)

**Visibility:** Never shown to any user except the account owner via `GET /api/v1/me/taste-resonance`. Even then, shown as a qualitative label (e.g., "Aligned: Strong" for >0.7) rather than a raw number, to discourage optimization. The admin sees all users' resonance in admin panel.

**Instance without taste profile:** resonance computation disabled. Signal mode defaults to `egalitarian`. Trust coupling defaults to `none`.

---

## §16.18 Taste Vanguard Role (NEW)

**New section.** The curatorial layer: top-aligned users who act as the admin's taste scouts.

**Selection method (config: `vanguard.method`):**

| Method | Description | Default |
|--------|-------------|---------|
| `resonance_threshold` | Top N% by resonance (config: `vanguard.threshold_percent = 10`) | ✓ when taste profile exists |
| `admin_appointment` | Admin hand-picks users | |
| `community_election` | Users nominate and vote (quarterly) | |
| `contribution_volume` | Top curators by bookmark/review volume | |
| `hybrid` | Weighted combination | |

Weekly batch job applies the selection method. Admin can override at any time (grant/revoke individually).

**Vanguard permissions:**

| Permission | Config default | Notes |
|------------|----------------|-------|
| Pin works to fandom pages | `vanguard.can_pin = true` | Temporary (30 days), logged, admin can revoke |
| Nominate works for admin review | `vanguard.can_nominate = true` | Enters admin's review queue, not auto-approved |
| Vanguard Picks shelf (front page) | `vanguard.picks_shelf = true` | Public shelf of vanguard-pinned works |
| Create reading clubs | `vanguard.can_create_clubs = true` | See §17.6 |
| Provisional bounty triggering | See §20.3.1 | Config: `vanguard_provisional_bounties` |
| Post bounties at reduced cost | `vanguard.bounty_discount = 0.5` | 50% cost reduction on bounty creation |

**Profile badge:** visible to all users when `vanguard.public_badge = true`. Badge says "Vanguard" — never reveals *why* (i.e., never mentions taste/resonance/admin).

**Instance without taste profile:** Vanguard selection defaults to `contribution_volume` or `admin_appointment`. Resonance-based selection requires a taste profile.

---

## §0.6 Instance Presets (NEW)

**New section.** Bundles of configuration defaults so instance operators don't tune 40 parameters individually. Selectable at setup, individually overridable after.

| Preset | Taste gravity | Diversity injection | Signal mode | Trust coupling | Vanguard method | Topic bonuses |
|--------|--------------|-------------------|-------------|---------------|----------------|---------------|
| **Curated Boutique** | 0.8 | 5% | taste_weighted | tight | resonance_threshold | enabled |
| **Open Library** | 0.0 | 30% | egalitarian | none | community_election | disabled |
| **Admin's Garden** | 0.95 | 0% | admin_only | tight | admin_appointment | enabled |
| **Genre Haven** | 0.6 | 10% | taste_weighted | loose | contribution_volume | enabled (specific tags) |
| **Experimental Lab** | 0.5 (dynamic) | 25% | taste_weighted | loose | hybrid | enabled |

After selecting a preset, every parameter remains individually configurable. The preset is just a starting point.

```yaml
[instance]
preset = "curated_boutique"       # open_library | curated_boutique | admin_garden | genre_haven | experimental_lab | custom
```

Config override: any parameter set in the config file after `preset` takes precedence.

---

## §0.4.1 Anti-Echo-Chamber Valve (extend §0.4)

**Add to Instance Topics / Taste Profile section.** Directly serves priority 4: "admin-aligned but never monothematic."

```yaml
[signals]
diversity_injection_percent = 10   # 0–50, % of every discovery surface reserved for taste-distant content
```

- `0` = pure monoculture (admin's garden, no diversity)
- `10–15` = healthy default (admin-aligned with occasional surprises)
- `30+` = open platform (minimal gravity)

**Mechanism:** On every discovery surface (home, search results, tag pages, fandom pages), X% of slots are filled by a diversity-aware selector that deliberately selects works farthest from the admin's taste profile while still meeting minimum quality thresholds (e.g., ≥100 words, not adult-restricted for adult users, etc.). This is invisible — the diversity-injected works look identical in the UI to taste-aligned works.

**Rationale:** Pure taste gravity creates monoculture. This ensures the archive can surprise even the admin. When the admin engages positively with a taste-distant work (high rating, long reading time), the taste profile can expand organically (see §16.19).

---

## §16.19 Taste Probes (NEW)

**New section.** Active exploration mechanism that prevents the taste profile from ossifying.

**For the admin:** Periodically (config: `taste_probes.frequency = weekly`), the admin's discover page surfaces works that are *adjacent to but slightly outside* their taste vector (config: `taste_probes.distance = 0.1–0.3`). The system tracks admin engagement:

- **Positive engagement** (rating ≥4, reading >80%, bookmarking): the taste profile expands — the relevant dimension's `admin_target` shifts slightly toward the probed work's characteristics
- **Negative engagement** (quick bounce <30 seconds, rating ≤2): the boundary reinforces — no expansion, and future probes in that direction are suppressed

**For all users:** Works from the diversity injection (§0.4.1) serve as taste probes. User engagement with diversity-injected works feeds into resonance computation, so a user who consistently engages positively with taste-distant works sees their resonance drop (and vice versa) — correctly reflecting their alignment.

**Config:**

```yaml
[taste_probes]
frequency = "weekly"        # weekly | monthly | never
distance = 0.2              # 0.1 (adjacent) to 0.5 (far)
max_probes_per_session = 3  # limits how many probe works appear per admin session
auto_expand = true          # admin's positive engagement adjusts taste profile
```

**Never disclosed:** Probes are not labeled. They appear as normal recommendations. The admin's taste profile changes are logged in admin panel but not announced.

---

## §0.4.2 Onboarding Taste Quiz (extend §0.4)

**Add to Instance Topics / Taste Profile section.** Solves the cold-start problem for both new users and the resonance computation.

When a new user registers (and the instance has a taste profile), an optional onboarding quiz presents 10–15 work blurbs pre-selected by the admin (config: `taste_profile.quiz_works`). The user picks 3–5 they'd be interested in.

**Result:** The system computes an initial taste vector estimate from quiz selections, providing immediate resonance signal before any reading history exists. This enables:
- Taste-weighted notifications from day 1
- Meaningful discover page ordering from the first visit
- Initial resonance for Trust × Taste coupling (though the threshold should be generous for quiz-only data)

**Config:**

```yaml
[taste_profile.quiz]
enabled = true
min_works_shown = 10
min_works_picked = 3
skippable = true              # user can skip; falls back to egalitarian mode
```

**Never disclosed:** The quiz is presented as "help us show you better recommendations" — never "help us determine your alignment with admin taste."

---

## Dynamic Tag Gravity (extend §0.4.6 / M11-07..12)

**Modification.** The existing `apply_theme_gravity` (boost/suppress tags) becomes dynamic:

- Every tag maintains a rolling **gravity score** based on admin engagement over the last 90 days (reading time, ratings, bookmarks on works with that tag)
- Tags the admin actively engages with: gravity score rises, archive-wide boost applied
- Tags the admin ignores: gravity score decays toward neutral (0)
- Tags on admin anti-exemplar works: gravity score goes negative, suppression applied
- Admin can manually pin tags to boost/suppress regardless of engagement (existing behavior preserved)

**Config:**

```yaml
[theme]
dynamic_gravity = true
gravity_decay_days = 90
gravity_max_boost = 1.5       # max multiplier from dynamic tag gravity
gravity_max_suppress = 0.5    # min multiplier (never zero — content still findable)
```

**No taste profile:** dynamic gravity disabled, only manual boost/suppress applies.

---

## Implementation Priority

| # | Feature | Spec Section | Serves Goal Directly | Effort |
|---|---------|-------------|---------------------|--------|
| 1 | Taste-Weighted Signals | §9.7.3 | **Core gravity engine** — without this, nothing else matters | M |
| 2 | Multi-dimensional Taste Profile | §0.4 | **Foundation** — replaces scalar, enables everything | M |
| 3 | Taste Resonance Score | §16.17 | **Foundation** — powers signals, vanguard, trust coupling | M |
| 4 | Anti-Echo-Chamber Valve | §0.4.1 | **Prevents monoculture** — serves priority 4 | S |
| 5 | Onboarding Taste Quiz | §0.4.2 | **Solves cold start** — new users get gravity from day 1 | S |
| 6 | Taste Probes | §16.19 | **Keeps taste evolving** — prevents ossification | S |
| 7 | Fic Lifecycle Incentives | §9.8 | **Completion problem** — abandoned fics hurt enjoyment | S |
| 8 | Taste-Weighted Notifications | §9.9 | **Highest-leverage engagement driver** | M |
| 9 | Standing Bounties | §20.3.1 | **Removes admin bottleneck** from bounty economy | S |
| 10 | Flexible Bounties | §20.3.2 | **Community bounty economy** | M |
| 11 | Taste Vanguard Role | §16.18 | **Curatorial layer** — scouts for admin taste | M |
| 12 | Referral System | §20.3.3 | **Growth with taste alignment** | M |
| 13 | Trust × Taste Coupling | §19.x | **Governance integration** | S |
| 14 | Dynamic Tag Gravity | §0.4.6 | **Extends existing** theme gravity | S |
| 15 | Instance Presets | §0.6 | **Flexibility** — makes all configs approachable | S |
| 16 | Author Matchmaking | §14.5 | **Shapes supply** not just demand | S |
| 17 | Reading Clubs | §17.6 | **Manual gravity override** | S |
| 18 | Streak flat bonuses | §9.7.1 | **Engagement without diluting signal** | XS |

**M** = medium (multi-file, schema + domain + API), **S** = small (single area), **XS** = trivial.

**Phased order:** 1→2→3 are the foundation (ship together). 4→5→6 are the health layer. 7→8→9 are the engagement layer. 10→11→12→13 are the community layer. 14→15→16→17→18 are incremental improvements.

---

## Explicitly Dismissed

| Proposal | Reason |
|----------|--------|
| Streak multiplier on all actions | Dilutes taste signal; replaced with flat milestone bonuses (§9.7.1) |
| Scalar resonance as primary model | Insufficient; replaced with multi-dimensional taste profile (§0.4). Scalar remains as a derived projection. |
| Resonance visible as raw number | Violates "admin taste invisible"; shown only as qualitative label to account owner (§16.17) |
| Vanguard private forum | Forum infrastructure not yet built for private channels; deferred |
| Influencer/Ambassador analytics dashboard | Requires traffic analytics not yet present; badges ship, dashboard deferred |
| Custom referral landing pages | Requires frontend work; deferred |
| Embeddable fic widgets, OG images | Requires frontend/image generation; deferred |
| Collaborative bounty type | Complexity; standard/crowdfunded/reverse sufficient initially |
| Trust coupling affecting TL6 | TL6 is admin level; always bypasses taste coupling |
