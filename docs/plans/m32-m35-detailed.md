### 15a.2 Milestone 32 (repo) — Typed votes, budgets, meta-moderation, karma
Spec §35.2. Depends on: M31 (work reactions), M14 (trust levels).

Migration `0039_forum_votes.sql`:
```sql
-- Taxonomy per category (default for categories is NULL = use instance default).
CREATE TABLE forum_vote_types (
    id TEXT PRIMARY KEY,
    label TEXT NOT NULL UNIQUE,
    category_scope TEXT,            -- NULL = instance-wide default
    weight REAL NOT NULL DEFAULT 1.0,
    cost INTEGER NOT NULL DEFAULT 1, -- budget cost (>=1; negatives >1)
    is_negative INTEGER NOT NULL DEFAULT 0
);
CREATE INDEX forum_vote_types_category ON forum_vote_types (category_scope);

-- One vote per pseud per post. weight_at_cast captures the caster's effective
-- weight at vote time (after meta-mod decay), so re-computation is unnecessary.
CREATE TABLE forum_votes (
    post_id TEXT NOT NULL,
    pseud TEXT NOT NULL,
    vote_type TEXT NOT NULL,
    weight_at_cast REAL NOT NULL DEFAULT 1.0,
    created_at TEXT NOT NULL,
    updated_at TEXT NOT NULL,
    PRIMARY KEY (post_id, pseud)
);
CREATE INDEX forum_votes_type ON forum_votes (post_id, vote_type);

-- Meta-moderation of votes. TL4+ spend meta-mod points to flag fair/unfair.
CREATE TABLE forum_meta_votes (
    vote_post_id TEXT NOT NULL,
    vote_pseud TEXT NOT NULL,
    pseud TEXT NOT NULL,            -- the meta-mod who flagged
    fair INTEGER NOT NULL,          -- 1 fair, 0 unfair
    created_at TEXT NOT NULL,
    PRIMARY KEY (vote_post_id, vote_pseud, pseud),
    FOREIGN KEY (vote_post_id, vote_pseud) REFERENCES forum_votes(post_id, pseud)
);

-- Karma per pseud: derived from received votes, decayed on inactivity.
-- Display-only — never gates trust, moderation, ranking, or credits.
CREATE TABLE forum_karma (
    pseud TEXT PRIMARY KEY,
    karma REAL NOT NULL DEFAULT 0.0,
    updated_at TEXT NOT NULL
);

-- [forum] config additions: vote_budget_base, vote_budget_tl_multiplier,
-- meta_mod_tl_required (default 4), karma_decay_monthly (default 0.05),
-- karma_inactivity_threshold_days (default 30)
```

Domain module `forum_votes.rs`:
- `ForumVoteType` struct: `id`, `label`, `category_scope`, `weight`, `cost`, `is_negative`
- `ForumVote` struct: `post_id`, `pseud`, `vote_type`, `weight_at_cast`, `created_at`
- `VoteBudget` struct: `remaining`, `reset_at`, `trust_level`
- `Karma` struct: `pseud`, `karma`, `updated_at`
- Pure functions:
  - `budget_remaining(votes_cast_24h, trust_level, config) -> VoteBudget`
  - `vote_weight(pseud, meta_vote_history) -> f64` (decays with consistent unfair flags)
  - `karma_after_decay(karma, months_inactive, config) -> f64`
  - `can_meta_moderate(user_tl, config) -> bool`

DB layer `forum_votes.rs`:
- `list_vote_types(db, category_id) -> Vec<ForumVoteType>`
- `set_vote(db, post_id, pseud, vote_type) -> Result<ReactionOutcome>` (budget-checked)
- `delete_vote(db, post_id, pseud) -> Result<()>`
- `vote_counts(db, post_id) -> HashMap<String, i64>`
- `vote_by(db, post_id, pseud) -> Option<String>`
- `cast_meta_vote(db, vote_post_id, vote_pseud, pseud, fair) -> Result<()>`
- `vote_weight_for(db, pseud) -> f64` (computes effective weight from meta-vote history)
- `budget_remaining(db, pseud, now) -> VoteBudget`
- `karma_for(db, pseud) -> Karma`
- `decay_karma_batch(db, now) -> Result<()>` (background job: UPDATE forum_karma SET karma = karma * (1 - decay) WHERE updated_at < threshold)

HTTP routes (add to `community.rs` router):
```
POST   /forum/posts/{id}/vote        -> post_vote (RequirePseud)
DELETE /forum/posts/{id}/vote        -> delete_vote (RequirePseud)
GET    /forum/posts/{id}/votes       -> get_votes (RequireSession; tier-aware)
POST   /forum/votes/{id}/meta        -> post_meta_vote (RequirePseud, TL4+)
GET    /me/vote-budget               -> get_budget (RequirePseud)
GET    /forum/karma                  -> get_own_karma (RequirePseud)
GET    /forum/users/{id}/karma       -> get_user_karma (public)
```

Route handlers:
- `post_vote`: parse body `{ "vote_type": "insightful" }`; check budget (reject 429 BUDGET_EXHAUSTED if 0); check taxonomy validity for post's category; call `set_vote`; return `{ "outcome": "cast|changed|retracted" }`
- `delete_vote`: retract; always succeeds (idempotent)
- `get_votes`: return `{ "counts": {...}, "mine": "insightful" }` — individual votes only if author/mod/meta-mod
- `post_meta_vote`: body `{ "fair": true }`; check TL4+; call `cast_meta_vote`; recompute caster's vote_weight
- `get_budget`: return `{ "remaining": 7, "reset_at": "...", "trust_level": 3 }`
- `get_own_karma`: return `{ "karma": 123.4 }`
- `get_user_karma`: public profile karma display

Acceptance tests (create `milestone_32.rs`):
- `budget_rejects_exhausted_voter` (429)
- `meta_mod_decay_reduces_weight` (check weight_at_cast drops after unfair flags)
- `transparency_hides_individual_votes_from_strangers` (anonymous by default)
- `karma_decays_on_inactivity` (simulate 31 days, verify *= 0.95)
- `category_taxonomy_overrides_default` (Critique category has constructive/harsh-but-fair)
- `negative_vote_costs_more_budget` (disagree costs 3, costs tracked correctly)

Frontend:
- Extend `ForumTopic.svelte` with vote bar per post (show counts, allow voting)
- Add `fetchVoteTypes`, `postVote`, `deleteVote`, `fetchVotes`, `fetchBudget`, `fetchKarma` to `api.ts`
- New component `ForumVoteBar.svelte` (similar to ReactionBar but for posts)
- `VoteBudget.svelte` badge showing remaining votes
- `KarmaBadge.svelte` on pseud profiles

### 15a.3 Milestone 33 (repo) — Thread modes for creative work
Spec §35.3. Depends on: M31 (linked topics), M32 (votes inside prompt/AMA).

Migration `0040_thread_modes.sql`:
```sql
ALTER TABLE forum_topics ADD COLUMN mode TEXT NOT NULL DEFAULT 'plain';
-- plain | ama | reading_group | critique | wiki_pin | collab_fic | prompt | character_voice

CREATE TABLE topic_schedules (
    id TEXT PRIMARY KEY,
    topic_id TEXT NOT NULL,
    position INTEGER NOT NULL,
    title TEXT NOT NULL,
    unlocks_at TEXT NOT NULL,
    chapter_start INTEGER,
    chapter_end INTEGER
);
CREATE INDEX topic_schedules_topic ON topic_schedules (topic_id);

CREATE TABLE topic_wiki_pins (
    id TEXT PRIMARY KEY,
    topic_id TEXT NOT NULL UNIQUE,
    post_id TEXT,
    revision INTEGER NOT NULL DEFAULT 0,
    approved_by TEXT,
    approved_at TEXT
);

CREATE TABLE critique_queue (
    id TEXT PRIMARY KEY,
    topic_id TEXT NOT NULL,
    pseud TEXT NOT NULL,
    position INTEGER NOT NULL,
    posted_at TEXT,
    UNIQUE (topic_id, pseud)
);
CREATE INDEX critique_queue_topic ON critique_queue (topic_id);

CREATE TABLE prompt_posts (
    id TEXT PRIMARY KEY,
    topic_id TEXT NOT NULL UNIQUE,
    prompt_date TEXT NOT NULL,
    winner_pseud TEXT
);

-- Character voice: posts reference a character from the author's work.
ALTER TABLE forum_posts ADD COLUMN character_id TEXT;
```

Domain: `forum_thread_modes.rs`
- `ThreadMode` enum: `Plain`, `Ama`, `ReadingGroup`, `Critique`, `WikiPin`, `CollabFic`, `Prompt`, `CharacterVoice`
- `parse()` and `as_str()` 
- `TopicSchedule`, `WikiPin`, `CritiqueQueue`, `PromptPost` structs
- Pure: `visible_schedules(schedules, now) -> Vec` (filter unlocks_at <= now), `next_critique_turn(queue) -> Option<String>`

DB layer `forum_thread_modes.rs`:
- `set_topic_mode(db, topic_id, mode) -> Result<bool>`
- `schedule_section(db, topic_id, position, title, unlocks_at, chapter_range) -> Result<()>`
- `visible_sections(db, topic_id, now) -> Vec<TopicSchedule>`
- `submit_wiki_edit(db, topic_id, post_id, editor_pseud) -> Result<WikiPin>` (pending approval)
- `approve_wiki_edit(db, topic_id, approver_pseud) -> Result<()>`
- `join_critique_queue(db, topic_id, pseud) -> Result<usize>` (returns position)
- `submit_critique_post(db, topic_id, pseud) -> Result<()>` (enforces turn order)
- `compile_collab_fic(db, topic_id) -> String` (stitches posts in order into one text)
- `promote_to_work(db, topic_id, owner_pseud) -> Result<WorkId>` (creates work from thread, sets thread read-only)
- `create_prompt_post(db, topic_id, prompt_date) -> Result<String>` (called by prompt scheduler)

HTTP routes:
```
PUT    /topics/{id}/mode              -> put_topic_mode (author)
POST   /topics/{id}/sections         -> post_schedule_section (author)
GET    /topics/{id}/sections         -> get_sections (RequireSession; time-filtered)
POST   /topics/{id}/wiki             -> submit_wiki_edit (RequirePseud)
POST   /topics/{id}/wiki/approve     -> approve_wiki_edit (author/mod)
POST   /topics/{id}/critique/join    -> join_critique_queue (RequirePseud)
POST   /topics/{id}/critique/submit  -> submit_critique (RequirePseud; turn-checked)
POST   /topics/{id}/compile          -> compile_collab_fic (RequirePseud; returns text)
POST   /topics/{id}/promote          -> promote_to_work (RequirePseud; creates work)
```

Acceptance tests (`milestone_33.rs`):
- `reading_group_hides_future_sections` (404 on locked section)
- `critique_enforces_turn_order` (422 WRONG_TURN when out-of-order)
- `wiki_pin_invisible_until_approved` (draft state vs published)
- `compile_stitches_posts_in_order` (verify output text)
- `promote_to_work_creates_readable_work` (work exists with chapters)
- `ama_sorts_questions_to_top` (check ordering)
- `character_voice_post_resolves_to_user_for_mod` (block/mute still works)

Frontend:
- `ThreadModePicker.svelte` in topic author controls
- `ReadingGroupProgress.svelte` (schedule sidebar)
- `WikiPin.svelte` (collaborative post with edit/approve UI)
- `CritiqueQueue.svelte` (turn display)
- `CollabCompileButton.svelte` (on collab_fic topics)
- `CharacterVoiceBadge.svelte` (avatar + name rendering)

### 15a.4 Milestone 34 (repo) — Spoilers, warnings, readability
Spec §35.4. Depends on: M12 (post drafts, scheduled posts tables exist — verify).

Migration `0041_spoilers_warnings.sql`:
```sql
-- Per-topic spoiler scope: "spoilers through chapter 12"
ALTER TABLE forum_topics ADD COLUMN spoiler_scope_chapter INTEGER;

-- Collapse long posts: user preference (NULL = use default 800)
ALTER TABLE forum_posts ADD COLUMN fold_at_word_count INTEGER;

-- Content warnings taxonomy (reuses §15.16 vocabulary)
CREATE TABLE content_warnings (
    id TEXT PRIMARY KEY,
    label TEXT NOT NULL UNIQUE,
    severity INTEGER NOT NULL DEFAULT 0  -- 0=info, 1=warn, 2=severe
);

CREATE TABLE post_content_warnings (
    post_id TEXT NOT NULL,
    warning_id TEXT NOT NULL,
    custom_text TEXT,
    PRIMARY KEY (post_id, warning_id)
);

-- Scheduled posts (if not already in M12 schema — check before adding)
ALTER TABLE forum_posts ADD COLUMN scheduled_at TEXT;
ALTER TABLE forum_posts ADD COLUMN published_at TEXT;  -- when actually published
CREATE INDEX forum_posts_scheduled ON forum_posts (scheduled_at) WHERE scheduled_at IS NOT NULL;

-- Post drafts (if not already in M12 — verify)
-- M12 has post_drafts table; if not, create:
-- CREATE TABLE post_drafts (id TEXT PRIMARY KEY, author_pseud TEXT NOT NULL, topic_id TEXT NOT NULL, body TEXT NOT NULL, saved_at TEXT NOT NULL, UNIQUE (author_pseud, topic_id));
```

DB layer `forum_spoilers.rs`:
- `topic_spoiler_scope(db, topic_id) -> Option<i32>`
- `set_spoiler_scope(db, topic_id, chapter) -> Result<()>`
- `post_warnings(db, post_id) -> Vec<ContentWarning>`
- `add_warning(db, post_id, warning_id, custom_text) -> Result<()>>`
- `fold_at(user_prefs, post_word_count) -> Option<usize>`
- `save_draft(db, author_pseud, topic_id, body) -> Result<()>` (UPSERT)
- `load_draft(db, author_pseud, topic_id) -> Option<String>`
- `publish_scheduled_posts(db, now) -> Result<Vec<String>>` (returns published post ids)

Acceptance tests (`milestone_34.rs`):
- `spoiler_block_hidden_from_screen_reader` (verify aria + content visibility)
- `content_warning_blur_follows_viewer_prefs` (different users see different states)
- `draft_survives_hard_reload` (save → reload → resume)
- `scheduled_post_publishes_once_at_its_time` (idempotent)
- `fold_never_hides_first_screenful` (measure rendered height)

Frontend:
- `SpoilerBlock.svelte` (blur + click-to-reveal, a11y-correct)
- `ContentWarningBar.svelte` (per-type prefs gate)
- `PostComposer.svelte` with autosave (30s interval)
- `ReadingTimeBadge.svelte` from word count

### 15a.5 Milestone 35 (repo) — Discovery, health, UX, federation
Spec §35.5. Depends on: M31–M34, M18 (federation base), M24 (moderation).

Migration `0042_discovery.sql`:
```sql
-- Thread summaries: optional AI, deterministic abstention
ALTER TABLE forum_topics ADD COLUMN summary_text TEXT;
ALTER TABLE forum_topics ADD COLUMN summary_revision INTEGER DEFAULT 0;

-- Semantic search (PostgreSQL only; use IF NOT EXISTS for SQLite compat)
-- For PostgreSQL: CREATE EXTENSION IF NOT EXISTS vector; ALTER TABLE forum_topics ADD COLUMN embedding vector(768);

-- Federation scope per topic
ALTER TABLE forum_topics ADD COLUMN federation_scope TEXT NOT NULL DEFAULT 'public';
-- public | local | unlisted

-- Thread forking audit
ALTER TABLE forum_posts ADD COLUMN original_topic_id TEXT;
ALTER TABLE forum_posts ADD COLUMN forked_to_topic_id TEXT;

-- Slow mode
ALTER TABLE forum_topics ADD COLUMN slow_mode_seconds INTEGER DEFAULT 0;

-- Featured posts (best-of digest)
ALTER TABLE forum_posts ADD COLUMN featured INTEGER NOT NULL DEFAULT 0;
ALTER TABLE forum_posts ADD COLUMN featured_by TEXT;
ALTER TABLE forum_posts ADD COLUMN featured_at TEXT;

-- Activity history (daily reply counts, JSON array)
ALTER TABLE forum_topics ADD COLUMN activity_history TEXT NOT NULL DEFAULT '[]';

-- Moderation ladder state
ALTER TABLE forum_posts ADD COLUMN moderation_action TEXT;
-- NULL | verbal_warning | post_throttle | read_only | forum_ban | site_ban
ALTER TABLE forum_posts ADD COLUMN moderation_expires_at TEXT;
ALTER TABLE forum_posts ADD COLUMN moderation_reason TEXT;
```

Migration `0043_federation.sql` (if not covered by M18):
```sql
-- Remote profile cache
CREATE TABLE remote_profiles (
    actor_id TEXT PRIMARY KEY,
    instance_host TEXT NOT NULL,
    display_name TEXT,
    bio TEXT,
    avatar_url TEXT,
    cached_at TEXT NOT NULL,
    ttl_seconds INTEGER NOT NULL DEFAULT 3600
);

-- Instance reputation
CREATE TABLE instance_reputation (
    instance_host TEXT PRIMARY KEY,
    spam_rate REAL NOT NULL DEFAULT 0.0,
    report_rate REAL NOT NULL DEFAULT 0.0,
    avg_takedown_seconds REAL NOT NULL DEFAULT 0.0,
    auto_defederated INTEGER NOT NULL DEFAULT 0,
    updated_at TEXT NOT NULL
);

-- Federated polls (ActivityPub Question)
CREATE TABLE forum_poll_votes (
    poll_id TEXT NOT NULL,
    actor_id TEXT NOT NULL,  -- can be remote actor IRI
    option_index INTEGER NOT NULL,
    created_at TEXT NOT NULL,
    PRIMARY KEY (poll_id, actor_id)
);
```

DB layer `forum_discovery.rs`:
- `set_summary(db, topic_id, summary, revision) -> Result<()>`
- `get_summary(db, topic_id) -> Option<String>`
- `set_federation_scope(db, topic_id, scope) -> Result<()>`
- `fork_topic(db, source_topic_id, post_range, new_title, actor) -> Result<String>` (creates new topic, tombstones originals)
- `feature_post(db, post_id, curator_pseud) -> Result<()>>`
- `set_slow_mode(db, topic_id, seconds) -> Result<()>>`
- `record_activity(db, topic_id, date, reply_count) -> Result<()>` (appends to activity_history JSON)
- `log_moderation(db, post_id, action, reason, expires_at) -> Result<()>>`
- `remote_profile(db, actor_id) -> Option<RemoteProfile>` (with TTL check)
- `instance_reputation(db, host) -> InstanceReputation`

HTTP routes:
```
GET    /forum/topics/{id}/summary         -> get_summary (public)
PUT    /forum/topics/{id}/summary         -> put_summary (author/mod; optional AI)
PUT    /forum/topics/{id}/federation-scope -> put_fed_scope (author/mod)
POST   /forum/topics/{id}/fork            -> fork_topic (TL4+ or mod)
POST   /forum/posts/{id}/feature          -> feature_post (TL5+ or curator)
PUT    /forum/topics/{id}/slow-mode       -> put_slow_mode (author/mod)
GET    /forum/digest                      -> get_digest (featured posts)
GET    /forum/health                      -> get_health_dashboard (admin)
GET    /forum/search                      -> semantic_search (FTS + pgvector)
GET    /forum/remote-profiles/{id}        -> get_remote_profile (cached)
```

Acceptance tests (`milestone_35.rs`):
- `local_topic_absent_from_federation_outbound` (verify ActivityPub payload)
- `forking_preserves_audit_trail` (original_topic_id set, tombstone present)
- `health_dashboard_respects_privacy_ceilings` (no individual user data)
- `slow_mode_enforces_rate_limit` (429 SLOW_MODE)
- `federated_poll_counts_one_actor_once` (duplicate actor_id rejected)
- `remote_profile_cache_expires` (TTL respected)

Frontend:
- `ThreadSummary.svelte` (collapsible TL;DR)
- `ForkButton.svelte` (mod/TL4+ only)
- `FeatureBadge.svelte` (on featured posts)
- `DigestView.svelte` (best-of page)
- `SlowModeIndicator.svelte` (shows cooldown)
- `RemoteProfileCard.svelte` (with home instance badge)
- `KeyboardShortcuts.svelte` (? modal)
- `SearchBar.svelte` with semantic toggle

### 15a.6 Cross-cutting concerns

- **Config section `[forum]`**: `work_discussion_default`, `vote_budget_base`, `vote_budget_tl_multiplier`, `meta_mod_tl_required`, `karma_decay_monthly`, `karma_inactivity_threshold_days`, `slow_mode_default_seconds`, `fold_default_word_count`, `summary_min_replies`, `summary_regenerate_every`
- **Worker jobs**: `decay_karma` (daily), `publish_scheduled_posts` (every 60s), `generate_summaries` (every 5min), `update_instance_reputation` (hourly)
- **Federation**: topic federation_scope travels in ActivityPub payload; local topics never federated
- **Privacy**: health dashboard aggregates only (§24.3); no individual user data exposed
- **Trust**: nothing in §35 promotes/demotes trust (§19 owns that ladder)

### 15a.7 Implementation order

1. M32 first (votes are the signal inside M33 prompt/AMA modes)
2. M33 next (thread modes build on M31 links + M32 votes)
3. M34 can be parallel with M33 (independent surfaces)
4. M35 last (builds on all of the above + M18 federation)

Each milestone: migration → domain → db → routes → tests → frontend → E2E.
