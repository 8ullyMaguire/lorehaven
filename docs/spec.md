# Lorehaven: Complete Implementation Specification

This is a dependency-ordered specification for building the entire platform—not a prototype. It defines architecture, data structures, workflows, implementation milestones, and verification requirements.

This document specifies intended behavior. It does not claim that any feature, adapter, integration, or performance target has already been implemented or verified.

**Amendments:** The taste-gravitational system (multi-dimensional taste profile, taste-weighted signals, resonance score, vanguard role, instance presets, and related extensions) is specified in `docs/spec-amendments/taste-gravitational-system.md`, which amends §0.4, §0.4.6, §9.7, §9.8, §9.9, §14.5, §16.2, §16.17–16.19, §17.6, §19, and §20.3. Where this document and the amendment conflict, the amendment governs. Implementation plan: `docs/plans/taste-gravitational-system-v2.md` (supersedes v1).

---

# 0. Site Premise

## 0.1 In one sentence

Lorehaven is a self-hosted, self-governing fanfiction platform that lets users shape their own experience through extensions, grows the body of available fiction through frictionless importing and writing, keeps authors motivated through a positivity-first feedback culture, and uses trust-level and quorum-based community governance so the administrator can focus on curating taste rather than policing behavior.

## 0.2 Priority stack

When two goals conflict, the higher-numbered priority yields to the lower-numbered one.

1. **Fully customizable website** through installable themes, recommendation engines, feed composition, widgets, writing tools, challenge variants, and reader enhancements.
2. **Maximize available high-quality fiction** through aggressive importing, low-friction writing, effective discovery, and reader-behavior quality signals.
3. **Maximize positive feedback and suppress destructive negativity.** Only positive or constructive criticism reaches authors. Constructive critique requires opt-in.
4. **Grow admin-aligned fiction without becoming monothematic.** Private taste influence guided by deliberate diversity mechanisms.
5. **Trust-level self-governance with minimal administrator involvement.** Community moderates itself under stated policy.
6. **Quorum-based moderation and curation.** Significant decisions require multiple reviewers.
7. **Translate everything.** Interface fully translated; content translation available on demand.
8. **Revenue** through credits, subscriptions, and marketplace fees for resource-intensive operations.

## 0.3 Foundational protections

These are not priorities that yield to others. They constrain every priority above.

- Private libraries, drafts, reading history, source credentials, and pseud linkage stay private.
- Authors control their work. Readers control their data.
- Core reading, writing, publishing, and basic community participation remain free.
- Bounded resource use at every trust and payment level.
- Accessibility, child safety, and harassment protection are foundational.
- No purchased trust, purchased ranking, or purchased moderation authority.
- Credits and gamification rewards must never purchase trust, moderation authority, or search ranking.
- No advertising, no third-party trackers, no sponsored placement anywhere in the product.
- Third-party AI crawlers are refused by default (§24.14); a work's availability for AI training is the author's statement, not something the instance can enforce, and it is presented as such.
- The administrator's taste profile must never be visible, inferable, or hinted at through any user-facing label, multiplier name, or credit breakdown.
- Honest verification claims. Feature presence in this document is not evidence of implementation.

---

# 0.4 Instance Topics

An instance **may** declare the topics it is about — but is never required to.
An instance whose focus is a kink or sensitive subject does not have to say so.

Each topic has three properties:
- **`name`** — the operator's own label ("hurt/comfort", "omegaverse", …).
- **`public`** — `true` means the topic is shown publicly (landing page, meta API, leaderboard descriptions); `false` means it is used only internally for recommendations and credit bonuses, never named.
- **`bonus_credits`** — extra credits a reader earns for finishing a work in that topic (default 0).

An instance may declare **multiple topics**; a work may match several; the
bonus stacks per matching public topic. Private topics earn the same bonus
but are never surfaced. **No topics = topic-agnostic**; the baseline gamification rules in §9.7 apply universally, no bonuses.

## 0.4.1 Gamification effects of public topics

An extra completion-credit row in §9.7.3: "Finish a work in a public topic" — `bonus_credits` per matching public topic, once per work per pseud, with the same minimum time-on-page gate as §9.7.3.

Per-public-topic leaderboard category in §9.7.5 (metric: works finished in that topic). Only appears when the operator declares at least one public topic.

## 0.4.2 Anti-gaming

The topic completion bonus never applies to a work the reader imported or wrote themselves. Same minimum time-on-page, same pseud isolation, same unique-source dedup as §9.7.8. A reader cannot inflate a topic bonus by re-reading the same work.

## 0.4.3 Reporting and visibility

`/api/v1/meta` surfaces the instance's public topic names only — never counts,
never flags indicating which are kink-related. Private topics are entirely invisible on every public surface.

Instance operators who do not declare any topics behave identically to a
topic-agnostic instance. Kink-focused instances are the **default**; declaring
anything is the deliberate act.

## 0.4.4 Configuration

```toml
# lorehaven.toml
[site]
name = "Lorehaven"
topics = [
  { name = "hurt/comfort", public = true,  bonus_credits = 3 },
  { name = "omegaverse",   public = false, bonus_credits = 5 },
]
```

Each entry: `name: String`, `public: bool`, `bonus_credits: i64` (default 0).

## 0.4.5 Monetization disclosure

`/api/v1/meta` also surfaces the instance's monetization state — pool
sizes, cap, median, fee range, weights — exactly as §20.10.7 specifies.
The disclosure is the anti-corruption mechanism: public, always visible,
and never optional when billing is enabled.

## 0.4.6 Theme mode semantics

An instance's **theme mode** decides how strongly topic gravity steers discovery:

| Mode | Effect on discovery |
|------|-------------------|
| `generic` | No instance-wide topic gravity. Each reader's personal taste and §16's dials apply normally. Public topics still earn §0.4.1 gamification bonuses, but do not rank a work higher for anyone who has not opted into them. |
| `thematic` | (Default.) The instance's declared public topics provide a bounded upward nudge in §43.2's candidate blend. A reader's §16.5 dial attenuates this nudge; when the dial reaches zero the nudge is neutralised entirely unless the operator has locked the dial (§0.4.6). |
| `adaptive` | The instance's theme drifts toward what its long-term contributors actually read and write, bounded by `adaptive_max_drift_bp`. The operator's declared topics are the starting anchor; the nudge may never exceed the anchor's strength plus the drift bound. |

The mode is configured under `[theme]`. The default is `thematic` — an instance that declares topics is assumed to want them to matter.

**Dial lock.** When `allow_user_opt_out = false`, the §16.5 dial's lower bound becomes `theme_dial_floor` (default 10%) rather than zero. The reader may attenuate but never neutralise topic gravity. This is the operator's curation prerogative; the landing page states the instance's dial policy when it is locked.

**Influence sources.** The `influence_sources` list orders who shapes instance taste, from strongest to weakest signal:

| Kind | What it contributes |
|------|-------------------|
| `operator_topics` | The declared public topics' tag vectors (§0.4). |
| `admin_taste` | The §16.2 administrator taste profile. |
| `long_term_users` | A composite of accounts whose tenure ≥ `long_term_tenure_days` (default 90) and contribution count ≥ `long_term_min_contributions` (default 5). Uses §16.16.1 contribution events — not logins. The §16.15 cohort floor (≥ 5 distinct accounts) applies, so a small in-group cannot masquerade as the readership. |

Adaptive mode requires `long_term_users` in `influence_sources`; startup validation refuses a configuration that drifts without a drift source. Each source is independent — an operator may run thematic mode with only `operator_topics`, or add `admin_taste` for a sharper profile.

**Transparency.** `/api/v1/meta` surfaces the mode and influence sources (by kind only, not member identities). Adaptive mode additionally publishes a coarse histogram of topic-pull distribution: decile buckets of works by their topic-gravity score. This is enough to audit drift without enabling per-work gaming.

---

# 1. Ground Rules for Implementation

## 1.1 Completion means working behavior

A feature is complete only when:

- Its database migrations exist.
- Its backend behavior exists.
- Its permissions are enforced server-side.
- Its frontend uses the real backend.
- Its loading, empty, error, and success states exist.
- Relevant automated tests pass.
- Its documentation matches the implementation.

A screen containing mock data is not an implemented feature.

## 1.2 Track verification honestly

Maintain `docs/verification.md` with these statuses:

- Implemented and locally tested.
- Implemented and fixture tested.
- Implemented but not executed.
- External integration not live verified.
- Partially implemented.
- Unsupported.

Importer, extension, translation-provider, and payment-provider support use the same distinction. Never count a skeleton as a supported integration. Never promise adapter counts in advance.

## 1.3 Build vertical slices

Complete small end-to-end workflows before expanding them.

Priority slices:

> Install extension → configure → use → uninstall.

> Paste URL → private import → read → download → offline read.

> Publish chapter → reader appreciates → author sees positive feedback.

> Report negative comment → quorum review → hide or approve.

> Request translation → pay credits → review → publish.

Do not build fifty backend endpoints before connecting the first frontend page.

## 1.4 Architectural decision records

Store decisions in `docs/adr/`:

```text
0001-stack.md
0002-content-model.md
0003-pseud-isolation.md
0004-extension-marketplace-architecture.md
0005-import-storage-and-deduplication.md
0006-source-credentials.md
0007-cross-source-identity.md
0008-positivity-filter-model.md
0009-quorum-and-trust-thresholds.md
0010-translation-pipeline.md
0011-credit-and-billing-model.md
0012-analytics-and-retention.md
0013-gamification-and-quality-metrics.md
0014-declarative-recipes-vs-scripting.md
0015-shadowban-policy.md
0016-presence-and-typing-indicators.md
0017-work-monetization.md
0018-ai-crawler-posture.md
0019-media-entity-model.md
0020-redistribution-floor.md
0021-browse-ordering-contract.md
0022-demand-weight.md
```

## 1.5 Features do not override foundational protections

No feature—including a paid or admin-configured one—may:

- Reveal hidden pseud linkage.
- Bypass content eligibility checks.
- Purchase trust, ranking, or moderation authority.
- Reward output volume, or compound it into a ladder. Leaderboards, badges, bounties, and credits are permitted; their metrics must measure quality and the instance's declared themes (§0.4) — completion, positive feedback, contribution to the body of available fiction — never word count, post count, works published, or time online. Recognition is episodic (a leaderboard period, an achievement) or spendable (a credit), never a lifetime personal total (§9.7.1).
- Disable meaningful recommendation opt-out.
- Remove the free core.
- Publish machine translations as human work.
- Expose private libraries or reading history.

## 1.6 Requirements traceability

Maintain `docs/requirements.csv` with:

```text
requirement_id
description
priority (1-8, foundational)
milestone
backend_owner
frontend_surface
privacy_class
acceptance_test
verification_status
documentation_path
```

Every optional integration also has disabled, unavailable, misconfigured, and failed states.

---

# 2. Selected Architecture

## 2.1 Stack

| Area | Choice |
|---|---|
| Backend | Rust |
| HTTP | Axum |
| Async execution | Tokio |
| Serialization | Serde |
| Database access | SQLx |
| Frontend | Svelte + TypeScript + Vite |
| Editor | Tiptap with a restricted document schema |
| Styling | CSS variables, scoped component CSS, design tokens |
| SQLite search | FTS5 with trigram extensions for fuzzy matching |
| PostgreSQL search | Native full-text search with `pg_trgm` for fuzzy matching |
| Browser testing | Playwright |
| Frontend unit tests | Vitest |
| WASM execution | `wasmi` initially |
| Reverse proxy | Caddy |
| Service management | systemd |
| Default storage | Local filesystem |
| Optional billing | Payment processor adapter (operator-chosen), with billing-disabled operation |
| Optional AI (translation, summarization, classification) | Provider interface with pluggable adapters including Ollama |
| Optional caching | Redis, with in-process fallback |

Pin compatible versions and commit lockfiles.

### Why Rust

Native compilation, memory efficiency, an expressive type system, and ARM64 support.

### Why Svelte and TypeScript

Concise interactive frontend code that works well with browser APIs and does not require Node.js in production.

### Why a modular monolith

Simplified local development, transactions, deployment, backups, debugging, and resource control. A separate worker process is an optional operating mode of the same executable.

### Why a single instance per deployment

Multi-tenancy is explicitly excluded. Data isolation, pseud privacy, and resource accounting are enormously simpler in a single-tenant design. Operators who need multiple communities run multiple instances.

## 2.2 Runtime components

```text
Browser / Installed PWA / Authorized API Client / Bot
                       |
                     HTTPS
                       |
                     Caddy
                       |
              Lorehaven executable
                ├── HTTP/API
                ├── Embedded frontend
                ├── Public page rendering
                ├── Extension host (WASM)
                ├── Job scheduler
                ├── Import workers
                ├── Export workers
                ├── Translation workers
                ├── Search indexing
                ├── Positivity filter service
                ├── Notification delivery
                └── Federation workers
                       |
               SQLite or PostgreSQL
                       |
               Local file storage
```

Optional integrations:

- SMTP.
- Browser push infrastructure.
- Payment processor.
- Document converters.
- AI providers for translation, summarization, and comment classification.
- ActivityPub servers.
- Chat platforms.
- Redis.

Core reading, writing, publishing, private importing, and basic community must remain functional without optional cloud services.

**Payment processor compatibility.** The operator chooses the payment
processor adapter and owns its compatibility with the content the instance
hosts: mainstream processors restrict adult content in their terms of
service and freeze accounts that host it. The spec names no processor; the
operator must select one whose content policy matches the instance's
(§20.9). This is an operator compliance concern, not a platform feature.

## 2.3 Repository structure

```text
lorehaven/
├── Cargo.toml
├── Cargo.lock
├── crates/
│   ├── app/
│   ├── domain/
│   ├── db/
│   ├── identity/
│   ├── content/
│   ├── imports/
│   ├── library/
│   ├── search/
│   ├── discovery/
│   ├── community/
│   ├── positivity/
│   ├── governance/
│   ├── economy/
│   ├── extensions/
│   ├── translation/
│   └── integrations/
├── frontend/
│   ├── src/
│   │   ├── app/
│   │   ├── components/
│   │   ├── features/
│   │   ├── routes/
│   │   ├── stores/
│   │   ├── styles/
│   │   └── locales/
│   └── tests/
├── migrations/
│   ├── sqlite/
│   └── postgres/
├── fixtures/
├── scripts/
├── packaging/
│   ├── systemd/
│   ├── caddy/
│   └── docker/
└── docs/
```

## 2.4 Distribution

Use AGPL-3.0-or-later for first-party application code with compatible dependency licensing and documented notices. Include license notices, source-code link in the application, operator guidance for modified deployments, and a dependency inventory.

Imported fiction remains subject to its own rights and permissions.

Provide official distribution artifacts:

- Single-binary builds for x86-64 and ARM64.
- Dockerfile and docker-compose reference configuration.
- systemd unit files.
- Caddyfile examples.

---

# 3. Shared Engineering Conventions

## 3.1 IDs and timestamps

Use UUIDs for primary identifiers. Store as native UUID (PostgreSQL) or consistently encoded UUID (SQLite). Public API uses UUID strings. Timestamps are UTC internally, RFC 3339 in APIs. Display in the user's locale.

Do not expose sequential IDs or ownership through public URLs.

## 3.2 Money, credits, and counts

- Money: integer minor currency units.
- Credits: integer units.
- Word counts: nonnegative integers.
- Never use floating-point for balances.

Statistical estimates, recommendation scores, and positivity classifications may use floating-point with documented interpretation.

## 3.3 API conventions

Prefix application endpoints with `/api/v1`.

Successful collections:

```json
{
  "items": [],
  "next_cursor": null
}
```

Errors:

```json
{
  "error": {
    "code": "REVISION_CONFLICT",
    "message": "This chapter changed since you opened it.",
    "field_errors": {},
    "request_id": "..."
  }
}
```

Important error codes:

```text
AUTH_REQUIRED
ACCESS_DENIED
NOT_FOUND
VALIDATION_FAILED
REVISION_CONFLICT
RATE_LIMITED
QUOTA_EXCEEDED
CONTENT_RESTRICTED
SOURCE_UNSUPPORTED
SOURCE_UNAVAILABLE
SOURCE_AUTH_REQUIRED
SOURCE_CREDENTIAL_EXPIRED
IMPORT_REVIEW_REQUIRED
QUERY_INVALID
QUERY_TOO_COMPLEX
JOB_FAILED
CONVERTER_UNAVAILABLE
INSUFFICIENT_CREDITS
EXTENSION_PERMISSION_DENIED
EXTENSION_QUOTA_EXCEEDED
COMMENT_HELD_FOR_REVIEW
TRANSLATION_UNAVAILABLE
TRANSLATION_PERMISSION_REQUIRED
QUORUM_INSUFFICIENT
DMCA_TAKEDOWN_PENDING
```

Use `404` rather than revealing inaccessible private objects.

Translate user-facing messages without changing machine codes.

## 3.4 Concurrency

Every editable resource has a monotonically increasing `version`. Updates include `expected_version`. Version mismatch returns `409 REVISION_CONFLICT`.

## 3.5 Authentication

Use opaque server-managed sessions in secure cookies (`HttpOnly`, `Secure` in production, appropriate `SameSite`, narrow path/domain, explicit expiry and revocation). CSRF protection for state-changing cookie-authenticated requests. API integrations use separately issued, scoped tokens with hashed storage.

## 3.6 Authorization

```text
authenticate or establish anonymous actor
→ resolve active pseud where applicable
→ load resource
→ evaluate policy
→ perform operation
→ record required audit event
```

Policy lives in policy functions, not scattered frontend checks. Frontend visibility is convenience, not security.

## 3.7 Privacy classifications

Classify data and endpoints as:

- Public.
- Unlisted.
- Restricted community.
- Pseud-private.
- Account-security.
- Source-secret.
- Staff-confidential.
- Aggregate-public.
- Aggregate-internal.

Cache keys, search indexes, analytics, notifications, and background jobs preserve these classifications.

## 3.8 Rate limiting

Layered limits: account or API token, anonymous first-party client identifier, IP/network ceiling, endpoint/resource class, source domain, concurrent job count, expensive-operation budget. An anonymous client identifier is untrusted and does not bypass network ceilings. Trust forwarded IP headers only from configured proxies.

---

# 4. Database Plan

Use database-specific migrations and repository implementations where SQL differs. Run integration tests against SQLite and PostgreSQL from the beginning.

## 4.1 Shared table conventions

Unless inappropriate, mutable tables contain `id`, `created_at`, `updated_at`, `version`. Index foreign keys used in lookups, owner+time pairs, status+next-processing time for jobs, unique normalized handles, canonical source identifiers, composite filter keys, and search document scope+revision.

Use JSON for flexible documents and extension manifests, not as a substitute for searchable relationship data. Document deletion and retention behavior per migration.

## 4.2 Identity tables

| Table | Important fields |
|---|---|
| `accounts` | status, email, email_verified_at, age_state, registration_mode_at_signup |
| `password_credentials` | account_id, password_hash |
| `sessions` | token_hash, account_id, expires_at, revoked_at |
| `recovery_tokens` | token_hash, purpose, expires_at, used_at |
| `second_factors` | account_id, type, encrypted_secret |
| `pseuds` | account_id, handle, display_name, bio, discoverability |
| `public_pseud_links` | source_pseud_id, target_pseud_id |
| `privacy_settings` | account_id/pseud_id, individual policy fields |
| `age_assessments` | account_id, age_band, assurance_method, policy_version |
| `guardian_authorizations` | account_id, status, verification_reference, expiry |
| `blocks` | source_pseud_id, target_pseud_id |
| `mutes` | pseud_id, target_type, target_id, mute_scope |
| `api_tokens` | account_id, acting_pseud_id, token_hash, scopes, expiry |
| `integration_authorizations` | account_id, client_id, granted_scopes, expiry, revoked_at |
| `invitations` | issuer_account_id, code_hash, expiry, used_at, used_by_account_id |
| `registration_applications` | applicant_reference, status, submitted_at, reviewer_account_id, decision_at |

Only specifically authorized staff may retrieve private pseud ownership.

Mute scopes include: hide from feed, hide from search results, hide comments authored by, and combinations.

## 4.3 Content and identity tables

| Table | Important fields |
|---|---|
| `works` | title, summary, language, rating, visibility, lifecycle, completion |
| `work_contributors` | work_id, pseud_id, role, public_attribution |
| `chapters` | work_id, order_key, title, current_revision_id |
| `chapter_revisions` | chapter_id, document_json, sanitized_html, plain_text, word_count |
| `publication_events` | work_id, chapter_id, action, published_at |
| `series` | title, description, owner_pseud_id |
| `series_entries` | series_id, work_id, position |
| `work_relations` | source_work_id, target_work_id, relation_type |
| `media_assets` | owner_id, storage_key, media_type, size, checksum |
| `collaboration_invites` | work_id, invited_pseud_id, role, status |
| `story_identities` | canonical metadata, visibility, status |
| `story_identity_members` | identity_id, work_id/external_record_id, edition_relation |
| `identity_merge_proposals` | proposed members, evidence, status |
| `identity_merge_history` | previous identities, resulting identity, decision_reference |
| `work_feedback_preferences` | work_id, accepts_critique, accepts_anonymous_thanks, custom_note |
| `dmca_notices` | claimant_reference, targeted_content, status, filed_at, resolved_at |
| `dmca_counter_notices` | notice_id, submitter_account_id, status, filed_at |

## 4.4 Import and storage tables

| Table | Important fields |
|---|---|
| `sources` | domain, adapter_id, enabled, rate_policy |
| `adapter_versions` | adapter_id, version, verification_status |
| `source_health_windows` | source_id, adapter_version, outcome_counts, latency_summary |
| `source_incidents` | source_id, status, failure_class, opened_at, resolved_at |
| `external_records` | source_id, normalized_source_key, public metadata |
| `library_items` | owner_pseud_id, origin_type, work_id/external_record_id |
| `import_snapshots` | library_item_id, source_revision, metadata, checksum |
| `import_chapters` | snapshot_id, source_chapter_key, order, content_reference |
| `import_jobs` | owner_pseud_id, source_url, destination, status |
| `import_attempts` | job_id, stage, result, redacted_error |
| `provenance_records` | item_id, source_url, author_label, import_time, permission_state |
| `source_credentials` | owner_pseud_id, source_id, secret_type, ciphertext, nonce, key_id, expires_at |
| `source_credential_consents` | credential_id, policy_version, accepted_at, revoked_at |
| `import_batches` | owner_pseud_id, kind, source_reference, destination, status |
| `import_batch_entries` | batch_id, source_key, job_id, outcome |
| `preservation_batches` | manifest_reference, permission_basis, collection_id, review_state |
| `content_blobs` | storage_key, checksum, size, media_type, retention_class |
| `content_references` | blob_id, authorized_resource_type, authorized_resource_id |
| `source_revision_cache_entries` | source_key, revision_key, security_scope, blob_id, expiry |
| `author_watches` | owner_pseud_id, source_key, last_seen_bibliography_revision |
| `cross_post_targets` | work_id, destination_source_id, external_reference, status |

## 4.5 Taxonomy tables

```text
canonical_entities
entity_aliases
entity_relations
work_characters
work_character_attributes
work_character_roles
relationships
relationship_participants
work_relationships
work_relationship_dynamics
work_tags
tag_proposals
tag_votes
tag_merge_history
metadata_completeness
metadata_correction_proposals
metadata_suggestions
mood_tags
work_moods
rating_check_flags
```

Alias uniqueness is scoped by type and namespace. Relationship participant sets have a stable canonical signature. Character attributes belong to an identified work-character assertion. Relationship prominence belongs to the work's assertion. Spoiler status belongs to the assertion. Merges preserve redirects and history. Suggestions do not become author assertions without appropriate approval.

## 4.6 Other entity groups

| Module | Tables |
|---|---|
| Library | bookmarks, notes, shelves, shelf_entries, reading_progress, reading_events, reading_aggregates, saved_searches, search_alerts, author_watches, reading_goals |
| Reader feedback | ratings, reviews, review_revisions, work_metric_aggregates, quick_reactions, appreciation_notes, cheer_events |
| Positivity | comment_classifications, feedback_holds, moderation_queue_entries, author_visible_feedback, classifier_training_signals |
| Jobs | jobs, job_attempts, job_events, outbox_events |
| Search | search_documents, search_index_state, search_demand_aggregates, fuzzy_correction_suggestions |
| Community | comments, reactions, follows, subscriptions, groups, memberships, boards, topics, posts, polls, poll_votes, post_drafts, scheduled_posts |
| Forum depth | topic_read_states, topic_tags, topic_tag_assignments, watch_preferences, mention_events |
| Messaging | conversations, conversation_members, messages, chat_rooms, presence_preferences |
| Collections | collections, collection_roles, collection_submissions, collection_entries |
| Writing events | challenges, prompts, signups, assignments, claims, fulfillments, mentorships, sprints, wishlist_items, wishlist_votes, request_candidates |
| Governance | trust_policies, trust_history, expertise, role_assignments, reports, cases, proposals, votes, sanctions, appeals, audit_events, process_feedback, quorum_records, shadowban_actions |
| Economy | wallets, ledger_transactions, ledger_entries, credit_holds, subscriptions, payment_events, bounties, entitlements, work_pricing, work_entitlements, author_earnings_ledger, payouts, monetization_assertions, pool_b_distributions, monetization_period_summaries, work_ai_declarations |
| Extensions | packages, package_versions, manifests, installations, grants, reviews, approvals, execution_usage, revocations, extension_purchases, extension_ratings, webhook_subscriptions |
| Discovery | user_preferences, taste_profiles, taste_profile_versions, permitted_signals, exposure_events, aggregate_affinities, similarity_suggestions, similarity_votes, recommendation_recipes, recipe_versions, diversity_budgets, editorial_picks, browse_sort_preferences, taste_sources, taste_source_members, demand_weights, demand_weight_history, demand_signal_events, demand_diversity_state |
| Interface | dashboard_layouts, widget_instances, user_locale_preferences, navigation_customizations |
| Integrations | notifications, notification_preferences, push_subscriptions, feed_tokens, federation_actors, federation_deliveries, delivery_addresses, bot_links, sitemap_state |
| Translation | translation_requests, translation_jobs, translation_reviews, translated_works, translation_permissions, translation_memory_entries |
| Operations | aggregate_site_metrics, security_events, retention_runs, feature_flags, ab_test_assignments |

Introduce each schema through its owning vertical slice.

---

# 5. Milestone 0: Repository, Tooling, and Running Application

## Implement

1. Rust workspace and frontend.
2. Development configuration.
3. Structured logging with request IDs.
4. Embedded frontend assets.
5. Database selection.
6. Migration commands.
7. Health endpoints.
8. Development seed command.
9. Continuous integration.
10. Dependency and license checks.
11. OpenAPI generation infrastructure.
12. Docker packaging.
13. Feature flag infrastructure.

Commands:

```text
lorehaven serve
lorehaven worker
lorehaven migrate
lorehaven seed --development
lorehaven doctor
lorehaven flags list
lorehaven flags set <name> <value>
```

Configuration precedence: command-line → environment variable → configuration file → documented default.

Never log secrets, full authenticated URLs, private feed tokens, or source credential material.

## Feature flags

Server-side feature flags support:

- Gradual rollout of new features.
- Kill switches for problematic features.
- Per-environment configuration.

Feature flags do not affect security, privacy, or safety features. Those are always on.

Flags are visible to administrators. Users are not routinely told which flags affect them. Flags never target individual users for behavior experiments without explicit consent.

## Acceptance

- Clean checkout builds.
- SQLite and PostgreSQL startup both work.
- Frontend page loads from the Rust executable.
- `/health/live` and `/health/ready` work.
- Production startup rejects unsafe development configuration.
- OpenAPI reflects implemented endpoints only.
- Docker image builds and runs both databases.
- Feature flags can be toggled without restart.

---

# 6. Milestone 1: Design System, Navigation, Localisation, and Extension Slots

Priority 1 (customization) and priority 7 (translation) start here.

## 6.1 Reusable components

Buttons, links, inputs, labels, selects, comboboxes, dialogs, drawers, tabs, pagination, toasts, error summaries, loading skeletons, work cards, metadata chips, identity switcher, empty-state panels, job-progress panel, source-health badge, visibility selector, permission-request panel, positivity-context banner, feedback-preferences panel, extension-slot boundaries.

Design tokens:

```text
color.background
color.surface
color.text
color.muted
color.primary
color.danger
color.positive
space.*
radius.*
font.interface
font.reader
```

Visual direction: warm neutral surfaces, deep plum accent, restrained teal secondary accent, generous spacing, light/dark/system modes.

## 6.2 Navigation

Desktop:

```text
Discover | Search | Library | Write | Community | Notifications | Pseud
```

Mobile:

```text
Discover | Search | Library | Write | More
```

Reader pages may use a reduced shell.

Users may customize navigation by pinning their most-used links. Safety controls, moderation notices, extension management, and the notifications entry cannot be hidden.

## 6.3 Extension slot architecture

Establish extension slots from Milestone 1 so the marketplace has stable integration points:

- Dashboard widget slots.
- Reader sidebar slots.
- Work-page metadata slots.
- Search-result decorator slots.
- Writing-tool panel slots.
- Theme override tokens.

Slots have documented data contracts, size constraints, and permission requirements. The extension host does not exist yet (Milestone 15), but slot contracts do so first-party features use the same mechanism third-party extensions will.

## 6.4 Command palette

Provide `Ctrl+K` / `Cmd+K`.

Initial commands: search works, paste URL to import, open library, resume reading, create draft, switch pseud, open saved view, install extension, open help, change appearance, change language.

Mutating commands require ordinary confirmation. Do not intercept editor or assistive-technology shortcuts.

## 6.5 Interface localisation

Localise interface chrome, not just work metadata.

Requirements:

- Message catalogs from the beginning.
- Locale-aware pluralization.
- Date, number, and currency formatting.
- No sentence construction by concatenating fragments.
- Language selector independent of work-language preferences.
- Document `lang` and direction attributes.
- Support for longer translations and right-to-left text.
- Locale fallback without blank labels.

Initial catalogs: English and Spanish. Additional locales through the same contribution workflow. Do not advertise a locale as complete until reviewed.

## 6.6 Contextual help

Contextual `?` links, in-app help panel or modal, direct links to documentation, "try it" links that prefill forms, searchable help, non-JavaScript help where practical. Help and tutorial share topic identifiers. Never put credentials, private draft text, or sensitive queries into help deep-link URLs.

## 6.7 Appearance modes

Two first-party layout presets:

- **Modern:** card-oriented discovery and contemporary navigation.
- **Archive:** compact metadata, list-oriented browsing.

Themes and layout presets from the marketplace override these. Reader typography remains independently configurable. Neither preset may hide safety controls, attribution, or essential metadata.

Beyond the presets:

- **Accent colour choice** decoupled from full themes: one token, a small palette, trivial to ship, enormous felt customization.
- **Reader-only themes** distinct from interface themes. A reader who wants a dark reading surface on a light interface, or the reverse, gets both without one overriding the other.
- **Appearance import/export**: a full appearance bundle — theme, accent, reader settings, dashboard layout — as one JSON file. This is what makes customization portable between self-hosted instances.
- **Per-pseud appearance.** An account shares identity across its pseuds but not necessarily an environment; appearance follows the same per-pseud rule as dashboard layouts (§16.8).

## Acceptance

- Keyboard operation and visible focus.
- Dialog focus trapping and restoration.
- Usability at 320 CSS pixels and 200% zoom.
- Reduced-motion support.
- Accessible error announcements.
- Critical journeys work in English and Spanish.
- Pseudolocalisation catches clipping.
- Extension slots render placeholder content until Milestone 15.
- Navigation customization cannot hide safety controls.
- Help links validated in CI.

---

# 7. Milestone 2: Accounts, Pseuds, Privacy, Age Policy, and Registration Modes

## 7.1 Implement

- Registration, login, password reset, email verification.
- Session listing and revocation.
- Sign-in alerts: a new session on an account notifies the account's verified address, naming the device class and general region, and the alert is opt-out rather than opt-in.
- TOTP and recovery codes.
- Pseud creation and switching.
- Privacy settings.
- Block and mute primitives with scoped mutes.
- Age-policy state.
- Scoped API tokens.
- Per-pseud learning, reading-history, and feedback-visibility controls.
- Registration modes: open, invite-only, application-based.
- Invite code generation and tracking.
- Registration application queue.

Use an established password-hashing implementation with reviewed parameters.

## 7.2 Pseud behavior

Each pseud has separate: public profile, works, follows, messages, recommendation settings, ratings and reviews, feedback preferences, public bookmarks, reading history, dashboard layout, navigation customization, installed extensions, source credential grants, notification preferences, presence preferences, and language preferences.

The account shares: credentials, security state, private wallet, trust eligibility, and invite quota.

Do not publicly reveal shared ownership.

A scoped integration token binds to an explicitly selected pseud unless it genuinely needs account-level functions.

## 7.3 Age implementation

State machine:

```text
unknown → declared_minor → declared_adult
       → authorization_required → authorized_under_policy → restricted
```

Do not treat self-declared adult as verified. For a Spain-based operator, Spain generally uses 14 as the threshold for child data-processing consent; this does not settle all age-related obligations. Do not enable unrestricted child registration through a checkbox without a legally and operationally established workflow.

Avoid collecting full birth dates unless necessary.

## 7.4 Registration modes

Administrator selects the active mode:

- **Open:** any visitor may register.
- **Invite-only:** registration requires a valid invite code.
- **Application-based:** applicants submit a short application; moderators review.

Invite codes:

- Issued by users with sufficient trust (configurable).
- Have an expiry date.
- Have a use count limit (default one).
- Track who used the invite.
- Never reveal the inviter's pseuds to the invitee.
- Can be revoked before use.

Registration applications:

- Include a short "why do you want to join" field.
- Enter a moderator review queue (quorum required per policy).
- Applicants receive a decision notification.
- Rejected applications may be resubmitted after a cooldown.
- Applications do not expose applicant IP or fingerprint data to reviewers by default.

## 7.5 Blocks and scoped mutes

Blocks are bidirectional invisibility: neither party sees the other's content or can contact them.

Mutes are one-directional and scopable:

- Mute from feed only (still visible in search).
- Mute from search results (visible in feed if followed).
- Mute comments authored by (their comments hidden on your works and works you read).
- Mute entirely (equivalent to a soft block from the muter's side).
- Mute specific fandoms, tags, or moods.

Mutes are private to the muter. Muted parties are not notified.

## 7.6 Shared content eligibility

```text
can_access_content(actor, content_rating, visibility, policy)
```

Use for reader, search, downloads, feeds, notifications, recommendations, API, extensions, bots, translations, and public statistics.

## 7.7 API

```text
POST   /api/v1/auth/register
POST   /api/v1/auth/login
POST   /api/v1/auth/logout
POST   /api/v1/auth/password-reset
POST   /api/v1/auth/password-reset/complete
GET    /api/v1/auth/sessions
DELETE /api/v1/auth/sessions/:id

GET    /api/v1/pseuds
POST   /api/v1/pseuds
PATCH  /api/v1/pseuds/:id
POST   /api/v1/pseuds/:id/activate

GET    /api/v1/settings/privacy
PATCH  /api/v1/settings/privacy
GET    /api/v1/settings/content
PATCH  /api/v1/settings/content
GET    /api/v1/settings/feedback
PATCH  /api/v1/settings/feedback
GET    /api/v1/settings/presence
PATCH  /api/v1/settings/presence

GET    /api/v1/api-tokens
POST   /api/v1/api-tokens
DELETE /api/v1/api-tokens/:id

GET    /api/v1/invitations
POST   /api/v1/invitations
DELETE /api/v1/invitations/:id

POST   /api/v1/registration-applications
GET    /api/v1/registration-applications/:id (own only)
```

## 7.8 First-run experience

A new account's first session ends in something to read, not in an empty shelf.

The flow is short, skippable and resumable:

- **Choose fandoms.** A searchable selection with no minimum and no maximum.
- **Choose a few moods or tones** from the taxonomy (§15.8).
- **Choose a format preference:** written, media, or both (§30).
- **State the content notes the reader wants surfaced**, drawn from the content-note vocabulary (§15.16), so the first work shown already respects them.
- **Offer the taste quiz.** A short set of "which of these two would you rather read" comparisons — the arena mechanism of §29.2 applied to works rather than to roadmap cards — which seeds a taste profile before the reader has any history. Skippable.
- The reader is then shown a small set of works drawn from what they just said, each with the reason it was chosen stated in plain language.

Rules:

- **The first session ends with a work being opened.** If the stated preferences produce too few results, the flow says so and widens the recommendation honestly rather than presenting generic trending content as though it matched.
- Everything chosen here is editable afterwards in the ordinary settings, and the flow is re-runnable from help.
- **The flow explains the comment norms before the reader can post:** appreciation is the default, critique is opt-in per work (§8.4), and the filter (§12.1) is described as what it is. A reader who has not been told meets the norms as a surprise rather than as a culture.
- The flow never asks for a real name, never requires contact details beyond what registration required, and adds nothing to the reader's public profile.
- Skipping the flow leaves a working account, and recommendations then fall back to the baseline engines (§16.1) with the coverage caveat stated.
- Recommendation coverage is stated honestly throughout: a reader is told which signals are in use, and a first-session recommendation is labeled as based on stated preferences rather than on behavior.

## Acceptance

- No cross-account pseud editing.
- No hidden linkage in public responses.
- Revocation takes effect.
- Frontend bypass cannot retrieve restricted content.
- Sensitive data absent from logs.
- Minor-protective messaging defaults persisted.
- Private history, source credentials, and extension grants remain compartmentalized after pseud switching.
- Invite codes cannot be reused beyond their limit.
- Registration applications route to the moderator queue.
- Mute scopes take effect immediately across all surfaces.

---

# 8. Milestone 3: Drafts, Chapters, Publishing, Feedback Preferences, and Post Drafts

## 8.1 First vertical slice

Create draft → add chapter → save → preview → publish → read publicly → edit → republish.

## 8.2 Work states

```text
Lifecycle: draft | scheduled | published | withdrawn | deleted
Visibility: public | unlisted | restricted
Completion: in_progress | complete | hiatus | abandoned
```

Explain that unlisted content is accessible to anyone with the URL under applicable restrictions.

## 8.3 Editor

Restricted Tiptap schema: paragraphs, headings, emphasis, strong, lists, blockquotes, links, scene breaks, author notes, footnotes, endnotes.

- **Author notes** are first-class blocks, placed before or after a chapter's body and clearly distinguished from the story text. A note before the body carries posting context — schedule, warning pointers, thanks; a note after it carries replies and next-chapter plans. Both collapse in the reader once a reader has hidden them, and both are excluded from word counts, from exports' main body, and from body search (§15.9).
- **Footnotes and endnotes** are structured: a footnote anchor in the body resolves to a note rendered at the chapter's foot; an endnote collects to the work's end matter. They survive export (§13.1) as native EPUB footnotes where the format supports them and as clearly marked sections where it does not.

No arbitrary HTML, scripts, iframes, or embedded objects initially.

Autosave: debounce → local recovery save → versioned server update → visible saved/saving/offline/conflict state. Preserve local text when server rejects.

Imported DOCX, HTML, EPUB, and Markdown convert into the same restricted schema before becoming editable drafts.

## 8.4 Feedback preferences

Per work, per pseud, and per account default:

- Accept public comments (yes/no/moderated).
- Accept anonymous appreciation notes.
- Accept quick reactions.
- Accept private thank-you messages.
- Accept constructive critique (opt-in, default off).
- Custom author note shown to readers before commenting.

These preferences drive the positivity filter (Milestone 12).

## 8.5 Publishing transaction

In one transaction: validate metadata, verify contributor permissions, update publication state, record publication event, insert outbox events for notifications and indexing.

Do not send email inside the transaction. Scheduled publication uses the same idempotent service.

## 8.6 Post drafts and scheduled posts

Forum posts and comments support local draft saving:

- Automatic draft recovery on refresh.
- Optional server-side draft storage per pseud.
- Draft expiry after a configurable period.
- Draft encryption is not implied; drafts share pseud privacy classification.

Scheduled posts:

- Available for forum topics and forum posts.
- Positivity filter applies at publication time, not creation time.
- Author may edit or cancel before publication.
- Failed scheduled publications notify the author with reason.

## 8.7 API

```text
POST   /api/v1/works
GET    /api/v1/works/:id
PATCH  /api/v1/works/:id
POST   /api/v1/works/:id/publish
POST   /api/v1/works/:id/withdraw

POST   /api/v1/works/:id/chapters
PATCH  /api/v1/chapters/:id
POST   /api/v1/works/:id/reorder-chapters
GET    /api/v1/chapters/:id/revisions
POST   /api/v1/chapters/:id/restore-revision

GET    /api/v1/works/:id/feedback-preferences
PATCH  /api/v1/works/:id/feedback-preferences

POST   /api/v1/works/:id/contributors/invitations
PATCH  /api/v1/works/:id/contributors/:pseudId

GET    /api/v1/drafts/posts
POST   /api/v1/drafts/posts
DELETE /api/v1/drafts/posts/:id
```

## 8.8 Activity status and WIP honesty

Readers invest in works that stop. The archive states what it knows rather than leaving a reader to infer it from a date.

Activity status is derived and displayed alongside completion:

```text
Activity: active | slow | dormant | concluded
```

- `active` — updated within three months. `slow` — three to twelve months. `dormant` — over twelve months. `concluded` — the work is marked complete.
- Derived status is **information and never a penalty.** It changes no ranking, award, credit, badge or trust outcome, and a dormant work keeps everything it earned.
- **An author-set state always wins.** `hiatus` and `abandoned` (§8.2) override the derived reading, and a work marked complete is `concluded` regardless of when it was last touched.
- Readers may filter dormant and abandoned works out of search, saved views (§14.2) and recommendations, and **the filter is off by default.**
- A "likely complete" note may be offered for a work that has not been updated but reads as finished — no open threads, a completion-shaped structure, and the author's own completion field unset. It is a suggestion to the author and never a claim shown to readers as fact.
- After a long inactivity period the author receives **one** notice they can opt out of: whether the work is still going, and an offer to mark it hiatus or abandoned. It is sent once, never repeated on a schedule, and carries no guilt framing (§28.10). Declining is silent and permanent.
- The notice never appears on a work whose author has disabled notifications, and never during a configured quiet period.

## Acceptance

- Concurrent edits return conflicts.
- Revision restoration creates a new revision.
- Public readers never receive unpublished revisions.
- Repeated publication with one idempotency key does not duplicate notifications.
- Pseud switching does not change ownership.
- Invitations identify the exposed pseud.
- Feedback preferences apply before comments are stored.
- Post drafts survive session loss.
- Scheduled posts fail gracefully on positivity classification issues.

---

# 9. Milestone 4: Reader, Ratings, Reactions, History, and Goals

## 9.1 Routes

```text
/works/:id
/works/:id/chapters/:chapterId
/works/:id/download
/works/:id/comments
/works/:id/reviews
/series/:id
```

Private imported content uses authenticated library routes.

## 9.2 Reader features

Chapter navigation, whole-work mode, table of contents, typography settings — font family (including accessible and dyslexia-friendly options), size, line height, justification — light/dark/sepia/high-contrast themes, width/line-height controls, distraction-free mode, spoiler reveal, progress, private notes, search within current work, reading-time estimates, end-of-work actions.

**Text-to-speech.** The reader can have any chapter read aloud. The default engine is the browser's own speech synthesis — free, offline, and no text leaves the device. Where an AI provider is configured, a higher-quality voice is available as a metered AI task (§23.7) under the budget guardrails (§22.11): it quotes before it runs, and it never produces a stored audiobook the author did not publish. A published podfic or audiobook unit (§30.1) always takes precedence over generated speech. TTS output is never stored as a work, never indexed, and never federated.

**Reader layout persistence.** Paged, continuous and whole-work modes are saved per reader and per work, not per browser: a work read in one layout stays in that layout on the next device. Typography is remembered per work as well, because a work read in a right-to-left language and a work read in English do not share one setting.

**Custom reader CSS.** A reader may supply free-form CSS scoped to the reading surface, sanitized by the same pipeline as marketplace themes, opt-in and warned about. It is the cheapest honest answer to "customize my reading experience" and is off by default.

Long works must not require rendering every paragraph at once.

## 9.3 Progress

Store `work_or_library_item_id`, `chapter_id`, `content_revision`, `paragraph_anchor`, `position_fraction`, `updated_at`, `device_id`.

Use stable paragraph anchors where possible with approximate fallback after content changes. When devices disagree, present a choice.

## 9.4 Quick reactions

One-click labeled reactions on chapters and works:

- "made me cry"
- "the banter!"
- "I need more of this"
- "this scene!"
- "comforting"
- "worldbuilding!"
- Additional labels defined by the extension marketplace over time.

Reactions:

- Do not require typing.
- Are subject to the author's feedback preferences.
- Aggregate publicly if the author permits.
- Count once per reader per target.
- Bypass the positivity filter (they are pre-classified positive).

## 9.5 Ratings and reviews

Support optional 1–5-star personal rating, optional written review, spoiler marking, edit and delete, independent rating/review visibility, reporting and blocking, inclusion in personal discovery only under the relevant learning setting.

Defaults:

- Ratings are private.
- Written reviews remain private until explicitly published.
- Public review identity is the active pseud.
- Private ratings never contribute to public averages.

Public aggregate ratings include only explicitly public ratings, display count and method, use a minimum publication threshold, and are not a default ranking signal.

Work owners may disable display of public rating aggregates.

Public reviews are subject to positivity filtering (Milestone 12). Low-star reviews with harsh language may be held for moderator review. A low star rating alone is not classified as destructive; the accompanying text is what triggers classification.

## 9.6 Reading history and personal analytics

```text
/library/history
/library/reading-stats
```

History contains recently opened, resumable, finished works with timestamps and removal controls.

Settings separately control progress sync, detailed history retention, personal aggregate statistics, and recommendation learning. Disabling learning does not disable resume.

Personal analytics: works marked finished, chapters read, estimated words read, approximate reading time, optional personal streak.

A **year in review** page compiles the reader's own year: works finished, words read, fandoms visited, moods most read, bookmarks added, and the authors they appreciated most. It is private by default, shareable only as an explicit action, computed from data the reader already controls, and it shares the history settings' deletion behavior: cleared history is a cleared review.

Do not count opens as proof of reading. Label estimates as such.

## 9.7 Reading goals, streaks, and gamification

### 9.7.1 Design principles

Gamification rewards the behaviors the platform wants more of: completing fiction, leaving genuine feedback, importing new sources, reading diversely, and fulfilling community demand.

Rules:

- **Never punish inactivity.** Missing a day costs nothing. Streaks reset without guilt messaging.
- **Never create anxiety.** No "your streak will break" notifications. No loss-framed language.
- **Reward quality, not volume.** Completion rates and positive feedback multiply author earnings. Raw word count and post count do not. This governs every gamification surface, not only credits: a badge, a leaderboard position, a bounty, or a credit must be earned from quality signals (§9.7.4), reader behavior, and contribution to the body of available fiction — never from output volume, tenure, or spend.
- **Episodic, never cumulative.** Recognition resets. A leaderboard period closes and a placement expires with it; a badge records that something happened once; a credit is spent. There is no XP bar, no level, no lifetime point total and no cross-surface reputation score, so a past contribution cannot compound into present standing. A per-surface signal that decays on a schedule and confers nothing — §35.2's forum karma is the existing example — is permitted precisely because it neither compounds nor authorizes anything (§9.7.5, §9.7.6, §28.10). **Streak bonuses are flat milestone payouts (e.g., +5 credits at 7 days, +15 at 30 days), never a persistent multiplier on per-action credits.** A multiplier on all engagement dilutes the taste-weighted signal by amplifying noise (off-target bookmarks, random kudos) as much as signal. A flat bonus rewards the habit of showing up without distorting per-action economics. (See §9.7.1 for the full rationale and the narrow login-only multiplier alternative.)
- **Recognition, never a gate.** A badge, a leaderboard placement, or a credit balance never gates a feature the free core provides (§0.3), never rank-gates participation, and never confers governance authority (§19.1).
- **Credits and trust are separate.** Earning credits never advances trust level. Trust requires reviewed conduct.
- **Admin taste is invisible.** The taste multiplier affects author credit amounts silently. No user-facing label, breakdown, or hint reveals the administrator's preferences.

### 9.7.2 Daily login and action credits

**Login bonus:**

| Action | Credits | Notes |
|--------|---------|-------|
| Daily login | 5 | Once per calendar day (user's timezone). Flat rate regardless of streak length. |

**Daily action credits:**

| Action | Credits | Daily cap | Notes |
|--------|---------|-----------|-------|
| Read a chapter | 1 | 10/day | Once per unique chapter per day. Minimum 30 seconds on page or scroll depth threshold. |
| Leave a quick reaction | 1 | 5/day | Pre-classified positive. |
| Leave a positive comment (delivered) | 2 | 3/day | Must pass positivity filter and reach the author. |
| Leave a constructive review (delivered, author opted in) | 3 | 1/day | Higher reward for effort. |
| Post in a forum | 1 | 5/day | Minimum 100 characters. |
| Create a forum topic | 2 | 2/day | |
| Bookmark a work | 1 | 5/day | Private or public. |
| Import a work | 2 | 5/day | Unique source key per day. |
| Import from a new source (first time ever) | 10 | 1/day | One-time per source domain. |
| Fulfill a wishlist item | 15 | 1/day | Requester must confirm match. |
| Complete a translation chapter (approved) | 5 | 5/day | Human-reviewed. |
| Vote on a subforum proposal | 2 | 5/day | Governance participation. |

**Daily action cap:** 50 credits total from daily actions (75 for Author subscription, 100 for Curator/Patron).

### 9.7.3 Reader completion credits

| Action | Credits | Notes |
|--------|---------|-------|
| Mark a work as finished | 5 | Once per work per pseud. Minimum time-on-page enforced. |
| Finish a work under 10k words | 3 | Bonus for short complete fiction. |
| Finish a work over 100k words | 10 | Bonus for long fiction commitment. |
| Finish a work in a new fandom | 3 | Cross-fandom discovery. |

**Monthly cap:** 200 credits from completion bonuses.

**Quality gate:** A work counts as "finished" only if the reader spent a minimum time proportional to word count. Opening the last chapter and immediately marking finished does not count.

### 9.7.4 Author credits

Authors earn credits when readers engage with their work.

**Base author rewards:**

| Event | Credits to author | Notes |
|-------|-------------------|-------|
| Unique reader reads a chapter | 1 | Once per reader per chapter. Capped at 50 unique readers/day per work. |
| Reader completes the work | 5 | Once per reader per work. |
| Reader leaves a quick reaction | 1 | Per reaction per reader. |
| Reader leaves a positive comment | 2 | Per delivered positive comment. |
| Reader bookmarks the work | 1 | Per unique reader. |
| Reader adds to a public collection | 2 | Per collection. |

**Quality multiplier:**

```text
quality_multiplier = 1.0
  + 0.3 × completion_rate_bonus    (if >60% of starters finish)
  + 0.2 × positive_feedback_bonus  (if >80% of feedback is positive)
  + 0.2 × reread_bonus             (if >10% of readers return)
  + 0.1 × bookmark_rate_bonus      (if >15% of readers bookmark)
```

Range: 1.0x to 1.8x. Recalculated weekly from the previous 30 days. Minimum 10 readers before quality signals activate. New works start at 1.0x.

**Demand multiplier (silent):**

```text
demand_multiplier = 1.0
  + 0.25 × admin_taste_affinity
  + 0.15 × wishlist_demand
  + 0.10 × search_demand
```

Range: 1.0x to 1.5x. The admin taste component is **never disclosed**. Authors see only their total credits and the quality multiplier breakdown. The demand multiplier is folded silently into the total.

**What the author sees:**

```text
"Your work earned 180 credits this week."
"Quality bonus: +50% (high completion rate, positive reader feedback)."
```

No mention of "trending category," "demand bonus," "admin taste," or any label that hints at the administrator's preferences. The quality multiplier breakdown is shown because it is actionable and based on public reader behavior. The demand multiplier is not shown because it contains the private taste signal.

**Author credit caps:**

| Cap | Limit |
|-----|-------|
| Per work per day | 100 credits |
| Per author per day | 300 credits |
| Per author per month | 5,000 credits |

**Pseud isolation:** Pseuds on the same account do not generate author credits for each other's works.

### 9.7.5 Leaderboards

**Time windows:** Daily (resets midnight UTC), Weekly (resets Monday 00:00 UTC), Monthly (resets first of month).

**Categories:**

| Category | Metric |
|----------|--------|
| Top Completed Reads | Works marked finished |
| Top Positive Feedback | Positive reactions + comments received |
| Top Completion Rate | % of readers who finish (min 10 readers) |
| Top Wishlist Heroes | Wishlist items fulfilled |
| Top Translators | Approved translation chapters |
| Top Curators | Quorum votes cast, tag proposals approved |
| Top Importers | Works imported from new sources |
| Top Discoverers | Unique fandoms/moods read |

**Deliberately absent:** most words written, most posts, most kudos given, longest streak, and every ranking whose metric is output volume. A category's metric must be a quality signal (§9.7.4) or a declared public topic (§0.4.1).

**No all-time board.** Every category is windowed — daily, weekly, monthly — and a placement expires with its window. There is no all-time board and no cumulative score, so winning a period is a moment rather than an asset (§9.7.1's episodic rule). A period's reward is paid once and the board starts empty.

**Display:**

- Top 20 per category per time window.
- Users see their own rank even if outside top 20.
- **Opt-out:** all users appear by default. Users may hide themselves from public leaderboards in settings. Hidden users still earn leaderboard credits.
- Each entry shows pseud name and metric value only.
- Accessible from `/leaderboards` and as a dashboard widget.

**Leaderboard rewards:**

| Placement | Credits |
|-----------|---------|
| 1st | 50 |
| 2nd–3rd | 25 |
| 4th–10th | 10 |
| Participation (score > 0) | 2 |

Paid at the end of each period.

### 9.7.6 Badges

**Milestone badges (one-time, permanent):**

| Badge | Condition | Credit bonus |
|-------|-----------|-------------|
| First Words | Publish first work | 20 |
| First Finish | Mark first work finished (reading) | 10 |
| First Import | Import first work | 10 |
| First Translation | Publish first approved translation | 30 |
| First Fulfillment | Fulfill first wishlist item | 25 |
| First Review | Leave first constructive review | 10 |
| First Quorum | Cast first governance vote | 15 |
| Centurion Reader | Finish 100 works | 50 |
| Prolific Author | Publish 10 complete works | 50 |
| Polyglot | Read works in 5 languages | 30 |
| Explorer | Read works in 10 fandoms | 30 |
| Tastemaker | 50 of your bookmarks also bookmarked by others | 30 |
| Wishlist Hero | Fulfill 10 wishlist items | 50 |
| Bridge Builder | Read in 5 previously unread fandoms | 20 |

**Recurring badges (earned multiple times, display "Awarded ×N"):**

| Badge | Condition | Credits per award |
|-------|-----------|------------------|
| Daily Reader | Read 5+ chapters in a day | 2 |
| Week in Books | Read every day for 7 days | 5 |
| Feedback Star | Receive 20 positive reactions in a week | 5 |
| Completionist | Finish 5 works in a month | 10 |
| Challenge Finisher | Complete a writing or reading challenge | 15 |
| Leaderboard Top 10 | Place top 10 on any leaderboard | 10 |
| New Source Pioneer | Import from a previously unimported source | 15 |
| Demand Responder | Fulfill a wishlist item with 10+ votes | 20 |

**Seasonal badges (limited-time, display year/season):**

| Badge | Condition | Notes |
|-------|-----------|-------|
| Summer Reader [Year] | Finish 20 works during summer event | Unique per season |
| Holiday Writer [Year] | Publish during winter challenge | Unique per season |

Seasonal events run for 4–8 weeks with boosted multipliers (1.5x completion credits) and unique badges. They are announced in advance and entirely optional.

**Badge display:**

- Appear on public profile if user chooses to display them.
- Each badge shows icon, name, and **"Awarded ×N"** count.
- Global award count visible in the badge catalog ("Awarded 142 times across all users") to show rarity.
- Users feature up to 6 badges on their profile.
- Badge catalog at `/badges` with all available badges, conditions, and global counts.
- No badge gates a feature. No badge grants trust.
- **Badges are achievements, not levels.** A badge count is never summed into a rank, never orders a leaderboard, and never becomes a lifetime tally: a badge records that something happened once (§9.7.1).

### 9.7.7 Streaks

- Reading streak: consecutive calendar days reading at least one chapter.
- Writing streak: consecutive days saving at least one draft edit.
- **Private by default.** Public display opt-in.
- No credit multiplier for streak length.
- No "streak will break" notifications.
- Reset message is neutral: "Your reading streak reset. Start a new one?"
- **Streak freeze:** spend 5 credits to preserve a streak through one missed day. Once per streak. Included free with Author subscription and above.
- Streak milestones trigger cosmetic badges (Week in Books, etc.).

### 9.7.8 Anti-gaming measures

| Risk | Mitigation |
|------|-----------|
| Speed-reading to farm completions | Minimum time-on-page proportional to word count. |
| Alt account reaction farming | Accounts <7 days old or TL0 don't generate author credits. |
| Self-reacting across pseuds | Same-account pseuds don't generate author credits for each other. |
| Repeated imports of same fic | Import credits only for unique source keys. |
| Low-effort forum spam | Forum post credits require 100-character minimum. |
| Quality multiplier gaming | 30-day rolling window, minimum 10 readers. |
| Wishlist fulfillment farming | Requester must confirm fulfillment matches. |
| Leaderboard manipulation | Unique-account deduplication. |

### 9.7.9 Reading goals

Reading goals remain private, opt-in, and grant nothing. They are a personal
progress display, not a source of credits.

Users may set:

- Daily or weekly reading target (chapters, words, or works).
- Progress display on personal dashboard.

The goal display shows current progress toward the user's chosen target.
Missing a target is displayed as neutral information, not failure, and meeting
one earns no credit bonus: the daily reading credit in 9.7.2 is unaffected by
whether a goal exists, so a goal can never become a way to farm credits.

**Writing goals** work the same way for authors: a daily or weekly word-count or
drafting target, progress on the author dashboard, private by default, and
granting nothing. Words outlined, drafted, or revised all count toward it,
because the goal serves the author's own pacing and not a volume metric; there
is no leaderboard for it (§9.7.5) and no credit attaches to it. Sprints
(§18.4) remain the social form of the same need; the goal is the private one.

**Writing goals** work the same way for authors: a daily or weekly word-count or drafting target, progress on the author dashboard, private by default, and granting nothing. Words outlined, drafted, or revised all count toward it, because the goal serves the author's own pacing and not a volume metric; there is no leaderboard for it (§9.7.5) and no credit attaches to it. Sprints (§18.4) remain the social form of the same need; the goal is the private one.

**Acceptance**

- Daily login credits award once per calendar day.
- Reading goals are private, opt-in, and grant nothing.
- Action credits respect daily caps.
- Completion credits enforce minimum time-on-page.
- Author credits aggregate silently with quality and demand multipliers.
- No user-facing label, breakdown, or notification reveals the administrator's taste influence.
- Leaderboards rotate on schedule and pay rewards.
- Users can opt out of public leaderboard display.
- Badges display "Awarded ×N" count and global rarity.
- Streak resets produce neutral messaging with no guilt framing.
- Streak freezes work correctly and deduct credits.
- Anti-gaming measures prevent the listed exploits.
- Credit earnings never affect trust level.

## 9.8 Views and completion rates

Aggregate view counts:

- Deduplicate repeated opens within a documented window.
- Filter known automated traffic.
- Do not claim perfect human uniqueness.
- Exclude private imports.
- Author-configurable public display.

Completion rate:

- Track fraction of readers who start a work and mark it finished.
- Author-configurable public display.
- Used as a quality signal in discovery.
- Minimum sample threshold before display.
- Never used to penalize experimental or unpublished-length works.

**Author analytics.** Each work and each pseud has an analytics view holding
only aggregate, non-identity data: reads and unique readers over time,
completions and completion rate, reactions by label, positive comments,
bookmarks, collection additions, subscriptions, downloads by format, and
reading-time distribution. Every chart states its definition and exclusions,
with the same documented semantics as §24.2's public statistics.

- No per-reader data anywhere: no reader list, no reader journeys, no
  per-identity timestamps. An author who wants to know who read their work can
  ask them, and reader privacy answers.
- Referrers are not collected by default. An operator may enable domain-level
  referrer aggregation — no full URLs, no query strings, no cross-site
  identifiers — and turning it on is recorded in the modlog (§19.12).
- The view excludes other readers' private imports of the author's work; a
  private copy someone else holds is that reader's business.
- Author analytics feeds no ranking and never surfaces a demand-multiplier
  component (§9.7.4): the charts show reader behavior, never the
  administrator's taste.

## 9.9 Reading-time estimates

Baseline: word count and configurable reading speed. Optional dialogue-density adjustment after benchmarking. Display approximate range rather than false precision. Language-sensitive tokenization where available.

## 9.10 End-of-work page

Appreciation, bookmark, rating or review, quick reactions, positive comment prompt, mark finished, next in series, another work by the author, configurable next reads, optional writing opportunity, "recommend to a friend" with personal note.

## Acceptance

- Progress survives refresh.
- Content updates do not crash resume.
- Sanitized content cannot execute scripts.
- Public eligible pages have meaningful sharing metadata and non-JavaScript content.
- Clearing history removes retained events.
- Private ratings never appear in public APIs or averages.
- Repeated refreshes do not inflate views without bound.
- Completion rate calculation excludes automated traffic.
- Missed reading goals produce no coercive notifications.
- Broken streaks produce no loss-framed messaging.

---

# 10. Milestone 5: Jobs, Storage, Cache Boundaries, and Secret Management

Settle import storage and credential architecture before implementing website adapters.

## 10.1 Job model

```text
queued → running → succeeded
running → retry_wait → queued
running → failed
queued/running → canceled
```

Fields include `kind`, `owner_account_id`, `owner_pseud_id`, `resource_class`, `priority_class`, `payload`, `idempotency_key`, `attempt_count`, `run_after`, `lease_owner`, `lease_expires_at`, `progress`, `error_code`.

Batch jobs contain durable child-job references and resumable enumeration cursors.

## 10.2 Worker behavior

Transactional job claims, leases and renewal, backoff for transient errors, bounded attempts, abandoned-job recovery, cancellation between safe checkpoints, idempotent handlers. Delivery is at least once.

## 10.3 Storage

Generated or content-addressed storage keys, never user-supplied paths. Temporary upload directory, atomic finalization, checksums, logical and physical storage accounting, quotas, reference-aware orphan cleanup, disk-pressure warnings, safe path resolution, extraction limits.

Define whether quotas charge logical ownership or physical bytes. Users must not infer another user's holdings from quota discounts.

## 10.4 Revision cache and physical deduplication

Separate:

1. **Authorized user snapshots:** durable copies referenced by private library items.
2. **Temporary fetch cache:** reusable bytes under documented scope and expiry.
3. **Approved public preservation corpus:** content explicitly authorized for public archival use.

Which of these a fetched body may occupy is the instance's decision rather than the importer's: §11.15 states it once, and an instance set to `aggregate` produces none of the three — metadata is stored and the body is never fetched.

Cache keys include source identity, source revision or validated fingerprint, adapter extraction version, and security scope.

Rules:

- Credentialed fetches are private-scoped by default.
- A checksum is not an authorization credential.
- Blob lookup by arbitrary hash is not a public API.
- Deduplication does not expose whether another user has a work.
- Cache reuse does not bypass current source-access policy.
- A shared byte store does not automatically create a shared search corpus.
- Retained cache entries expire independently of user-owned snapshots.
- Deletion removes references and eventually collects unreferenced blobs.
- Public preservation is an explicit workflow, never a side effect.

## 10.5 Secret encryption

Provide authenticated encryption for recoverable integration secrets. Use a reviewed AEAD implementation (such as AES-256-GCM) with unique nonces, authenticated context binding, versioned ciphertext, key identifiers, key rotation, keys stored outside the database, backup and recovery documentation.

Encryption at rest does not protect against a fully compromised running server. State that limitation. Jobs store secret references, not plaintext.

## Acceptance

- Worker death does not lose jobs.
- Parallel workers do not publish or charge twice.
- Low disk produces actionable errors.
- Traversal and archive-bomb attempts fail safely.
- Errors are redacted.
- Cross-user cache reuse cannot bypass authorization.
- Credential ciphertext cannot be decrypted without the configured key material.
- Cache eviction does not delete a referenced snapshot.

---

# 11. Milestone 6: Imports, Credentials, Batches, Watches, Preservation, and Cross-posting

Priority 2 (maximize fiction) is centered here.

## 11.1 Adapter abstraction

```rust
trait SourceAdapter {
    fn identify(&self, url: &Url) -> Option<SourceKey>;
    async fn fetch_metadata(&self, request: ImportRequest) -> Result<SourceMetadata, ImportError>;
    async fn fetch_chapter(&self, chapter: SourceChapter) -> Result<ImportedChapter, ImportError>;
}
```

Optional capability interfaces for bibliography enumeration, update checking, source authentication, conditional requests, source-specific revision identifiers, and cross-posting.

Capability absence must be visible. Adapters use the shared safe fetcher.

**The transport is handed in, never held.** An adapter receives its fetcher for the call it is making (`preview`, `fetch_chapters`, `fetch_chapter`) and constructs, owns and configures no client of its own: no transport field, no constructor taking an endpoint or a timeout, no route to a host its `hosts()` does not name.

This is a requirement rather than a style preference, because the alternative fails quietly. An adapter that built its own client would still compile, still satisfy the trait and still pass its own parser tests, while bypassing every guard in §11.5 — SSRF refusal, robots compliance, per-domain pacing, credential handling — and nothing in review would show it, because the bypass sits inside a call that looks ordinary. Handing the fetcher in makes those guards unreachable by construction rather than by discipline.

It is also what makes §11.7's fixtures possible. `preview_from_html` and `chapters_from_html` parse a recorded page with no transport in sight, so a parser test is an offline test and stays one.

## 11.2 Destinations

Explicit choice:

- Private library.
- Own draft.
- Republication with permission.
- Approved preservation batch, for authorized operators.

Default URL imports to private library. A source login demonstrates access, not permission to republish.

## 11.3 Entry points

- Paste-a-URL box on home and library pages.
- `/library/imports/new`.
- File upload.
- Command-palette action.
- Drag-to-bookmarks-bar bookmarklet.
- PWA share target where supported.
- Clipboard paste of raw text.
- Drag-and-drop text selection.

The bookmarklet transfers only the selected page URL, does not scrape cookies or content, opens Lorehaven's confirmation screen, never imports through state-changing GET, and warns about source URLs containing private tokens.

Prefer transient client-side transfer and immediate URL cleanup for sensitive URLs. Redact importer entry URLs from access logs.

## 11.4 Workflow

```text
URL/file/text → source detection → source status and authentication check
→ metadata preview → destination selection → confirmation → queued import
→ chapter fetching → sanitation → duplicate/update review → completed library item
```

## 11.5 Safe fetching

Reject unsupported schemes, loopback, link-local, private-network, and metadata-service targets. Validate every redirect. Address DNS rebinding. Limit bytes, time, redirects, decompression, concurrency. Strip or proxy unsafe embedded resources. Avoid forwarding credentials to unrelated origins. Respect source rate limits. Redact credential material.

A source credential does not justify an SSRF exception. Trusted administrative integrations that need internal destinations use separate explicitly configured capabilities.

Do not implement CAPTCHA, paywall, or access-control circumvention.

### Rate limits come from the source, and the floor is one request a second

"Respect source rate limits" means the source's published number, not one chosen here. For every host an import reads:

- Fetch that host's `robots.txt` and read it as the source's own statement of how it wants to be read. Its `Crawl-delay` sets the minimum gap between requests to that host. Adapter-declared intervals are a fallback, not an override: the operator of a server knows what it can take, and a number in this repository is a guess that goes stale.
- Honor `Disallow` as a refusal, not a warning. A path the source forbids is not fetched, and an import that needs it reports the restriction rather than a parse failure. Matching is the de-facto standard's: `*` and trailing `$` supported, most-specific rule wins, `Allow` breaks a tie, and a group naming our product token applies ahead of the `*` group. The token is our `User-Agent`'s leading word (`Lorehaven`). **An instance operator may switch this off** — see below.
- Use **one request per second** when no delay is published. "No information" must not be read as "no limit", and absence is not permission to go faster. One second is also the floor for a host whose published delay is shorter or unreadable, so a malformed directive can never become a faster pace.
- Treat a `404` or `410` for `robots.txt` as a site with no restrictions. Treat any other failure to read it as rules unknown: proceed at the default pace, and record the condition against the source's health rather than refusing a reader's import over a file that is temporarily broken.
- Read it once per host per import run, and cache it for a bounded interval when a process outlives one run.

This is enforced inside the shared fetcher, for the same reason the address checks are: an adapter that could opt out of pacing would make the rule advisory. An import is resumable, so a slow import is a cost the reader can wait out; an import that hammers a volunteer-run archive is a cost somebody else pays.

#### The `Disallow` override is the operator's, and it is narrow

`robots.txt` is a convention between a crawler and a host, and the operator of an instance is the party who answers for that instance's crawling. So an instance may be configured not to honour `Disallow` (`imports.honour_robots`), and it honours it by default.

The override is deliberately narrow, and each limit is a rule rather than an implementation detail:

- **It does not touch pacing.** `Crawl-delay` from the same file, and the one-second floor beneath it, are read and enforced exactly as before. A permission question and a load question arrive in one file; answering the first differently says nothing about the second. An instance that overrode the permission and then hammered the host would have converted a lost permission into a lost address.
- **It is not access-control circumvention.** This section's prohibition stands untouched. `robots.txt` states what a host wants crawled; it is not authentication, and nothing here defeats a login, an age gate, a paywall, or a challenge. A page the host's own code gates remains out of reach on every setting.
- **It is instance-wide and visible.** A per-source switch would let an override be made once and forgotten about for the source it affects, so it is a single instance-level setting that shows up in the configuration an operator reads. It is reported on the source's catalogue entry rather than being invisible in a log, so a reader on such an instance can see how it is configured.
- **Every overridden read is counted, and the first per host is logged.** An operator who switched it off has to be able to say what it cost, and a number is the smallest thing that answers that.

The reason this is a configuration value and not a build flag: it is a judgement about *whose* instance is doing the crawling. An operator may reasonably conclude that an archive which forbids the whole site while serving a public reading view did not aim its rule at a person importing one work to read privately — but that is the operator's judgement to make, on their own instance, and it must be made where it can be seen rather than assumed by a default.

## 11.6 Per-source credential vault

Support source authentication only for adapters with a documented authentication method. Prefer source-issued tokens or scoped credentials. Password or session-cookie storage requires explicit consent and adapter-specific documentation.

Vault behavior:

- Pseud-scoped by default.
- Encrypted at rest.
- Plaintext never returned by an endpoint.
- Decrypted only for the required operation, briefly.
- Default expiry of 30 days or the source's earlier expiry.
- Shorter configurable expiry.
- Immediate revocation.
- Re-consent for renewal.
- Origin-bound credential use.
- Audit events without secret contents.
- No automatic copying across pseuds.
- No credentials in job payloads, exports, logs, traces, or analytics.

Expired credentials pause affected jobs with actionable status. Do not repeatedly retry authentication failures.

```text
GET    /api/v1/source-credentials
POST   /api/v1/source-credentials
DELETE /api/v1/source-credentials/:id
POST   /api/v1/source-credentials/:id/test
```

Responses expose metadata only. Deleting a credential does not automatically delete already imported copies.

## 11.7 Initial formats and adapters

Local file import first: plain text, HTML, EPUB, DOCX, Markdown.

Then implement website adapters individually.

For each adapter:

1. Document recognized URLs.
2. Declare capabilities.
3. Add representative fixtures.
4. Implement metadata parsing.
5. Implement chapter enumeration.
6. Implement extraction.
7. Test malformed input.
8. Test updates.
9. Test authentication where supported.
10. Perform live verification when permitted.
11. Record evidence and status.

Do not promise a source count in advance. Adapter counts are an outcome of verified implementation, never a marketing claim.

## 11.8 Runtime source health

Health states: `unknown | healthy | degraded | unavailable | paused`.

Distinguish failures: source outage, rate limiting, authentication failure, parser incompatibility, policy restriction, network failure, user-specific access denial.

Rolling outcome windows, bounded retry, per-domain concurrency, circuit breakers, operator pause/resume, recovery probes, user-visible incident messages, adapter-version attribution.

Do not label a source unavailable because one user's credentials expired. Public source status omits private import volumes, credential use, and user identities.

## 11.9 Author bibliography imports and watches

Accept supported author, user, or bibliography URLs.

```text
identify bibliography → enumerate resumably → preview discovered works
→ select/filter → estimate size and limits → confirm destination
→ enqueue child imports → show per-work progress
```

Support select all or subset, exclude already imported, update existing, cancellation, resume, per-entry retry, pagination and enumeration limits, rate-limit-aware scheduling.

**Author watches:** an existing library item or explicit bookmark of an author URL can be marked as watched. The scheduler checks periodically, respecting source rate limits, and queues new works for private import with notification. Watch frequency is bounded and configurable.

Bibliography importing does not bypass permission requirements or default to republication.

## 11.10 Cross-source identity

Distinguish duplicate imports from the same source, confirmed cross-posting, different editions, translations, adaptations, similar but unrelated works.

Users may privately group their own copies without changing the public catalog. Public identity merges require evidence and review.

A unified story page may list public source editions and the viewer's own private copies. It must never expose another user's private holdings.

Merges preserve source records and provenance, do not combine private notes or ownership, do not grant access to another edition's body, are reversible, preserve redirects and decision history.

## 11.11 Preservation and archive migration batches

Support archive-migration workflows without implying affiliation.

Require authorized operator role, import manifest, source archive identity, documented permission or reviewed legal basis, original author attribution, source URLs and timestamps, destination collection, claiming and correction procedure, dry-run report, idempotent execution, duplicate review, batch rollback strategy.

Never assign imported authors to local accounts solely by matching names or email strings.

Preservation batches may initially remain private or review-only. Public release is a separate approval step.

A preservation batch captures text and metadata. Media bytes are captured only where the instance hosts media (§30.2); elsewhere the reference is preserved as a reference, so a preservation record survives the loss of its origin without this instance having mirrored anything.

Where the instance retains work bodies as references rather than copies (§11.15), a preservation batch is refused rather than run in a reduced form. Capturing text is what such an instance has decided not to do, and a batch that ran anyway as metadata-only would be the same request answered two ways depending on who asked.

## 11.12 Cross-posting to external sites

Support publishing local Lorehaven works to external sites through adapters that provide cross-posting capability.

Requirements:

- Explicit user authentication with the destination site.
- Per-post confirmation with preview.
- Permission handling at the destination.
- Cross-post status tracked per work per destination.
- Adapter-specific limitations documented (some sites do not accept certain markup).
- Failed cross-posts do not affect the local work.
- Cross-post credentials use the same vault as import credentials.

Cross-posting is a manual per-work action. Automatic cross-posting of every published work is not offered; users must confirm each destination.

## 11.13 Updates

Never overwrite imported copies destructively. Create a new snapshot and preserve notes, shelves, bookmarks, ratings, and progress where mappable. Show review notices for removed, reordered, or substantially changed chapters.

## 11.14 Import result quality

§11.8 grades the **source**; this grades the **result**, and the two must not be conflated. A source that answers every request promptly while every answer is a challenge page is `healthy` by §11.8 and useless in fact. Health says whether to keep asking; quality says whether what came back is a work.

Every fetched candidate is classified before it is offered:

- **accepted** — a real title, a real author, a non-zero length.
- **rejected** — certainly not a work: an empty or placeholder title or author, or canonical filler text in a metadata field. A rejection carries its reason, and the candidate is never imported silently.
- **held** — no confident call could be made. Held for a person rather than deleted.

A zero word count is **held, not rejected**. A real work with no counted words exists, and a rule that called it junk would lose it without trace.

Norms:

- Classification is a pure function of the fetched metadata, so it is testable without a network and answers identically in the preview, the import and the update check.
- The default is *reject the obvious junk, hold the rest*: a rejected candidate is a reason shown to whoever asked, and a held one waits for a decision.
- A rejection is never an outage. One unreadable page does not trip §11.8's circuit breaker, does not mark the source degraded, and does not abandon the other candidates in the same import.
- The reason is recorded on the job's report beside the candidate's URL, so an operator sees what was refused and why rather than a count.
- Quality never overrides eligibility (§7.6). A readable, eligible work is imported whether or not it looks impressive, and an ineligible one is refused however immaculate it looks.
- An adapter may contribute source-specific evidence — a marker the site itself uses for an unposted or withdrawn work — and the shared classification remains the floor beneath it. Evidence may harden a rejection; it may not turn a rejection into an acceptance.

## 11.15 Work body retention

An instance that caches what it imports and an instance that is a catalogue of links are both complete instances, and which one this is must be the operator's decision rather than a consequence of how a work happened to arrive.

```text
Work body retention: cache | aggregate
```

- **`cache`** — the fetched body is stored on this instance and served from here. A library item is a durable snapshot (§10.4.1) referenced by that reader's copy, so the work reads here, downloads here, and reads offline. This is the default.
- **`aggregate`** — metadata, attribution, provenance and canonical links are stored; the body is never fetched into this instance's storage. A library item is a reference to the work at its origin. The work page presents the work — title, author, summary, tags, fandoms, statistics, comments, ratings — and links out for the text.

Both modes produce a real work record. Eligibility (§7.6), metadata search, tags, fandoms, characters, series, collections, challenges, ratings, comments, bookmarks, notifications and the people directory all behave identically; what differs is whether this instance holds the words.

Two values, not three, and not the words §30.2 uses. The media setting needed a third because media has a player shell to omit; text has no shell, so a third value here would be a state with no behaviour behind it. §30.2 states the reasoning in full.

The setting is the operator's, and it is stated once. It applies to the instance, never to a request, a work, an importer, an extension or a federated peer: nothing an uploader, an adapter or a peer does may raise it. An instance set to `aggregate` refuses to store a body wherever a body could arrive — a URL import, a file upload, a clipboard paste, an authorized preservation batch (§11.11), a federated announcement, or a cache fill — and every refusal names the instance's policy rather than failing as a generic error.

An operator may set retention per source family, and an override may only narrow. Caching most sources while aggregating one is expressible; the reverse on an `aggregate` instance is not, because that would restore storage the instance decided against, and the operator who wants it can change the instance setting itself, where the change is recorded. Overrides are recorded in the modlog with who set them and when.

**Neither mode degrades silently, in either direction.** On a caching instance, a body that fails to fetch is a failed import that retries and eventually reports — never a work quietly reclassified as a link, because that turns a temporary source failure into a permanent loss of something the reader asked this instance to keep. On an aggregating instance, nothing fetches a body "because it was available": a bulk import of four hundred works does not become forty gigabytes because the adapter could have managed it.

Consequences that follow from holding no body, stated so that they are decisions rather than surprises:

- **Body search covers what this instance holds.** §15.9 indexes permitted body text; an aggregated work has none to index, so it is found by title, author, tags, fandom and summary, and a body-only search cannot match it. Metadata-only and combined modes are unaffected.
- **Offline reading and exports cover what this instance holds.** An aggregated work cannot be downloaded for offline reading (§13.5) and cannot be exported (§13.1–13.3), because there are no chapter bytes to package. The action is absent or honestly disabled with the reason shown, never a download that produces a file containing a link.
- **A preservation batch is refused.** §11.11 exists to capture text, and an instance set to `aggregate` has said it does not want captured text. The refusal names the policy.
- **Quotas and storage accounting reflect it.** An aggregated library item consumes metadata, not bytes, and an operator's storage figures say so.

**What `aggregate` does not mean.** It is not a fallback, not a degraded mode, and not permission to lose the record: provenance, source URL, timestamps, attribution and availability checking (§11.13) all remain, and a source that vanishes leaves the record marked unreachable rather than deleted. It is also not a way around §11.5, §11.6 or §15.16 — the same safe fetching, credential handling and tag curation apply to whatever is fetched to build the metadata.

An instance may change the setting. Widening to `cache` does not retroactively fetch bodies for works already aggregated; that is a per-work action somebody takes, recorded, and bounded by the same source rate limits as any other import. Narrowing to `aggregate` does not delete bodies already held: an existing reader's snapshot is content this instance stored for them, and removing it is the deletion workflow (§10.4), not a policy change.

```text
GET    /api/v1/admin/retention/policy
PATCH  /api/v1/admin/retention/policy
GET    /api/v1/admin/retention/sources
PUT    /api/v1/admin/retention/sources/:sourceKey
```

| Table | Important fields |
|---|---|
| `instance_retention_policy` | body_mode, updated_by, updated_at |
| `instance_retention_source_overrides` | source_key, body_mode, updated_by, updated_at |

## Acceptance

- Repeat imports avoid accidental duplicates.
- Failed chapter fetching resumes.
- External content never becomes public automatically.
- Malicious archives cannot escape extraction storage.
- Unsupported sources excluded from support counts.
- Credentials cannot leak through redirects.
- Expiry pauses rather than loops jobs.
- Batch progress survives restart.
- Identity merging does not alter content permissions.
- Preservation dry runs do not publish anything.
- Author watches respect rate limits and can be paused.
- Cross-posting requires explicit destination authentication.
- An unreadable page is a rejected candidate carrying its reason, not a degraded source and not a lost import.
- An instance set to `aggregate` imports a work into a complete record with metadata, attribution and a canonical link, and stores no body.
- An instance set to `aggregate` refuses a body from every path that could deliver one — URL import, file upload, paste, preservation batch, federated announcement and cache fill — and each refusal names the policy.
- A body that fails to fetch on a caching instance leaves a failed, retryable import and never becomes a silent link.
- An aggregated work is found by metadata search, tags, collections and series, and is not matched by a body-only search.
- An aggregated work offers no offline download and no export, and says why instead of producing a file containing a link.
- An operator may narrow retention for one source and may not widen it on an aggregating instance.

---

# 12. Milestone 7: Positivity Filter and Feedback Delivery

Priority 3 (positivity-first) has its own milestone because it constrains every feedback surface.

## 12.1 Positivity model

Every comment, review, and forum reply on a work or author-facing surface passes through a classification pipeline before the author sees it:

```text
submitted → classified → held for review or auto-delivered → author-visible or hidden
```

Classifications:

- **Positive:** appreciation, encouragement, non-critical engagement.
- **Constructive critique:** identifies specific craft issues respectfully.
- **Ambiguous:** insufficient signal to classify confidently.
- **Negative/destructive:** hostility, personal attacks, gratuitous harshness, spam.

## 12.2 Classification implementation

Layered approach:

1. **Rules and heuristics** (deterministic, fast, offline): profanity signals, harassment patterns, all-caps ratios, known hostility phrases in the requesting locale.
2. **Optional AI classifier** (when configured): a plug-in provider returns a confidence score for each classification. Costs credits or subscription budget. Ollama is a supported provider.
3. **Author self-hosted rules** (advanced): authors may add per-work keyword filters.

When no AI provider is configured, classification falls back to rules and heuristics with a wider "ambiguous" band that routes more comments to moderator review.

## 12.3 Delivery rules

Based on the author's feedback preferences (Milestone 8):

- **Positive comments:** delivered directly to the author with a "positive" indicator.
- **Constructive critique:** delivered only if the author has opted in to critique; otherwise held or hidden.
- **Ambiguous:** held for community moderator review unless the author has enabled auto-delivery for ambiguous.
- **Negative/destructive:** hidden from the author, held for moderator review, and never surfaces publicly on the work page.

Public visibility of a comment on the work page follows the same rules. A negative comment is not silently allowed to appear publicly while being hidden from the author only.

## 12.4 Commenter experience

The commenter is not told their comment was classified as negative. They see:

- "Comment posted." (positive, constructive-accepted, ambiguous auto-delivered)
- "Comment held for moderator review." (ambiguous held, negative)

The commenter can edit or delete their held comment. They cannot see the classification score.

Repeated negative comments trigger rate limits and eventually account-level moderation review.

## 12.5 Quorum review of classification

When a comment is held, it enters the moderation queue. Trusted community moderators (Milestone 13) review it. A single moderator can:

- Approve for delivery.
- Reject and hide.
- Reclassify (for classifier training data).

A second moderator's confirmation is required for accounts with a pattern of held comments (repeated review or sanction).

The line between constructive and destructive is set here, by the moderator quorum, not by the author or the automated classifier.

## 12.6 Author overrides

Authors may:

- View a "held feedback" queue for their own works (opt-in, off by default). This shows only comment presence and category, not text, unless the author explicitly requests to read it.
- Whitelist trusted commenters (their comments skip classification for that author).
- Blacklist commenters (all their comments are hidden without review).
- Disable comments entirely on a work.

Authors cannot override moderator decisions to hide destructive comments from the public work page. They can only decide whether to read the content of held comments themselves.

## 12.7 Appreciation and quick reactions

Quick reactions (Milestone 9) and appreciation notes bypass the positivity filter because they are pre-classified positive. Anonymous thank-you notes go through a lighter check (spam and known-hostile-account filters).

## 12.8 Forum and community context

Forum posts, group discussions, and general community messages have their own positivity policy, weighted less strictly than direct author feedback. The default is that constructive disagreement is allowed in forum threads. Direct hostility and personal attacks are still filtered.

Forum quorum moderation applies to disputed posts.

## 12.9 Author-visible feedback view

Authors have a dedicated view separate from public work-page comments:

- Positive comments received.
- Constructive critique (if opted in).
- Aggregate reaction counts.
- Anonymous appreciation notes.
- Cheer count for WIPs.

Notifications for author feedback default to positive-only.

## 12.10 Author boundary tools

The filter (§12.1) handles the worst of reader behavior after it happens. Authors also need to set expectations before it does, and every tool here is framed as author comfort rather than reader punishment.

- **Per-work author note:** a short, prominent note the author writes — posting schedule, whether update questions are welcome, whether the work is complete. Displayed where a reader decides to comment, not buried in a settings page.
- **Per-work comment policy:** the author states that update requests, unsolicited concrit, or reader-suggested directions are not wanted. The interface says so before the reader types, and a reply that ignores a stated policy is still delivered but marked for the author, who decides — nothing is filtered silently on the author's behalf.
- **"No requests" flag on the profile:** an author-level statement that they do not take prompts, bounties or writing requests. Request surfaces (§16.9, §18.5) then exclude that author from matching and from being named as a candidate.
- **Comment throttle per work:** a per-reader-per-day limit the author sets, implemented by the same machinery as rate limiting (§3.8). Reaching it is answered with a clear explanation and a time, never a silent drop.
- **Cooling-off mode:** for a work receiving intense attention, the author may batch and slow comment delivery and notifications. The comments still arrive, in a digest, so nothing is lost and nothing is censored.
- **Pause comments entirely** on a work, reversibly, with existing comments retained and the state shown to readers.

None of these tools hides a reader's words from the author, and none is a sanction: they are the author's own boundary, applied by the author, recorded as such, and reversible at any time.

## Acceptance

- Destructive comments never appear on public work pages.
- Destructive comments never reach the author without their explicit opt-in to read held comments.
- Commenter is not told which classification their comment received.
- Rules-only mode functions without an AI provider configured.
- Moderator queue processes held comments.
- Author whitelist and blacklist take effect immediately.
- Reactions and appreciation notes bypass the filter.
- Repeated negativity from one account triggers rate limits.
- Public work-page comment visibility matches author-visible comment set (for public comments).

---

# 13. Milestone 8: Exports, Device Delivery, and Offline Reading

## 13.1 Export order

Plain text, sanitized standalone HTML, Markdown, EPUB, PDF converter, AZW3 converter, MOBI converter.

Markdown export documents which rich-text features are simplified.

For converters: discover availability at startup, build fixed arguments, no unsafe shell interpolation, enforce time/memory/output limits, disable unavailable formats with installation guidance, record converter version in verification evidence.

Format support and device-delivery support are separate claims.

## 13.2 Export behavior

Exports include title and author attribution, chapter ordering, table of contents, language, source provenance, edition/snapshot information, applicable license or permission statement, user-selected typography where supported.

Private notes excluded by default. Generated download URLs are authenticated or short-lived and scoped.

## 13.3 EPUB validation

Validate ZIP structure, MIME declaration, package metadata, navigation, chapter order, Unicode, attribution, source provenance.

## 13.4 Send-to-Kindle and device email

Optional export-delivery adapter using SMTP.

```text
add device address → verify ownership → configure approved sender if required
→ choose work and format → review privacy notice → queue export and delivery
→ show delivery status
```

Device addresses are private. Destination changes require confirmation. Rate and size limits. No arbitrary public mail relay. No untrusted user-controlled mail headers. Attachment and export limits. Bounce/failure handling. Revocation and address deletion. Explicit disclosure that the delivery provider receives content.

Use a format supported by the destination's current documented delivery workflow. "SMTP accepted" is not "arrived on device."

## 13.5 PWA

Web manifest, icons, service worker, IndexedDB schema, download manager, offline reader route, update notification, supported-platform share target.

| Data | Strategy |
|---|---|
| Versioned assets | Cache-first |
| Public dynamic data | Network/revalidation |
| Explicitly downloaded works | IndexedDB |
| Private authenticated responses | No blanket shared caching |
| Reading progress | Local first, queued synchronization |

## 13.6 Offline privacy

On logout, offer removal of local private downloads with a privacy-protective default on shared devices.

Explain that browser storage may be evicted, downloaded copies cannot always be remotely revoked, device access can expose offline content, service-worker caching is not encryption, account deletion does not guarantee deletion from disconnected devices.

## Acceptance

- Downloads open without a network.
- Missing chapters show actionable offline states.
- Interrupted downloads recover.
- Quota errors preserve existing downloads.
- IndexedDB upgrades tested.
- Private data does not cross accounts or pseuds.
- Foreground synchronization works without Background Sync.
- Device delivery cannot be used as an open relay.
- Unavailable converters honestly disabled.
- A work whose body this instance does not hold offers no offline download and no export, and says why rather than producing a file that contains a link.

---

# 14. Milestone 9: Library, Saved Views, Bookmarks, and Updates

## Routes

```text
/library
/library/imports
/library/imports/new
/library/imports/:id
/library/import-batches/:id
/library/shelves
/library/shelves/:id
/library/bookmarks
/library/downloads
/library/history
/library/reading-stats
/library/goals
/library/views
/library/views/:id
/library/source-credentials
/library/watches
```

## 14.1 Core library features

Shelves, reading statuses, private tags, bookmark notes, batch actions, source filters, update checking, duplicate review, cross-source edition grouping, storage usage, import provenance, source health, credential-expiry notices, watch management, CSV export and import of bookmarks.

Default bookmarks to private. Deleting a shelf does not delete its works.

Bookmark CSV format is documented and stable. Import handles duplicates, missing works, and permission errors gracefully.

## 14.2 Saved searches and named views

A saved view contains `name`, `versioned_query_ast`, `sort`, `scope`, `display_preferences`, `pinned`, `visibility`.

Support save current search, rename, duplicate, pin to navigation or dashboard, edit through visual filters or query syntax, delete, optional public sharing of safe public-work views.

Private-library views are private. Public view serialization must not expose private shelf IDs, notes, reading state, or hidden pseud information.

Relative filters ("updated in the last seven days") retain relative meaning. After a search-schema upgrade, migrate the AST or show an explicit repair state.

Saved views can produce RSS feeds through the same scoped token system as other private feeds.

**Saved-search alerts.** A saved view may run on a schedule and notify the
reader when new eligible works match it — "tell me when someone posts a
completed slow-burn under 20k in this fandom." The alert runs the saved query
with the reader's own permissions at run time, so a match that has become
ineligible is not reported. Results arrive in the notification stream (§23.3)
with the count and the view's name, never the works' full metadata in the
notification itself. Alerts are per-view, bounded in frequency (daily by
default), pausable, and earn no credits. They are private: nobody, including
the author of a matched work, is told that a view matched.

**Pinned searches.** A saved view may be pinned to the dashboard or sidebar, where it renders as a live widget — the first page of current results, not a link — with its own default sort. Sorts are per saved query: last updated, date published, completion date, word count, appreciation count, public bookmark count, title, author, or random; the view states its sort like any other filter.

## 14.3 Batch outcomes

```json
{
  "succeeded": ["..."],
  "failed": [{"id": "...", "code": "ACCESS_DENIED"}]
}
```

## Acceptance

- Public bookmark lists exclude private entries.
- Removing an item distinguishes deleting a reference from deleting a private copy.
- Source update checking respects rate limits.
- Pinned views survive refresh and pseud switching correctly.
- Shared views cannot expose private filters.
- History and statistics deletion controls work independently of library ownership.
- Watches can be paused and resumed.
- CSV import/export preserves user data without leaking cross-user information.

---

# 15. Milestone 10: Structured Taxonomy, Mood Search, Query Language, and Fuzzy Matching

## 15.1 Character assertions

```text
Character A:
  prominence = protagonist
  roles = [mentor]
  attributes = [vampire, BAMF]
```

## 15.2 Relationship assertions

```text
Participants = [A, B]
Kind = romantic
Prominence = central
Dynamics = [enemies_to_lovers]
```

Support more than two participants.

## 15.3 Typed query AST

```text
And | Or | Not | Text | Phrase | WorkField
ExistsCharacter | ExistsRelationship
TagMatch | MoodMatch | MetadataCompleteness
```

Example:

```json
{
  "and": [
    {"exists_character": {
      "character_id": "A",
      "prominence": ["protagonist"],
      "attributes_all": ["BAMF"]
    }},
    {"not": {"exists_relationship": {
      "participant_any": ["A"],
      "kind_any": ["romantic", "sexual"]
    }}}
  ]
}
```

Compile bound predicates into SQL `EXISTS` clauses. Never allow one character to satisfy another character's attributes.

## 15.4 User-facing query language

Quoted phrases, `AND`/`OR`/`NOT`, parentheses, leading `-` exclusions, fielded search, documented escaping, autocomplete and error locations.

Initial fields:

```text
title: author: fandom: character: relationship: tag: mood:
summary: body: language: status:
```

Examples:

```text
fandom:"Example Fandom" AND title:"winter"
body:"we were never alone" -tag:"major character death"
mood:comfort AND status:complete
```

Define operator precedence and implicit conjunction explicitly. Parser produces the same typed AST as the visual filter builder. Malformed syntax produces helpful errors. Never pass user query text directly as SQL.

**Query aliases.** A reader may define a macro — `myfandom:` expanding to `fandom:x fandom:y tag:z` — as a named shorthand over the same AST. Aliases are private, expand before parsing, cannot shadow the built-in fields, and are expandable in the interface so a shared search link never depends on the recipient's aliases.

## 15.5 Fuzzy matching and typo tolerance

Misspelled tags, author names, and fandom names produce "Did you mean?" suggestions rather than silent auto-correction.

Implementation:

- SQLite: trigram-based similarity or FTS5 with distance ranking.
- PostgreSQL: `pg_trgm` extension for similarity queries.
- Suggestion threshold configurable per query type.
- Never auto-execute a corrected query; the user must confirm.

Fuzzy matching applies to tag names, author handles, fandom names, and work titles in search input. It does not apply to body text (which uses exact phrase matching).

**Ephemeral "for now" filters.** Above every feed and result list the reader can pin a temporary filter — "completed works only, this session" — without editing any saved view. It is one tap, visibly labeled as temporary, and clears with the session; it never writes to a recipe or a saved query.

## 15.6 Metadata completeness

Negative filters support:

- Match declared metadata.
- Require sufficiently complete metadata.
- Include uncertain results separately.

Missing ship metadata is not proof that a story contains no ship.

## 15.7 Filters

Characters and prominence, relationships and prominence, relationship kinds and exclusions, character attributes and roles, fandom and crossovers, completion, word/chapter ranges, rating and warnings, language, dates, author, collection, series, tropes, settings, moods, content notes (worldbuilding-heavy, dialogue-driven, etc.), length histogram, public works/private library scope, read/unread, user mutes.

Bound query depth, clause count, result windows, execution time.

**Sorts.** Every search presents an explicit sort menu with a stated default of
best match for text queries and last updated for browses: last updated, date
published, completion date, word count (with the §15.15 runtime substitution
for media works), appreciation count, public bookmark count, public comment
count, title, author name, and random. Exact sorts are exact: a sort by word
count orders strictly by word count and is blended with no quality,
freshness, or taste signal (§15.10, §16.2). Random is a stable shuffle per
request window, so paging does not reshuffle underneath the reader. Where a
sort draws on a public aggregate, works below the display threshold of §9.5
sort at the end rather than as zero.

**Additional freeform axes.** Beyond mood (§15.8), an instance may enable
further curated classification axes — genre, AU type, POV, tense, narrative
style — as first-class, filterable, canonicalizable tag namespaces. Each axis
follows the tag workflow (§15.11), is proposed and quorum-reviewed like any
taxonomy extension, feeds autocomplete, and can be hidden by readers who do
not want it. Axes are opt-in per instance because every axis a reader must
wade past in the filter panel is a cost paid on every search.

## 15.8 Mood and tone taxonomy

Curated initial mood tags: comfort, angst-with-happy-ending, angst, cozy, adventure, slow-burn, fast-paced, episodic, dark, hopeful, humorous, bittersweet, catharsis.

Authors assign moods to their works. Readers filter by mood. Community proposals extend the taxonomy through quorum.

Moods are distinct from plot tags: they describe reading experience, not plot elements.

## 15.9 Full-text body search

Index permitted body text for published eligible local works, the requesting pseud's private imported copies, and explicitly authorized public preservation content.

Phrase search, prose and dialogue search, chapter-level matches, highlighted snippets, jump-to-match anchors, search within one work, explicit metadata-only/body-only/combined modes.

Body search does not require a globally shared body cache. Permission checks apply before counts, facets, snippets, and result serialization. Inaccessible content must not leak through counts or autocomplete.

A work this instance retains as a reference (§11.15) has no body here to index. It stays findable by metadata, and a body-only search does not match it — which is the honest answer, not a gap to paper over by indexing a summary as though it were the text.

Sanitize highlighting output. Strip executable markup before indexing. Index revision state so stale snippets can be invalidated.

## 15.10 Ranking

Only primary tags add tag-ranking boosts. Secondary tags remain filterable. Exact search is not taste-steered. Ordering on browse surfaces is governed separately by §43: `best-match` and any literal query are exempt from `for-you`, and the quality ordering shown here is what §43 calls `top`. Document scoring differences between database backends while keeping filtering and authorization consistent.

Additional signals for non-exact discovery:

- Completion rate.
- Aggregate positive-feedback ratio (reactions and public positive comments).
- Recency and update velocity.
- Author bibliography quality signals.

## 15.11 Canonicalization and metadata correction

Tag workflow:

```text
proposal → review → quorum → apply → reindex → preserve redirect/history
```

Cross-source identity proposals use an equivalent workflow.

Metadata correction supports title, author attribution, summary, completion status, language, source mapping, taxonomy assertions.

Authors control their authored work metadata under moderation policy. Curators cannot silently rewrite author text. Private imported metadata may be corrected locally without changing shared records. Source updates do not silently discard local overrides. Every shared correction records provenance and history.

**Rating checks:** automated heuristics flag works whose content appears inconsistent with declared rating. Flags produce suggestions, not automatic changes. The author is notified. Quorum reviews if the author disputes or ignores.

Auto-tag suggestions use deterministic rules first and optional AI later. They enter a review queue.

## 15.12 People directory

`/people` with A–Z browsing, handle/display-name search, fandom filtering, author/reader role filters where voluntarily declared, pagination, discoverability controls.

Include only pseuds that permit directory listing. Do not infer fandom interests from private reading or imports.

## 15.13 Fandom landing pages

`/fandoms/:fandom_id` with:

- Recent additions.
- Popular completed works.
- Community picks.
- Active challenges and requests.
- Recommended entry points ("where to start").
- Fandom statistics (public works, active authors).

Population is algorithmic with optional curator overrides. Entry-point recommendations may be community-nominated through quorum.

Landing pages respect all content eligibility and positivity constraints.

## 15.14 Zero-result demand insights

Optional privacy-preserving curator view of unmet search demand.

Clear collection setting and policy, exclude private-library and sensitive queries by default, avoid storing raw queries when aggregate buckets suffice, redact likely personal data, suppress low-count cohorts, bound retention, no public list of individual searches, no automatic import/publication/challenge creation.

External AI clustering requires separate explicit consent. Label clusters as observed demand, not proof that content does not exist.

## 15.15 Length histogram in search

Show a histogram of matching works by length. Filter by "under 5k", "5k–20k", "20k–100k", "100k+". Helps readers find fiction that fits available time.

## 15.16 Content notes

Four rating values are too coarse to decide with. Content notes are the fine-grained statement of what is *in* a text, as distinct from what it is *about*: tags describe subject matter, content notes describe what a reader will encounter.

The vocabulary is a controlled list, seeded with the conventional set and extensible per instance:

```text
graphic violence | on-screen character death | major character death | self-harm |
eating disorder | suicide | parental abuse | domestic abuse | sexual assault |
child endangerment | addiction | medical trauma | pregnancy loss | torture |
unreliable narrator | ambiguous ending | unhappy ending | claustrophobia |
vomit | insects | needles | drowning
```

Rules:

- **Content notes are a separate axis from tags.** A note is not a tag, is never merged into tag search, and does not appear in a tag cloud. A work is tagged `enemies to lovers`; it is noted `graphic violence`.
- **Notes are behind a reveal by default.** A reader opens them deliberately, so a note cannot spoil a story they were about to enjoy, and a reader who needs them finds them in the same place on every work.
- **Reader-configurable:** a reader marks notes they always want surfaced (shown open, without a click) and notes they never need warned about (collapsed entirely). The setting is per account and applies everywhere.
- **Author-provided by default**, with a distinguished "the author states there are no notes" state, which is not the same as "no notes were provided". The difference is real and the interface shows which one it is.
- **Community-suggested and quorum-reviewed** where the author provided none or few. A suggestion is evidence plus a note from the vocabulary; it is added by quorum (§19.4), attributed as community-provided, **shown to the author before it is shown to readers**, and removable by the author with a recorded reason.
- Content notes feed the filters (§15.7), saved views (§14.2) and the first-run flow (§7.8), and are the context the positivity filter uses when judging whether a comment is reacting to unmarked content (§12.2).
- A note is never a judgement of the work, and no surface frames one as advice against reading it.

## Acceptance fixture

Include contrasting works: A protagonist/B vampire, A vampire protagonist, A supporting vampire, A/B central, A/B background, A with unknown relationship metadata, A with confirmed no romantic/sexual relationship, a private imported body containing a unique phrase, a withdrawn work containing the same phrase, works with each mood tag.

Test both databases for correct binding, negative-filter semantics, query parser round trips, phrase search, snippet sanitation, permission-safe counts, index invalidation, query complexity limits, mood filtering, fuzzy suggestion accuracy.

---

# 16. Milestone 11: Discovery, Taste Influence, Recipes, and Dashboards

Priority 4 (admin-enjoyable without monothematic) is centered here.

## 16.1 Baseline engines

Recent, trending by time window, content similarity, also bookmarked, similar by bookmarks, Blind Date, user-preference matching, community-suggested similarity, completion-rate weighted, mood-matched, cross-fandom dynamic matching, complete-and-under-read.

Each engine implements:

```text
generate_candidates(context, limit) → candidate IDs and baseline scores
```

All candidates pass shared eligibility rules.

Private bookmark and rating data are not silently pooled. Use explicitly permitted signals, aggregation thresholds, documented retention.

**Feed reason transparency.** Every feed response may include, per item, the recipe reason — which of the reader's own recipe terms matched ("because: fandom:HP, tag:time travel"). This is the `reason` field of the engine contract (§16.1); surfacing it to its owner is the default. It is explicitly distinct from the administrator-influence secrecy rule (§0.3): truth about one's own feed costs nothing, and §16.5's dial governs the part that is not the reader's own. A reason may name an *instance-level* term only for a topic declared `public = true` (§0.4); otherwise it degrades to one undifferentiated line (§16.16.2).

## 16.2 Administrator taste profile

Explicitly selected taste-source profile. Exclude moderation sessions, troubleshooting, import tests, accidental opens, activity marked private-from-learning.

Signals: explicit likes/dislikes, selected bookmarks, private ratings, optional completion events, admin seed prompts, wishlist items.

One profile, recomputed on demand and inspectable by its administrator. Long-term and recent behavior are both inputs; the blend is an implementation detail, not a published number, and it is not tuned before there is evidence to tune against. Profile versions are kept so an administrator can see what changed and when.

## 16.3 Influence layer

Administrator affinity is one bounded multiplier inside the reader's own recipe, not a term in a published scoring formula. The reader may set its weight to zero (§16.5); that zero is the strongest influence control in the system, and it is a dial rather than a switch.

Additions: relevance floor, author concentration limits, repetition limits, negative-feedback cooldowns, diversity reranking, completion preference, positivity ratio consideration. The weights among these are tunable, not universal guarantees, and are not printed in the interface because a score this specific invites exactly the inference §0.3 forbids.

## 16.4 Diversity budget

Explicitly reserve configurable percentages of recommendation slots for:

- Outside-admin-taste content.
- New authors (below a threshold of works or readers).
- Underrepresented fandoms.
- Different moods than the reader's usual preferences.

Diversity budgets prevent taste-profile collapse into a feedback loop. Configurable per user and by admin. Default budgets are non-zero.

## 16.5 Meaningful opt-out

Setting:

> Include this instance's evolving discovery preferences alongside your own interests.

The control is a **dial, not a switch**: a weight from zero to the maximum the instance allows. Zero removes administrator influence from candidate generation, ranking, reranking, recipes, dashboard widgets, prompts, challenges, notifications, and cached recommendations. Those surfaces are enumerated in §43.1, and the dial covers every one of them; there is no separate per-surface strength control (§43.4). Any weight above zero is stated to the reader in plain words ("a little", "some", "as the operator suggests") without exposing numbers that invite inference.

Individual recommendations need not carry administrator-specific labels, but explanations must not be fabricated.

Private taste profiles are not returned to users, recipes, or plugins. External engines receive baseline context; the host applies any permitted private influence afterward.

Public behavior may permit broad inference; do not promise mathematical secrecy.

## 16.6 Community similarity suggestions

Readers may suggest "If you liked this, you may like that."

Optional rationale, spoiler marking, up/down usefulness votes, reporting, withdrawal, duplicate consolidation, eligibility and block enforcement.

Separate from machine-generated similarity. Votes count unique accounts internally, do not reveal hidden pseud linkage, affect this recommendation signal only, grant no trust or moderation authority.

## 16.7 Recommendation recipe builder

A recipe is declarative configuration, not arbitrary code.

Combines baseline engines, weights, filters, diversity settings, exclusions, completion preference, bounded boosts, reranking options.

Support preview, save privately, publish, install, duplicate, remix/fork, version history, reset.

Recipes cannot override eligibility, re-enable opted-out administrator influence, access hidden taste vectors, search someone else's library, buy ranking, or grant execution permissions.

Public recipes remove private object references. Installation validates schema and resource cost. Marketplace-listed recipes integrate with paid extensions without requiring WASM execution.

- **Per-surface recipes.** One recipe may serve `/discover`, another the library "updates" feed, another the e-mail digest. A single global recipe would make the feature far less useful; surfaces are selected per recipe.
- **Recipe inheritance ("start from").** New readers do not build from a blank page: three first-party recipes — Fresh, Blind Date, Completionist — ship as ordinary saved recipes, inspectable and forkable. They cost nothing (rows in the same table) and they teach the feature.
- **Recipe diff view.** Installing a recipe shows what it would change relative to the reader's current one. Without this, recipes are a black box people install blind.
- **Ephemeral "for now" overrides.** A reader can say "only completed works this session" without editing a saved recipe — the one-tap temporary filter of §15.5, pinned above the feed.

**Scripting is not part of recipes.** Users who want trigger-based automation build extensions in the WASM sandbox (Milestone 21). This is a deliberate security choice: declarative configuration cannot exfiltrate data or execute arbitrary code.

## 16.8 Widget-composed dashboard

First-party widget registry and default layout factory.

Initial widgets: continue reading, recent library updates, import progress, saved views, drafts, followed authors, selected recommendation engine, community subscriptions, optional writing opportunity, watched authors, cheer received (for authors), positive feedback received (for authors), reading goal progress.

Add/remove, reorder, resize within accessible constraints, per-pseud layouts, mobile adaptation, reset, safe mode.

The default layout is named, not implied: **Continue reading, Library updates, Saved views, Discover**. First run offers a choice between a reader home and a writer home — different default widgets from the same registry, no second code path. Widget configuration lives in the widget: each widget carries its own small settings control for its parameters, so configuration is not reachable only through the recipe builder.

Dashboard is not the only route to essential features. Third-party widgets use extension permissions and bounded data APIs.

Multiple dashboard views per pseud allow different layouts for different purposes (reading, writing, moderating).

## 16.9 Automatic and admin-seeded writing opportunities

Generate optional trope combinations, weekend prompts, response-fic opportunities, challenges, "write next" suggestions.

**Wishlist board:** any user (including admin) posts "I'd love to read a fic where X." Others can claim, write, and link fulfillments. Admin wishes appear as one stream among many, not preferentially featured.

Wishlist voting: users upvote wishlist items to surface demand. Votes are demand signals, not obligations for authors.

**Request candidates:** for each open wishlist item, the system identifies which of the viewing user's own bookmarked or authored works might match. Only the user's own content is scanned. Match suggestions never scan other users' private libraries.

Use deterministic pools first. AI is optional. Combine writer interests with instance affinity only when enabled. Never invent human sponsors, commissions, or community demand.

## 16.10 Blind-spot and surprise-me modes

- **Blind-spot suggestions:** "You've never read Fandom B, but it has 40 works matching your preferred dynamics."
- **Surprise-me mode:** reader explicitly requests recommendations outside their profile. The system inverts usual weighting.
- **Temporary taste boosts:** admin can temporarily boost a trope, fandom, or dynamic for a month without altering the long-term profile.

## 16.11 Editorial curator picks

Trusted curators and the administrator may feature works with short rationales. Displayed in a dedicated `/discover` slot separate from algorithmic recommendations.

Curator picks:

- Are attributed to the picking pseud.
- Include a short rationale.
- Rotate periodically.
- Do not affect ranking beyond the featured slot.
- Are subject to quorum review if disputed.

## 16.12 Administration

```text
/admin/discovery
```

Controls: taste sources and their membership (§16.15), signal inclusion, learning pause, influence pause, recency, weights, the §16.16 weighting mode and its floor/ceiling bounds, the demand diversity budget, profile history, reset and rollback, aggregate evaluation, recommendation cache invalidation, diversity budget tuning, temporary boosts, editorial pick slots, the browse-sort defaults of §43.4, and the §43.6 per-surface exposure report.

## 16.13 New voices and the cold start

A first work has no readers, and a reader choosing between two unknown works picks the one with history. Without a deliberate counterweight, an archive's newest authors are its least visible, which is the opposite of what keeps them writing.

- **New voices slot:** a discovery surface reserved for recent works by authors below a configured threshold of published works or followers, sized and placed so it is a real slot rather than a token one, and presented as one stream among the discovery surfaces rather than as a banner.
- **First-work marker:** a reader-facing indication that an author is new, framed as an invitation to encourage rather than as a caveat about quality. The author may hide it.
- **Early reader recognition:** a reader who reads, rates or reacts to a work within a configured window of its publication, while it has few readers, receives a badge or credits for it. The threshold and window are configuration, the reward is never a ranking and carries no governance weight, and it cannot be farmed by publishing and reading one's own work — the anti-farming rules of §28.11 apply unchanged.
- **Short blind-date residency:** a newly published work that has not yet been shown to anyone enters the blind-date pool (§16.10) for a bounded period, so a first work is guaranteed some exposure without being guaranteed attention.
- **The slot's effect is measured and reported** to the administrator like any other discovery surface, including whether works it surfaced are continued by their authors.

Nothing here overrides a reader's stated preferences, and no reader's discovery is flooded by new works: the slot is bounded, labeled and excludable.

### 16.15 Taste sources

§16.2 defines *a* profile: the administrator's. An instance may instead — or also — steer by one
or more **taste sources**, so "what this instance is about" can be drawn from the operator, a
configured subset of users, or a role, without becoming a second mechanism.

```toml
[discovery.taste_sources]
sources = [
  { kind = "admin" },
  { kind = "cohort", members = ["pseud-a", "pseud-b", "pseud-c", "pseud-d", "pseud-e"] },
  { kind = "roles", roles = ["tl4+"], min_members = 5 },
  { kind = "long_term_users", min_members = 5, min_tenure_days = 90, min_contributions = 5 },
]
```

- **`long_term_users`** — a virtual source derived from accounts meeting all three gates: account age ≥ `min_tenure_days` (default 90), contribution count ≥ `min_contributions` (default 5, using §16.16.1 contribution events — imports, fulfilled bounties, accepted curation, not logins), and the source as a whole must satisfy the same ≥ 5 cohort floor. This prevents an operator's handful of alt accounts from steering discovery. The composite signal is the aggregate reading-writing taste vector of qualifying accounts, recomputed on the same cadence as the admin profile.
- **Cohort floor.** A cohort or role source needs at least `min_members` (default 5) distinct
  accounts, validated at startup (§38.1). Below that the source is refused with a named reason: a
  small cohort *is* a person, and the operator's taste would be inferable from its public reading,
  which §0.3 forbids.
- **Opt-in and withdrawal.** A cohort member consents before contributing and may withdraw, at
  which point the source recomputes from what remains. A member never learns *how much* their
  behaviour moved a result — the §39.4 silence contract.
- **The §16.2 exclusions are absolute.** Moderation sessions, troubleshooting, import tests and
  accidental opens are never signals, from any source.
- **One dial.** §16.5's weight governs the *combined* influence. There is no per-source dial: a
  per-source control would reveal the source structure the cohort floor exists to hide.
- **Versioning and rollback.** Each source keeps §16.2's profile version history, so an operator
  can see what changed, when, and revert it.
- **Reason fields.** §16.1's `reason` may name an instance term only for a topic declared
  `public = true` (§0.4). Otherwise it degrades to one undifferentiated line, which is the single
  shape §16.3 permits for instance influence.

**Acceptance.** A cohort of four accounts is refused at startup and a cohort of five is accepted.
No member can distinguish "my contribution mattered" from "it did not". Withdrawing a member
recomputes the profile and changes no other reader's dial value. A private-topic instance never
emits a theme term in a reason field.

### 16.16 Demand weight — a reader's pull on unwritten content

A reader's **demand** signals — a wishlist vote (§18.5), a bounty's visibility and queue position
(§20.5), a prompt vote (§18.5), which "write next" opportunity surfaces first (§16.9) — carry a
weight. That weight decides what this instance asks for next. It never affects a published work's
search or discovery rank (§15.10), never affects trust or governance (§19.1, §34.3), and is never
purchased (§0.3).

```text
demand_weight = trust_multiplier(trust_level)           // safety ramp, §19.1 ladder
              × taste_multiplier(theme_affinity)        // silent; floor … ceiling
              × contribution_multiplier(domain_record)  // earned; windowed, decaying
```

**Every bound is configuration, not a constant:**

```toml
[weighting]
mode = "trust_taste_contribution"  # flat | trust | trust_taste | trust_taste_contribution
taste_floor = 0.75                 # the taste multiplier's floor — operator-configurable
taste_ceiling = 1.25               # its ceiling — operator-configurable
contribution_floor = 1.0           # the contribution multiplier's floor
contribution_ceiling = 2.0         # its ceiling
contribution_window_days = 180     # how far back contribution is counted
demand_diversity_percent = 20      # must be > 0 (§16.4's precedent)
```

`mode = "flat"` is the off switch and is always available: every demand signal weighs 1, exactly
as `[directory].vote_weighting = "flat"` already does for §39.4. Widening the range is the
operator's call; what no configuration can do is reverse a plain majority (§16.16.3).

**There is no XP, level, points or reputation value here, and this section deliberately
introduces no new unit.** The weight is computed when it is used, from signals the instance
already keeps, and is never accumulated into a number a user owns, earns, or can point at. Where
recognition is meant to be *visible*, it is episodic — §9.7.5's weekly and monthly leaderboards
and §9.7.6's badges — and the reader-facing reward for the same behaviours is a badge, a
placement, or credits.

#### 16.16.1 What the contribution term may count

This list is the specification. A metric outside it is a defect to fix, not a weight to tune.

| May count | Never counts |
|---|---|
| Finishing works with strong §9.7.4 quality signals, **in the domain the demand item belongs to** | Chapters read, works published, words written, posts made, hours online, days logged in |
| Bringing the instance works it did not have: a new-to-instance import, a first translation, a first narration edition (§11, §22, §24) | Re-importing a work the instance already had |
| Fulfilling demand: a wishlist item or a bounty, requester-confirmed (§9.7.2) | Fulfilling one's own request, or a request from an account with the same owner |
| Curatorial labour that survived review: accepted canonicalisation (§15.11), approved quorum decisions (§19.4), §33.3's tag-wrangling queue | Votes or reports filed without review; sanctions issued; proposals rejected |
| Positive feedback *delivered*: a comment that passed §12 and reached its author; a constructive review where the author opted in | Reactions alone, ratings alone, private bookmarks |
| Recency-weighted quality: §41.1's half-life on the works involved | Account age, credit balance, subscription tier, bounty size, follower count |

Two clarifications, because both are easy to get wrong:

- **The term is domain-scoped and windowed.** Contribution to fandom A does not lift a reader's
  weight on a demand item in fandom B, and it decays on a documented half-life over
  `contribution_window_days`, so a weight reflects recent work rather than a historical high-water
  mark. Otherwise an instance's early contributors would hold the demand queue permanently.
- **Quality is uncontested quality.** A work §33.2 marks `contested` contributes nothing until a
  quorum clears it, and the §9.7.8 anti-gaming rules (time-on-page, unique accounts, pseud
  isolation, self-action exclusion) apply unchanged.

#### 16.16.2 What is visible

| Instance declares | Mechanism documented? | A reader may see their own weight? | Reason fields may cite theme terms? |
|---|---|---|---|
| ≥1 topic `public = true` | yes — plain words, no numbers | yes, in coarse buckets ("a little", "some", "as the operator suggests") | yes, for public topics only |
| only `public = false` topics | mechanism documented, **components never** | **no, never** | no — one undifferentiated line |
| no topics declared (the default) | mechanism documented, **components never** | **no, never** | no — one undifferentiated line |

The mechanism is always documented; the *components* are what silence protects. §0.4.3's default —
"Kink-focused instances are the default; declaring anything is the deliberate act" — therefore puts
the default in the silent column, which is the safe direction. No weight appears in any API
response, export, error message or log line, and a test asserts the absence rather than trusting
the review (§41.3's pattern).

#### 16.16.3 The guards

Three properties hold under every configuration, and the first two close gaps that exist in §39.4
today:

1. **The floor is relative to the ceiling.** `taste_floor × taste_ceiling ≤ 1.0` must hold, so a
   single aligned vote can never outweigh two unaligned votes of equal trust. Startup refuses a
   configuration that breaks it, and the effective floor and ceiling are published on
   `/api/v1/meta` beside the instance's other operator-policy disclosures (§0.4.3, §20.10.7).
2. **Majority integrity.** A weighted bloc may **reorder** demand; it may never **reverse** a
   plain majority. Concretely: while `mode ≠ flat`, a demand outcome follows the weighted score
   only while the unweighted (one-account-one-vote) outcome agrees; if the two disagree on the
   outcome — which item surfaces first, which bounty is promoted, which proposal passes — the
   decision routes to §19.4 quorum review instead of resolving on weights. This is the property
   that keeps a taste-weighted queue from becoming a taste-filtered one.
3. **A demand diversity budget.** `demand_diversity_percent` (default 20, must be `> 0`) of the
   demand the instance surfaces must carry **no** boost from any term — the §16.4 diversity budget
   applied to supply rather than to display. §16.4 already establishes the shape: configurable,
   default non-zero, and the mechanism that keeps an instance from becoming monothematic.

#### 16.16.4 What demand weight never does

1. **It never ranks published works.** A work's search or discovery position is decided by §15.10,
   never by its author's or its voters' weight; otherwise an aligned reader becomes a marketing
   channel and §0.3's ban on purchased ranking acquires an alignment-shaped loophole.
2. **It never converts money into influence.** Bounty size buys fulfillment priority — that is what
   escrow is for — and contributes **zero** to the weight. §33.2's rule for rating weights ("from
   trust alone, never from credits, subscriptions, bounties or marketplace revenue") applies here
   unchanged, and now explicitly includes any future gamification unit.
3. **It never touches trust, moderation, quorum or sanctions** (§19.1, §19.2, §34.3). It is a
   demand signal; it is not a safety signal.
4. **It never obligates an author.** §18.5 stands: votes surface demand and do not obligate
   anyone; §16.9's rule against inventing human sponsors, commissions or community demand stands
   too.

#### 16.16.5 Tables

| Table | Important fields |
|---|---|
| `taste_sources` | id, kind (admin/cohort/roles), config_json, min_members, created_at, disabled_at |
| `taste_source_members` | source_id, account_id, consent_at, withdrawn_at |
| `demand_weights` | account_id, domain_key (fandom/tag scope), weight_bp, trust_component_bp, taste_component_bp, contribution_component_bp, computed_at, expires_at |
| `demand_weight_history` | id, account_id, domain_key, weight_bp, reason, computed_at |
| `demand_signal_events` | id, account_id, domain_key, signal_kind, reference_id, weight_bp, created_at, expires_at |
| `demand_diversity_state` | period, surfaced_with_boost, surfaced_without_boost, budget_percent |

`demand_weights` is ciphertext-of-intent — a computed number kept for audit and recomputation —
and no endpoint serialises it (§16.16.2).

**Acceptance.**

- With `mode = "flat"`, every demand signal weighs 1 and the ordering is identical to the
  unweighted one.
- A TL4 vote and a TL0 vote on the same wishlist item move its demand score by different amounts
  under `trust_taste_contribution`, and by the same amount under `flat`.
- Two readers with identical contribution records but disjoint domains have different weights on a
  demand item in fandom A, and identical weights on an item in neither.
- A configuration with `taste_floor × taste_ceiling > 1.0` is refused at startup with a named
  reason; the effective bounds are readable on `/api/v1/meta`.
- Twenty aligned high-weight accounts and two hundred unaligned voters reach opposite outcomes only
  by routing to §19.4 quorum review; the weighted result never silently overturns the unweighted
  one.
- A funded bounty raises its own fulfillment priority and changes the funder's weight by zero.
- At least `demand_diversity_percent` of surfaced demand carries no boost; setting the value to
  zero is refused.
- No endpoint, export, error message or log line contains a weight, a component, or a reason
  naming a private topic; the absence is asserted by test.
- Removing a cohort member recomputes the affected profiles, and no remaining reader's dial value
  changes.

#### 16.16.6 What this section deliberately does not do

- **No new unit.** No XP, levels, points, reputation or "standing" — the weight is a computation, not an asset (§9.7.1's episodic rule, §28.10). §35.2's forum karma is neither extended nor reused for this.
- **No reader-facing progress toward influence.** A progress bar toward power over what others
  write is a streak-shaped dark pattern (§0.2). Recognition is where progress lives: a badge, a
  leaderboard period, a credit balance.
- **No obligation, no commission, no promise.** A high weight makes a request louder, never more
  likely to be fulfilled than an author decides.
- **No second dial.** §16.5's single weight governs instance influence everywhere; this section
  adds none.

## 16.14 Freshness

Recency is already an input to ranking; freshness is the reader-facing statement of it.

- **"Updated this week" and "Newly completed" filters** in search and saved views, derived from the publication event log (§4.3) rather than from a timestamp on the work row.
- **Active author indicator** on a work card, derived from the same activity status as §8.8 and never from a private signal.
- **Newly completed is a discovery slot of its own:** a reader looking for something to finish in one sitting is not served a filtered view of trending.
- **Decay in trending:** a work's contribution to a trending calculation decays with age since its last publication event, so no work holds a trending position indefinitely on the strength of one event. The curve is configuration, is stated on the administrator's discovery page, and **is never applied to a reader's own library, bookmarks or follows.**
- Freshness never buries the archive: a reader who narrows by fandom and mood still sees the best matches regardless of age, and a decayed trend score never removes a work from a filtered result.

## Acceptance

- Opt-out changes candidates and scores across all surfaces.
- Exact and chronological sorts remain exact.
- Missing profiles fall back to baseline.
- Blocked or ineligible content never enters displayed results.
- Minors receive policy-appropriate discovery.
- Recipes cannot bypass host policies.
- Recipes cannot execute arbitrary code.
- Dashboard reset works even with a broken widget.
- Taste alignment never affects governance or trust.
- Taste alignment never adds to a recognition a user can see; the leaderboards and badges of §9.7.5–§9.7.6 are earned from quality signals alone (§16.16.1).
- A reason names an instance term only when the topic is public (§16.15, §16.16.2).
- A demand outcome is never reversed by weights against a plain unweighted majority (§16.16.3).
- Diversity budgets are honored under load.
- Request candidates only scan the requesting user's own content.

---

# 17. Milestone 12: Comments, Forums, Groups, Messaging, and Presence


---

## 17. Milestone 12: Comments, Forums, Groups, Messaging, and Presence

> **2026-09-20 revision.** Work-page discussion has been re-specified in
> §35.0: the default mode for new works is `ThreadOnly` — typed-vote
> reaction bar on the work page, all text discussion in the linked forum
> thread. `CommentsOnly` (this section's comment surface) remains a
> supported per-work mode indefinitely, and `Both` exists behind an
> admin warning. §17.1 below is therefore the `CommentsOnly`/`Both`
> behavior, not the default. Forum votes are typed votes under §35.2.

## 17.1 Comments and reviews

Work/chapter comments, replies, appreciation, spoiler formatting, appropriate edit history, author locking, reporting, block/mute enforcement, review moderation under the positivity framework.

All work/chapter comments and reviews pass through the positivity filter (Milestone 12).

Limit nesting depth and flatten deeper replies clearly.

## 17.2 Forums

Categories, boards, topics, posts, polls, reactions, pins, locks, subscriptions, pagination, topic tags, forum full-text search, mentions, read state and unread badges, post drafts, scheduled posts.

Forum posts have a lighter positivity policy: constructive disagreement is allowed; hostility and personal attacks are filtered.

## 17.3 Read state

Per-pseud topic high-water mark using stable post ordering. Opening a topic marks only content actually presented under the documented read policy. Paginated entry does not mark unseen later pages read. Writes bounded, monotonic, idempotent. "Mark topic/category read" is explicit. Deleted or hidden posts do not break cursors. Counts exclude inaccessible categories.

## 17.4 Forum search and tags

Search includes topic titles, post bodies, category scope, author, topic tags, dates, safe highlighted snippets.

Permissions apply before counts and snippets. Topic tags are distinct from fiction taxonomy but may share canonicalization infrastructure. Support scoped tag aliases, merge history, tag pages.

## 17.5 Mentions and notification preferences

`@handle` mentions with disambiguation. Mention creation checks visibility, blocks, unsolicited-contact restrictions, rate limits, minor-protective policy.

Per-category/topic watch levels: `muted | normal | tracking | watching`.

Notification settings: in-app, email, push, immediate versus digest, quiet periods, followed topics, mentions and replies.

Email digests optional and unsubscribeable without login where safely possible.

## 17.6 Groups

Roles: `owner | manager | member`. Visibility: `public | approval_required | private`.

## 17.7 Messaging

Conversation invitations, direct messages, group conversations, chat rooms, leave/mute/block/report, unsolicited-message restrictions, minor-protective defaults.

Persisted messages authoritative; live delivery is optimization. Do not claim end-to-end encryption.

## 17.8 Real-time delivery and catch-up

Common event envelope with durable cursors. WebSocket delivery, SSE read-only delivery where suitable, polling fallback, catch-up through opaque `after` cursor, duplicate suppression, reauthorization on reconnect and catch-up, gap recovery when cursor expires.

Single-process operation uses in-process fan-out plus durable storage. Optional Redis pub/sub distributes across processes; it is not the event history.

## 17.9 Presence and typing indicators

Presence and typing indicators are strictly opt-in per pseud.

Rules:

- Default off for all pseuds.
- Cannot leak across pseuds on the same account.
- Not visible to blocked or muted users.
- Not shown in public listings; only within conversations the user is a member of.
- Presence granularity: "active now" only, no last-seen timestamps.
- Typing indicators appear only when the user is actively typing in a specific conversation, cleared after a short timeout.

Presence infrastructure uses ephemeral in-memory state; no long-term presence history is stored.

## 17.10 Scoped sanctions

Category- or group-scoped posting timeout, reply restriction, topic-creation restriction, access ban where policy permits.

Every sanction records scope, reason, start, expiry, issuer, review reference, appeal path.

Forum reputation points and posting-volume leaderboards are not implemented.

## 17.11 Reading clubs

Groups can form reading clubs: pick a work, read on a schedule, discuss chapter by chapter in a linked forum thread. Reading club discussions bypass the individual author feedback filter but remain subject to forum policy.

## 17.12 Fandom-specific spaces

A large fandom has its own norms, in-jokes and moderation needs, and one general forum serves none of them well.

A fandom space is a group (§17.6) bound to a fandom, with:

- Its own sub-forums, topic tags and read state, built on the existing community machinery rather than a second forum implementation.
- **Its own moderators, appointed through the same quorum process as any other moderation appointment** (§19.4), with the same accountability, appeal path (§19.10) and modlog (§19.12). A fandom appointment is scoped to that space and confers nothing instance-wide, and it is gated on trust level rather than on any role.
- Its own content-note conventions, which may **add** notes to the instance vocabulary for works carrying that fandom (§15.16) and may never remove the instance's own.
- Its own events, challenges and reading clubs (§18.2, §17.11), linked from the fandom landing page (§15.13) alongside the works.
- Its own welcome text, written by its moderators, subject to the instance's positivity rules.

A fandom space is public by default and its moderators may set it to approval-required or private. A fandom with no space has no space: none is created automatically, and the landing page states whether one exists.

## Acceptance

- Removed members lose access.
- Private topics cannot be fetched by ID.
- Read badges accurate under pagination.
- Mentions do not bypass blocks.
- Digests omit newly inaccessible content.
- Reconnection does not duplicate messages.
- Redis failure does not lose persisted posts.
- Expired sanctions cease to apply.
- Reporting exposes only appropriately scoped evidence.
- Reading club discussions do not leak private content.
- Presence and typing indicators respect opt-in setting and pseud isolation.

---

# 18. Milestone 13: Collections, Challenges, Requests, Wishlists, and Writing Events

## 18.1 Collections

```text
submitted → approved/rejected
approved → withdrawn/removed
```

Curator roles, invitations, submission approval, sections and ordering, work-owner withdrawal, preservation-batch provenance links.

Collections do not grant permission to republish or expose private imports.

## 18.2 Challenges

Signups, prompt pools, assignments, claims, deadlines, reveal dates, anonymous-until-reveal submissions, fulfillment, withdrawal.

Represent variants through a shared configurable workflow. Named variants
include exchanges with assignments, pinch hits (reassignments when a
participant defaults), treats (bonus fulfillments for unassigned prompts),
fests with open prompt pools, and Big Bang variants with draft checkpoints and
artist pairing. A variant is a configuration of the same signup, assignment,
claim, deadline and reveal machinery, not a hard-coded event type.

**Finished-work reading challenges:** "Read 5 completed fics under 10k words this month." Encourages completed-work reading.

Reading challenge completion is private by default and grants a recurring badge plus its credit bonus (9.7.6). It produces no public ranking and no trust reward.

## 18.3 Mentorship and beta-reading

Applications, availability, interest matching, invitations, session/task completion, reporting, private participation settings.

**Reciprocal beta-reading matching:** authors opt in with fandoms, strengths, and needs. System matches pairs for mutual beta reading.

## 18.4 Sprints

Start/end time, optional shared room, private/public counters, manual word-count updates, no publication requirement, no compulsory streaks.

## 18.5 Requests, bounties, and wishlists

Implement request handling now. Attach credit escrow after the ledger milestone.

Distinguish:

- Search help.
- Recommendation request.
- Writing prompt.
- Wishlist item (no obligation).
- Commission or bounty (with escrow).
- Preservation request.

Wishlists are public boards where anyone can post "I'd love a fic where X." Others can claim. Fulfillments link back to the wishlist item.

Admin wishlist items are marked as such but do not receive preferential featuring.

Wishlist voting surfaces demand, weighted by §16.16's demand weight. Vote counts are visible to all users and to potential fulfillers as an aggregate; a per-user weight is never shown to anyone (§16.16.2). Votes do not obligate authors.

## 18.6 Editorial curator picks

Trusted curators (and the administrator) may feature works with short rationales in the dedicated discovery slot (Milestone 16).

## 18.7 "Best of" community-voted lists

Periodic community votes for completed works by category (fandom, mood, length, etc.). Separate from algorithmic trending. Voting requires trust level and completes with quorum review.

## 18.8 Collaborative drafting tools

Contributors are already modelled (§4.3, §8.7). These are the tools that make working on one work together possible without leaving the platform.

- **Shared outline:** a per-work planning document, editable by contributors with write access and private to them.
- **Beta-reader inline comments:** anchored to a passage in a chapter revision, visible only to contributors and invited beta readers, and entirely separate from public comments and from the positivity filter (§12.1). This is a private author-to-beta channel where blunt criticism is the point, and it is never delivered to the author as reader feedback. Resolvable, and never published with the work.
- **Revision comparison:** a side-by-side or unified diff between any two revisions visible to the contributor, so a co-author can see what changed and who changed it. The revision history is already append-only (§8.5); this adds the reader for it.
- **Round-robin assignment:** for a multi-author work, an optional ordering of who drafts which chapter, shown on the contributor view and not published.
- **Contribution notes:** each revision may carry a note naming its contributor's contribution, used to compile public attribution on request rather than asserted automatically.

Beta-reader inline comments are the one place in this specification where text is deliberately not positivity-classified, and it is stated here so the exemption is a decision on the record rather than an oversight: the classification exists to protect authors from readers, and a beta reader is not a reader.

## 18.9 Reading paths

A reading path is an ordered, curated walk through several works, and it is a first-class content type: ownable, editable, bookmarkable, shareable, commentable and rateable.

- Each stop names a work, an optional curator's note on why it is there, and whether it is required or optional.
- A path states whether order matters; a path where it does not is displayed as a list rather than a sequence.
- Progress through a path is tracked per reader and private by default, using the same completion machinery as a work (§9.8).
- Paths may be public, unlisted or restricted, are searched with the standard filters, and appear in collections, series listings and the fandom landing page (§15.13).
- Community paths may be created by any reader; featuring one is a curator decision (§18.6) and never a popularity vote, and a path's rating rates the curation rather than the works inside it.
- A path lists every stop's availability, so a reader who may not read one work still sees the shape of the whole and decides for themselves whether to skip it.
- A path never reorders or re-ranks the works it contains, and a work's position in a path changes nothing about the work.

## 18.10 Gift works and dedications

A work may be gifted to a reader or dedicated to one or more pseuds.

- The author names the recipient pseud at or after publication. The gift shows
  on the work page, in the recipient's gifts listing, and nowhere else.
- The recipient is notified once. A recipient who does not want gifts can
  decline them account-wide, which stops new gifts and hides existing ones
  from their listing without touching the work.
- A gift confers nothing between the pseuds — no follow, no contact
  permission, no exception to blocks. Gifting a user who blocks the author
  fails with a validation error that does not reveal the block.
- Gifts and dedications are public attribution, the same surface class as the
  contributors line: the recipient's display name appears, never anything
  about their account.
- A challenge fulfillment with a named recipient (§18.2) and a gift are the
  same link seen from two sides, not two records.

| Table | Important fields |
|---|---|
| `work_gifts` | work_id, recipient_pseud_id, gift_note, challenge_fulfillment_id, created_at, declined_at |

## Acceptance

- Challenge identities remain hidden until reveal.
- Collection ownership does not bypass permissions.
- Missed deadlines have defined outcomes.
- Withdrawals preserve required audit and attribution history.
- Generated events do not fabricate human sponsorship.
- Wishlist admin markers do not confer ranking advantage.
- Curator picks are attributed.
- Reading challenges grant no gamification rewards.

---

# 19. Milestone 14: Trust, Reports, Quorum, Appeals, Sanctions, and Process Feedback

Priorities 5 and 6 are centered here.

## 19.1 Trust model

Separate:

1. Account reliability TL0–TL6.
2. Scoped expertise.
3. Appointed staff roles.

| Level | Meaning |
|---|---|
| TL0 | New account |
| TL1 | Established participant |
| TL2 | Regular participant |
| TL3 | Reviewed trusted contributor |
| TL4 | Trained steward |
| TL5 | Senior independent reviewer |
| TL6 | Explicitly appointed trustee |

Higher levels require reviewed conduct, not merely point totals. Core publishing and reading remain available at TL0.

**Trust is earned through reviewed conduct.** Advancement requires:

- Time-in-good-standing thresholds.
- Reviewer nomination and quorum approval.
- No active sanctions.
- No pattern of held destructive comments.

Trust is not calculated from XP, post count, kudos received, credits earned, or any activity-volume metric.

**Single-admin mode is the default mode.** An instance with one operator skips quorum for every action: the quorum fields remain in the schema and the audit trail stays complete, but approvals default to the operator. Trust levels TL4–TL6 are opt-in — the levels stay defined and the code exists, but no surface requires them on a small instance. With fewer than two eligible reviewers, the operator decides appeals alone; the independence rule (§19.10) applies to instances large enough to have independence. The modlog (§19.12) is data first: the audit trail exists in the database, and a reader-facing modlog page is optional and off by default.

## 19.2 Effects

Trust may increase rate limits, batch sizes, proposal eligibility, curator eligibility, extension resource ceilings, gift/bounty limits, moderation queue eligibility, wishlist claim priority, invite-code issuance quota.

Trust is also the *safety ramp* of §16.16's demand weight: a week-old account's demand counts for
less, which is the same question §9.7.8 answers for author credits. That is trust used as an input,
never trust computed from an input — no trust level is derived from a demand weight, and no demand
weight grants trust.

It does not automatically grant private-message access or administrator powers.

Credits, ratings, views, posting volume, reading streaks, and marketplace purchases do not buy trust.

## 19.3 Reports

```text
submitted → triaged → investigating → proposal → decision → appealed → closed/reversed
```

Trust may prioritize review; it does not establish guilt. Use account identity privately to prevent multiple pseuds from counting as independent reporters or voters.

Report categories include: spam, harassment, copyright, inappropriate content, positivity violation, safety concern, other.

Reporters can check the status of their own reports (pending, needs_admin, resolved, dismissed).

## 19.4 Quorum defaults

- Routine tag change: two independent approvals.
- High-impact tag or public identity merge: three.
- Routine sanction: proposer plus independent reviewer.
- Permanent ban: three eligible reviewers where staffing permits.
- Appeal: reviewers independent of the original decision.
- Emergency containment: one authorized moderator, followed by review.
- Positivity filter classification override: single moderator; repeated overrides for one account trigger secondary review.
- Metadata correction: two reviewers for shared records.
- Extension approval: three reviewers with permission-review expertise.
- DMCA takedown: single authorized reviewer; counter-notice triggers quorum review.
- Shadowban: two-reviewer minimum with defined expiry.

Preservation releases and significant metadata corrections use explicitly assigned review policies.

## 19.5 Emergency actions

Temporarily hide content, freeze replies, restrict messaging, suspend posting, disable an extension, pause an unsafe importer, escalate the positivity threshold for a work under attack.

Require reason, review deadline, escalation, audit entry.

## 19.6 Sanctions

Sanctions include:

- Warning.
- Rate limit reduction.
- Scoped posting timeout.
- Category or group ban.
- Site-wide temporary suspension.
- Permanent ban.
- Shadowban (see 19.7).

Every sanction records scope, reason, start time, expiry, issuer, review reference, appeal path.

Sanctions do not affect account deletion rights or data export rights.

## 19.7 Shadowban policy

Shadowbans hide a user's content from others without informing the user, used exclusively against confirmed bots, spam operations, and repeat harassment accounts that have evaded prior sanctions.

Constraints:

- Requires two-reviewer quorum minimum.
- Time-limited (default 30 days, renewable through quorum).
- Logged in the audit trail with reason.
- Reviewed automatically at expiry.
- Shadowbanned users retain data export and deletion rights.
- Aggregate shadowban counts appear in the public modlog (not individual identities).
- Never used against users for good-faith rule violations; those get transparent sanctions.

Shadowbans are a last resort against actors who use transparency to game the system. Their use is deliberately constrained.

## 19.8 Bootstrap mode

If only one administrator is available, label single-person decisions honestly. Do not call them quorum. The site can operate in bootstrap mode indefinitely; the administrator makes governance decisions until trusted community moderators exist.

Bootstrap-to-community transition is a deliberate action: appoint initial trusted users, transfer moderation queue access, document policy handoff.

## 19.9 Admin-quorum relationship

The administrator can:

- Set policy that governs quorum decisions.
- Appoint and remove trusted users.
- Override quorum decisions in emergencies (with audit and stated reason).
- Reserve certain decisions to administrator only (marked explicitly).

The administrator should not:

- Routinely override quorum decisions on individual moderation cases.
- Punish moderators for good-faith decisions the administrator disagrees with.
- Curate every moderation queue personally (defeats the purpose).

## 19.10 Ban appeals

Banned users may submit an appeal with a written reason. Appeals go to reviewers independent of the original decision.

Appeal outcomes:

- Uphold ban.
- Reduce sanction.
- Reverse ban.
- Additional information requested.

Appeal decisions are logged in the audit trail. Repeated frivolous appeals may be rate-limited but never blocked entirely.

## 19.11 DMCA notices

Handle takedown requests through a documented workflow:

```text
notice received → validated → targeted content restricted
→ owner notified → counter-notice window → decision → resolved
```

Requirements:

- Notice claimant identity recorded.
- Targeted content is restricted, not deleted (recoverable if reversed).
- Content owner notified with counter-notice option.
- Counter-notices route to quorum review.
- Decisions logged in the modlog with redacted claimant identity.
- Repeat infringers subject to escalating sanctions per policy.
- Fraudulent notices trigger review of the claimant.

Where the instance holds a reference to a work rather than its bytes, a notice is answered by identifying the host and applying this workflow to the listing itself (§30.10), so a reference-only instance is not treated as the infringing party for media it never stored.

DMCA workflow respects legal requirements while preventing abuse of takedown mechanisms for harassment.

## 19.12 Public modlog

Publish redacted decision summaries, not private evidence.

Do not expose hidden pseud linkage, private messages, child-related evidence, reporter identity, source credentials, sensitive search or reading history, positivity classifier scores, individual shadowban targets, DMCA claimant identities.

On a single-admin instance the modlog is the operator's own record, published or not as the operator chooses; nothing in this section obliges a small instance to staff a public page it cannot fill.

## 19.13 Community feedback on moderation process

Process feedback, not popularity-based verdicts.

Eligible participants assess redacted decision summaries for clarity, consistency with published policy, procedural fairness, adequacy of explanation.

Optional participation, aggregation thresholds, anti-brigading controls, no disclosure of private case evidence, no automatic reversal or punishment, no public moderator popularity leaderboard, independent review of persistent process concerns.

Feedback is not a substitute for appeals and is labeled as participant opinion.

## Acceptance

- No self-approval.
- Two pseuds from one account count once.
- Appeals exclude original decision-makers.
- Temporary actions trigger deadlines.
- Public summaries are redacted.
- Process feedback cannot directly impose sanctions.
- Financial activity has no effect on eligibility.
- Bootstrap-to-community transition is auditable.
- Administrator overrides are logged with reason.
- Shadowbans expire automatically without renewal.
- DMCA counter-notices restore content on quorum reversal.

---

# 20. Milestone 15: Credits, Fair Queues, Bounties, and Billing

Priority 8 is centered here.

## 20.1 Ledger

Balanced entries, not balance mutation without history.

```text
transaction_id
type
idempotency_key
reference
entries[]
created_at
```

Separate earned credits, subscription grants, purchased credits (if enabled), held credits. Balances transactional or derived from verified cached totals.

## 20.2 Job charging

```text
quote → reserve → submit → complete → capture actual charge
```

Failure: release hold or apply documented partial charge.

Quotes cover batch imports, device delivery, conversion, translation, AI features.

## 20.3 Initial credit economy

| Action | Credits | Cap |
|--------|---------|-----|
| Daily login | 5 | 1/day |
| Read a chapter | 1 | 10/day |
| Quick reaction | 1 | 5/day |
| Positive comment (delivered) | 2 | 3/day |
| Constructive review (delivered) | 3 | 1/day |
| Forum post | 1 | 5/day |
| Forum topic | 2 | 2/day |
| Bookmark | 1 | 5/day |
| Import work | 2 | 5/day |
| Import from new source | 10 | 1/day, once per source |
| Mark work finished | 5 | 200/month |
| Finish short work (<10k) | 3 | 200/month |
| Finish long work (>100k) | 10 | 200/month |
| Finish in new fandom | 3 | 200/month |
| Fulfill wishlist item | 15 | 1/day |
| Translation chapter (approved) | 5 | 5/day |
| Quorum vote | 2 | 5/day |
| Author: unique reader per chapter | 1 | 50 readers/day/work |
| Author: reader completion | 5 | Per reader |
| Author: reaction received | 1 | Per reader |
| Author: positive comment received | 2 | Per reader |
| Author: bookmark received | 1 | Per reader |
| Author quality multiplier | Up to 1.8x | Weekly recalc |
| Author demand multiplier (silent) | Up to 1.5x | Weekly recalc |
| Leaderboard 1st place | 50 | Per category per period |
| Leaderboard 2nd–3rd | 25 | Per category per period |
| Leaderboard 4th–10th | 10 | Per category per period |
| Leaderboard participation | 2 | Per category per period |
| Milestone badge | 10–50 | Once per badge |
| Recurring badge | 2–20 | Per award |
| First publication | 20 | Once, abuse-reviewed |
| Accepted challenge completion | 10 | Monthly cap |
| Reviewed mentorship/beta | 10 | Monthly cap |
| Reviewed governance batch | 5 | Per batch |

**Daily action cap:** 50 (free), 75 (Author), 100 (Curator/Patron).

**Author caps:** 100/work/day, 300/author/day, 5,000/author/month.

**Priority job costs remain unchanged from previous spec.**

Reading and import activity earn credits under 9.7, but the metric is never raw volume: completion credits require time-on-page proportional to word count, action credits carry per-action daily caps, and streak length earns no multiplier. Do not reward posting volume, sanctions issued, positive star ratings, or streak length.

| Priority job | Credits |
|---|---:|
| Import priority | 2 base |
| Additional 20-chapter batch | 1 |
| EPUB/HTML/text/Markdown priority | 1 |
| PDF/AZW3/MOBI priority | 3 |
| Send-to-Kindle delivery | 2 |
| Author bibliography batch import | 5 base + 1 per 10 works |
| Author watch (per watched author per month) | 2 |
| Background report | 5 |
| AI translation | Explicit estimate per word count |
| AI summarization | Explicit estimate |
| AI comment classification (per author, monthly quota) | Included with subscription tiers |
| Natural-language search assist | 1 per query |
| Extension execution beyond free ceiling | Variable per plugin |

Standard jobs remain free within fair-use limits.

**Subscription gamification benefits:**

| Tier | Gamification benefit |
|------|---------------------|
| Reader | Daily action cap 50 (same as free). |
| Author | Daily action cap 75. Free streak freeze (1/month). |
| Curator | Daily action cap 100. Free streak freeze (2/month). |
| Patron | Daily action cap 100. Free streak freeze (unlimited). |

## 20.4 Scheduling

Weighted queues with aging. Reserve capacity for standard jobs. Paid demand must not starve free users.

Separate resource classes: a large converter cannot block all small imports; a translation job cannot block all imports.

## 20.5 Bounties

```text
funded → claimed → submitted → accepted → paid
```

Alternatives: `expired | disputed | refunded | canceled`.

Define deadlines, evidence, disputes, and acceptance authority before enabling transfers.

Wishlist items may optionally be funded with a bounty. Non-funded wishlist items remain valid.

A bounty's size buys fulfillment priority for its own request and nothing else: it contributes zero
to its funder's §16.16 demand weight, so money can never become influence over what the instance
asks for next. Before bounties are enabled, the operator states deadlines, evidence, disputes and
acceptance authority (§20.5), and the §16.16 weighting mode is visible on `/api/v1/meta`.

## 20.6 Subscriptions

Optional configurable reference plans:

- **Reader:** €3/month. Higher credit regeneration, larger download quotas, AI translation quota, ad-free supporter status.
- **Author:** €6/month. All Reader benefits plus larger publishing quotas, priority queue slots, AI-assisted comment classification for own works, cross-posting quota.
- **Curator:** €12/month. All Author benefits plus larger extension resource ceilings, priority marketplace review, larger batch import quotas.
- **Patron:** €25/month. All Curator benefits plus custom recognition, direct support channel.

Subscriptions are the standing way to obtain resource-intensive features: AI
translation quota, AI comment classification, send-to-Kindle and export
quotas, batch-import and bibliography quotas, priority job queues, extension
resource ceilings, and author-watch scheduling. One-off heavy needs are bought
with credits (§20.2–20.3); recurring needs belong on a plan. Free tiers keep
every feature reachable in reduced form (§0.3): a subscriber gets more of a
thing, never exclusive access to a thing free users are locked out of
entirely.

No unlimited compute, no purchased trust, no search-ranking advantage, no moderation authority, no positivity filter bypass.

### 20.6.1 Subscription revenue attribution

When the operator enables it, subscription revenue is attributed to authors
by reading time: each subscriber's fee distributes across the authors they
actually read that calendar month, weighted by time spent on their works —
never by word count, which rewards padding. Attribution reuses the
time-on-page infrastructure (§9.7.8, §9.8) and inherits its anti-gaming
thresholds: minimum time-on-page, deduplication within a window, and
exclusion of the subscriber's own works (§9.7.8's self-dealing rules).

Attribution is a Pool A flow (§20.10.1) and is subject to the earnings cap
(§20.10.3) like any other direct attribution.

## 20.7 Marketplace revenue

Paid extensions and themes generate revenue split with developers. Configurable reference split of 85/15 (developer/platform). Operator handles tax and invoicing setup.

Marketplace ratings and reviews do not affect account trust. Popular extensions do not receive preferential trust or moderation authority.

## 20.8 Webhook handling

Signature verification, event-ID storage, idempotency, out-of-order handling, reconciliation.

## 20.9 Work monetization

The platform's own revenue is credits, subscriptions and marketplace fees
(§0.2, §20.6, §20.7). This section gives *authors* a way to be paid for their
work, because a positivity-first archive that funds its infrastructure but not
its writers is asking writers to subsidize everyone else's hobby. It is
optional at every level: an author who never touches it sees nothing, a
billing-disabled instance has nothing to see, and the free core (§0.3) is not
diminished — a reader can always read, comment, appreciate, download within
quotas, and participate without paying anyone.

### 20.9.1 What may be monetized

```text
Work monetization eligibility: original | any-with-assertion | disabled
```

- **`original`** (default where billing is enabled): only works the author
  declares original — no fandom, no derivative basis — may carry a price. The
  declaration is the author's statement on the record, and a work declared
  original that is visibly derivative is a metadata-correction case (§15.11)
  that can lose monetization.
- **`any-with-assertion`**: the operator may allow fanworks to be monetized
  too, but the author must then assert they hold whatever rights their basis
  requires — permission from the original author where the basis demands it.
  The assertion is stored, versioned, and demanded again when the price
  changes. It is a statement of responsibility, not a copyright opinion: the
  instance is not the arbiter of who owns a fandom's characters, and §19.11
  (DMCA) remains the enforcement path when a rights holder disagrees.
- **`disabled`**: no monetization; credit tips (below) still work, because
  they move no money.

Monetizing an imported work (§11) follows the same rule as republishing it: a
source login demonstrates access, not permission. An imported work is never
monetizable in `original` mode at all.

### 20.9.2 Models

```text
Model: tips | early_access | purchase | patronage
```

- **Tips.** A reader sends the author money or credits. No unlock, no quid pro
  quo. Money tips land in the author's earnings ledger; credit tips transfer
  credits between wallets (§20.1) and are never convertible to money by the
  platform — credits are a community currency, and mixing them into payouts
  would turn every gamification rule into a money rule.
- **Early access.** A chapter carries a `public_at` moment later than its
  publication. Any reader may pay — or hold an active patronage — to read it
  now; at `public_at` it becomes free to everyone, permanently. Nothing is
  ever locked retroactively: a reader who read a chapter free keeps access to
  it even if the author later prices the work. This is the model that fits an
  archive built on scheduled publishing (§8.5): the author writes, supporters
  read early, the archive stays open.
- **Purchase.** One payment unlocks a complete work, or its future chapters,
  for that reader. A work later made free does not refund its purchasers and
  owes them nothing except the permanence of their access.
- **Patronage.** A recurring monthly pledge to an author, cancelable at any
  time, granting access to that author's early-access and purchased works
  while active. Patronage is between reader and author: it buys no platform
  entitlements, no badge outside the badge catalog, no trust (§19.1), and no
  ranking.

Every priced work remains fully eligible, filterable and searchable. A reader
who has not paid sees the work's complete public metadata — title, summary,
tags, content notes, length, reviews — and an honest paywall state on the
body, never a truncated teaser unless the author publishes one as a real
chapter.

### 20.9.3 Money rules

- **Separate ledgers.** Author earnings are money, accounted per author and
  paid out through the payment processor's payout flow. They never touch the
  credit ledger (§20.1), and the platform never converts credits to money in
  an author's favor. Tax, invoicing and payout compliance are the operator's
  setup, as §20.7 states for the marketplace.
- **Split.** A configurable platform fee applies to money payments — default
  85/15 in the author's favor — stated on every price the reader sees.
- **Refunds.** A reader may request a refund within a bounded window for a
  purchase whose content failed eligibility (§7.6) or was withdrawn unread;
  otherwise refunds are the author's decision through a defined flow.
  Chargebacks follow the processor's rules and suspend the entitlement
  pending resolution.
- **Entitlements are durable.** A purchase or active patronage is recorded as
  an entitlement bound to the account, surviving pseud renames, pseud
  switching (§7.2), and the work's later move between priced and free. Access
  checks read entitlements server-side like every other policy (§3.6).
- **No self-dealing.** The same-account pseud rules (§9.7.8) apply: purchases
  and tips between pseuds of one account are refused, and author earnings
  never accrue from the author's own reading.
- **Paid changes nothing social.** A paying reader's comments pass the
  positivity filter (§12.1) like anyone's; a paid work's ranking treatment is
  identical to a free work's. "Monetized" is a facet a reader may filter on,
  shown honestly, never a boost: no surface sorts or recommends by price or
  by earnings.
- **Privacy.** Purchases, tips and patronages are pseud-private (§3.7). A
  public supporters list exists only if the reader opts in, and an author sees
  totals, never a supporter roster they could expose.
- **Foundational protections hold.** Monetization purchases no trust, no
  moderation authority, no ranking (§0.3, §1.5). A paid work passes the same
  eligibility, age policy, content-note and rating-check requirements (§15.11)
  as a free one.

| Table | Important fields |
|---|---|
| `work_pricing` | work_id, model, price_minor, currency, public_at_offset, enabled, version |
| `work_entitlements` | account_id, work_id, kind, source_payment_id, granted_at, expires_at |
| `author_earnings_ledger` | author_account_id, amount_minor, currency, kind, payment_id, idempotency_key |
| `payouts` | author_account_id, amount_minor, currency, processor_reference, status, initiated_at |
| `monetization_assertions` | work_id, assertion_kind, policy_version, accepted_at, revoked_at |
| `work_ai_declarations` | work_id, declaration, declared_at, revised_at |

```text
POST   /api/v1/works/:id/pricing
DELETE /api/v1/works/:id/pricing
PUT    /api/v1/works/:id/ai-declaration
POST   /api/v1/works/:id/purchase
POST   /api/v1/works/:id/tips
GET    /api/v1/me/entitlements
GET    /api/v1/me/earnings
POST   /api/v1/me/payouts
GET    /api/v1/admin/monetization
```

### 20.9.4 AI content declaration

Every work carries an AI-usage declaration, set by its author and required
before monetization is enabled on the work:

```text
ai_content_declaration: none | assisted | co-written | generated
```

- **`none`** — no AI assistance.
- **`assisted`** — grammar, brainstorming, research only.
- **`co-written`** — substantial AI-generated text, human-directed.
- **`generated`** — primarily AI-generated.

The declaration is the author's statement on the record, versioned and
audited like the monetization assertions (§20.9.1). A false declaration is
a sanctionable offense escalating to permanent monetization ban (§19.6),
decided by quorum — a classifier may flag suspected undeclared AI for
review under §34.5's signals-never-verdicts contract, never decide it.

The declaration sets the work's Pool B multiplier (§20.10.4): `none` and
`assisted` at 1.0, `co-written` at 0.3, `generated` at 0 (Pool
B-ineligible). `generated` works remain fully available and eligible for
Pool A — tips and subscription attribution from readers who choose to read
them — because this is an economic boundary, not a content gate. The
declaration is displayed on the work page and exported with the work.

### 20.9.5 Processor fees

Every monetary transaction records the fee the payment processor deducted
(`payment_events.processor_fee_minor`). Pool calculations run on amounts
**after** processor fees: a €10 subscription with a €1.50 processor fee
contributes €8.50 to the pools (§20.10). The fee range actually paid over
the trailing 90 days is published on `/api/v1/meta` (§20.10.7) so the
solidarity arithmetic is transparent about what processing costs.

## 20.10 Redistribution floor

Author-bound revenue is split into two pools with different rules. The
per-flow split is configurable instance parameter, published on
`/api/v1/meta`, and changes require 90 days' public notice. The floor is
optional in the same way §20.9 is: a billing-disabled instance has none of
it, and an author who never monetizes sees nothing.

### 20.10.1 Pool A — direct attribution

Money with an obvious owner: tips to a named author, subscription revenue
attributed by reading time (§20.6.1), bounty payouts, marketplace sales.
Flows to the named author, subject to the cap (§20.10.3).

### 20.10.2 Pool B — solidarity pool

The overflow from the cap (§20.10.3) plus each revenue flow's Pool B
contribution (§20.10.6). Distributed to eligible authors by a
quality-weighted formula (§20.10.4). Pool B is money, never credits: it
enters the earnings ledger (§20.9.3) like any other author earnings and
respects the same separate-ledgers rule.

### 20.10.3 The cap

The cap applies to Pool A only. Money above the graduated bands spills into
Pool B; the author still receives their own Pool B share, because the cap
never excludes anyone from solidarity.

The reference point is the **active-earner median**: the median Pool A
income of authors who earned any Pool A income this month — not the median
of everyone who ever posted. The multiplier is a configurable instance
parameter (default 10), calculated over a trailing 3-month window to smooth
spikes and defeat month-end dumping:

```text
cap = cap_multiplier × median(active-earner Pool A income, trailing 3 months)
```

The cap is soft and graduated — a damper, not a cliff:

| Pool A income band | Author keeps | Spills to Pool B |
|---|---|---|
| up to 5× median | 100% | 0% |
| 5× – 10× median | 50% of the amount in this band | 50% |
| above 10× median | 0% | 100% |

Worked example (illustrative numbers): median €10, multiplier 10. An author
whose Pool A attribution is €60 keeps the first €50 (up to 5×) plus 50% of
the next €10 — €55 total — and €5 spills to Pool B.

The current median, cap value, overflow amount and the number of authors at
the cap are published (§20.10.7).

### 20.10.4 Pool B distribution

Pool B distributes proportionally to a per-author **quality score**, never
by word count, chapter count, or work count. An author with one excellent
oneshot can out-earn an author posting 50k words of mediocre content per
month. The quality score (0.0–1.0) is a weighted composite of signals the
platform already tracks (§9.7.4, §9.8):

```text
quality_score = 0.30 × completion_rate
             + 0.20 × reread_rate
             + 0.20 × positive_feedback_density
             + 0.15 × bookmark_rate
             + 0.10 × long_tail_engagement
             + 0.05 × reader_diversity
```

Each signal is normalized to 0.0–1.0 over the trailing 90 days. Reader
diversity counts distinct readers weighted by trust level (§19.1), so
farmed reads from throwaway accounts carry near-zero weight. The weights
are operator configuration, published on `/api/v1/meta`, changeable with
90 days' notice.

An author's Pool B share for a period:

```text
share = (quality_score × attributed_reading_time × ai_multiplier)
        / Σ(eligible authors: quality_score × attributed_reading_time × ai_multiplier)
```

AI content multipliers (§20.9.4) apply per work: a `generated` work
contributes nothing; a `co-written` work contributes at 0.3×. Pool B is
recalculated monthly and paid out on the same cadence as Pool A.

### 20.10.5 Eligibility floor

To receive Pool B distributions, an author must meet all of:

- At least one work with N distinct readers (N scales with instance size;
  default 20).
- Account age ≥ 30 days.
- Trust level ≥ TL1 (§19.1) — earned, never purchased (§0.3).
- Not currently under sanction (§19.6).
- No unresolved suspected-undeclared-AI flag on their works (§20.9.4).

The floor is fraud prevention, not gatekeeping: it exists so Pool B funds
real new authors rather than throwaway accounts. An excluded author sees
the reason and what would change it.

Authors opt into monetization per pseud (§7.2, §20.9):

- Pool A only (tips and subscription attribution, no solidarity).
- Pool A + Pool B (default).
- Donate their Pool B share back to the pool.
- Fully opt out (works remain available; no money flows).

### 20.10.6 Platform economics

All splits are configurable and published on `/api/v1/meta`:

| Flow | Platform cut | Pool B contribution | Author share (Pool A) |
|---|---|---|---|
| Subscriptions | 15% infrastructure | 15% | 70% |
| Tips | 5% | 10% | 85% |
| Bounties | 10% | — | 90% |
| Marketplace | 8% | 7% | 85% |
| Job fees (TTS/translation) | cost + 20% margin | split platform/Pool B | — |

The platform cut covers hosting, moderation, legal, and processor fees —
which for adult-content-compatible processors run far higher than
mainstream card rates (§2.2, §20.9.5). Quarterly financials are published;
surplus flows to Pool B by default unless the operator retains it as
reserve, disclosed.

### 20.10.7 Transparency

`/api/v1/meta` and a public dashboard publish:

- Total revenue this period, broken down by source.
- Total Pool A and Pool B sizes.
- Current active-earner median and cap value.
- Number of authors receiving Pool A, Pool B, both, neither.
- An anonymized distribution histogram: €0–10, €10–50, €50–200, €200–500,
  €500+.
- Platform infrastructure costs.
- The quality signal weights — the actual formula (§20.10.4).
- The processor fee range actually paid, trailing 90 days (§20.9.5).
- The Pool A/B split parameters and cap multiplier, with pending changes
  and their effective dates.
- The payment processor adapter's name — never credentials or account
  details.

**Never published:** individual author earnings, individual reader
activity, per-work revenue, supporter identities (§20.9.3's privacy rule).

### 20.10.8 Payout mechanics

- Minimum payout €20; below it the balance rolls forward.
- Monthly payouts through the instance's payment processor adapter, on the
  earnings ledger's payout flow (§20.9.3).
- Full ledger transparency to the author: every read, every attribution,
  every calculation is visible in their own earnings view.
- Payouts remain the operator's compliance surface (§20.9.3).

### 20.10.9 Tables

| Table | Important fields |
|---|---|
| `pool_b_distributions` | period_start, period_end, author_account_id, amount_minor, currency, quality_score, attributed_reading_time_seconds, ai_multiplier, idempotency_key |
| `monetization_period_summaries` | period_start, period_end, pool_a_total_minor, pool_b_total_minor, active_earner_median_minor, cap_value_minor, authors_in_pool_a, authors_in_pool_b, authors_capped, processor_fee_min_minor, processor_fee_max_minor |
| `work_ai_declarations` | work_id, declaration, declared_at, revised_at |

`payment_events` gains `processor_fee_minor` (§20.9.5). Migration 0049,
both dialects, following the §20.9.3 table conventions.

### 20.10.10 Acceptance

- An author earning 6× the median keeps 5× plus 50% of the band above it;
  the rest spills to Pool B (worked example: median €10, income €60 →
  keeps €55, €5 spills).
- Pool B distribution is quality-weighted, not volume-weighted: an author
  with one high-completion oneshot out-earns an author with five
  low-completion works despite fewer total reads.
- A `generated` work earns its author Pool A (tips, subscription
  attribution) and contributes nothing to Pool B.
- A `co-written` work's contribution to the Pool B numerator is multiplied
  by 0.3.
- Subscription revenue attributes by reading time: 30 minutes on author X
  and 10 on author Y splits the attributed fee 75/25.
- Processor fees are deducted before pool math: €10 with a €1.50 fee
  contributes €8.50.
- The eligibility floor excludes a 15-day-old TL0 account, and shows the
  author why.
- Self-dealing rules (§9.7.8) hold: same-account pseud tips are refused and
  an author's own reading never attributes subscription revenue to them.
- `/api/v1/meta` shows median, cap, pool sizes, fee range, weights, and
  pending parameter changes with effective dates.

## Acceptance

- Concurrent spending cannot overdraw.
- Retried webhooks do not duplicate grants.
- Failed jobs release holds.
- Standard jobs execute under paid load.
- Billing-disabled operation retains the core archive.
- Credits and monetary marketplace revenue remain distinct ledgers.
- Subscription tiers do not grant ranking, trust, or moderation authority.
- Wishlist bounties escrow correctly.
- Pool B is money on the earnings ledger, never credits; the credit ledger is untouched by redistribution.

---

# 21. Milestone 16: Marketplace, Extension Isolation, Webhooks, and Gallery Mechanics

Priority 1 (customization) is centered here.

## 21.1 Manifest

```text
id
version
category
entrypoint
required_host_api_version
permissions
resource_limits
supported_surfaces
license
network_allowlist
pricing (free | paid)
```

## 21.2 Categories

- Reader widgets.
- Dashboard widgets.
- Themes and layout presets.
- Recommendation engines.
- Declarative recipes.
- Search helpers.
- Writing tools.
- Challenge variants.
- Positivity filter rules extensions.
- Mood tag extensions.
- Fandom landing page templates.
- Integrations.

## 21.3 Capabilities

Examples: public metadata, selected preferences, user-selected draft, approved network domains, specific widget slots, mood classification, comment classification suggestions (never final classification without moderator review).

No implicit access to all drafts, messages, hidden pseud linkage, administrator taste profiles, arbitrary network destinations, source credentials, private feedback queues.

## 21.4 WASM execution

Use `wasmi` initially.

Enforce fuel, memory ceilings, output limits, host-call limits, bounded concurrency, process isolation where practical, worker termination for hard wall-clock limits, host-I/O timeouts.

Trust and subscription tiers change resource ceilings, not permissions.

Reference memory tiers:

```text
TL0 free 16 MiB
TL1 free 24 MiB
TL2 free 32 MiB
TL3 free 48 MiB
TL4 free 64 MiB
TL5 free 96 MiB
TL6 free 128 MiB

Reader subscription: +16 MiB
Author subscription: +32 MiB
Curator subscription: +64 MiB
```

Benchmark fuel rather than assuming direct milliseconds conversion.

## 21.5 Review workflow

```text
submitted → automated checks → permission review → security review
→ independent quorum approval → published
```

New permissions require renewed user consent. Provide emergency revocation and rollback.

Free extensions and paid extensions follow the same review process. Paid status never bypasses security review.

## 21.6 Gallery mechanics

Idempotent installation, one active installation per package and pseud scope unless explicitly designed otherwise, version pinning and update policy, uninstall, optional star rating and written review (subject to positivity filter for extension developers), review reporting, install counts with documented semantics, remix/fork lineage, license compatibility checks, attribution preservation.

Forking does not bypass review or copy user grants.

Do not inflate install counts through reinstalls or expose individual installation histories publicly.

## 21.7 Themes and layout safety

Tokens, constrained layout slots, scoped styling. Preview, reset, safe mode, accessibility checks, protected security and recovery controls.

Custom CSS is sandboxed:

- No `url()` to external origins (all resources must be inline or from approved storage).
- No `@import` from untrusted sources.
- No CSS exfiltration vectors (attribute selectors + external URLs).
- No arbitrary JavaScript through style attributes.

Themes cannot hide the positivity filter status, the feedback delivery mechanism, safety controls, or the extension management interface.

## 21.8 Webhooks

User-facing webhooks let extensions and integrations react to events:

- Per-pseud subscription with scoped events.
- Signed payloads with rotating secrets.
- Rate limits per subscription.
- Retry with exponential backoff and dead-letter after bounded attempts.
- Failed webhook subscriptions auto-disable after threshold.
- Webhook payloads respect the same privacy classification as API responses.
- Never include destructive comment content, source credentials, or hidden pseud linkage.

Webhooks are configured under `/settings/webhooks` per pseud.

## 21.9 Paid marketplace

Separate from credits: purchases, entitlements, refunds, developer revenue accounting, optional processor-backed payouts, configurable reference split of 85/15, operator tax/invoicing setup.

## Acceptance

- Infinite loops terminate.
- Memory excess fails safely.
- Host APIs enforce permission denial.
- Revoked packages stop running.
- Uninstall removes grants.
- Reset remains accessible after a broken theme.
- Forks preserve attribution and request their own grants.
- Custom engines and recipes honor opt-out.
- Extension reviews subject to the same positivity filter as work reviews.
- Subscription tier changes affect resource ceilings without opening new permissions.
- CSS cannot exfiltrate data through selector patterns.
- Webhook failures do not affect application performance.

---

# 22. Milestone 17: Translation Pipeline

Priority 7 is centered here.

## 22.1 Scope

Two layers:

1. **Interface translation:** all UI chrome, help text, error messages, documentation. Human-reviewed message catalogs. Established in Milestone 1.
2. **Content translation:** works, comments, forum posts, summaries, tags. On-demand AI with human review queue.

## 22.2 Content translation flow

```text
requested → generated/imported → private draft
→ submitted for review → changes_requested/approved/rejected → published
```

Requirements:

- Record source work and revision.
- Preserve attribution.
- Record translation permission basis.
- Distinguish human, machine-assisted, and machine-generated translation.
- Reviewer expertise scoped by language.
- Edits and decisions versioned.
- Source updates mark potentially stale translations.
- Publication requires permission and authorized approval.

A curator's approval is a quality/workflow decision, not a grant of copyright permission.

## 22.3 Cost model

Content translation costs credits per word. Reader subscription tier includes a monthly translation quota. Costs quoted before job submission with cancellation option.

Reader-initiated private translation (translating for personal reading only) uses a lighter workflow but still costs credits and respects permission. Private translations are never published without going through the review workflow.

## 22.4 Permission handling

Author of the original work controls whether translations may be published on Lorehaven. Options:

- No translation.
- Translation with author approval required.
- Translation permitted, review by community only.
- Translation permitted, machine-translated versions marked as such.

Imported works follow the original source's stated permission where available. Ambiguous cases default to review-required.

## 22.5 AI overrides human

Human translations always take precedence over AI translations of the same work in the same language. When a human translation exists, AI translation is disabled for that work-language pair unless explicitly requested for comparison.

## 22.6 Machine-translation labeling

Machine-translated content is clearly labeled at the work level, chapter level, and in metadata. Original language is preserved and always accessible. Machine translations do not replace the original.

## 22.7 Translation memory

Translation memory improves consistency and reduces cost:

- Per-translator memory is private by default.
- Translators may opt in to shared memory for their approved translations.
- Shared memory entries are scoped by language pair and domain (fandom).
- Shared memory never leaks private content (only approved published translations contribute).
- Users can query their own memory and browse shared memory in their language pair.
- Memory contributions are attributed.

## 22.8 Translation of comments and forum posts

Optional per-user setting: "Translate comments in languages I don't read." Uses cached translations shared across readers to reduce cost. Does not create a public "translated comment" — the translation is a rendering, not stored as a work.

## 22.9 Translation incentives

Human translators receive credits for approved translations (Milestone 15). Translators appear in the people directory as translators. Translation quality contributes to their reliability trust signal.

## 22.10 Translation search

Readers can search across all languages, filtered to their preferred reading languages. Translated works appear alongside originals with clear labeling. Search-language settings independent from interface-language settings.

## 22.11 AI cost controls

Cost estimation (§22.3) tells a requester what a job will cost. These are the ceilings that stop one requester, or one batch, from consuming an instance's month.

Budgets are separate per task class — translation, classification, summarization, search assist, transcription (§30.8) — because an instance may be willing to spend on one and not another, and a single pooled budget lets the cheapest-running task starve the one the operator actually cares about.

- **Per-account daily and monthly spend caps**, independent of the credit balance. A subscriber with a large balance still meets the cap, and the cap is on AI spend rather than on credits, so buying credits cannot raise it.
- **An instance-wide monthly budget** set by the administrator, with alerts at configurable thresholds (default 50%, 80%, 100%) delivered through §24.12.
- **Behavior at each threshold is stated and configurable**, with the honest default: at the soft threshold new jobs queue with the reason shown, and at the ceiling they are refused with an explanation and an estimate of when the budget resets. **A job already running is never killed mid-flight by a threshold it was admitted under.**
- **Cost is estimated before admission, and admission is what reserves it.** Two jobs admitted against the same remaining budget must not both be accepted; the reservation is the concurrency control (§3.4).
- **A job over a configurable cost threshold requires a second, explicit confirmation** naming the estimate, per job rather than as a standing consent.
- **The rules-only fallback is explicit:** when the classification budget is exhausted the positivity filter (§12.1) continues on its deterministic rules rather than failing open or refusing to accept comments, and the degraded state is visible to moderators.
- **Every AI feature has a configured provider and a configured absence.** With no provider, or with the budget exhausted, each task class degrades to a named non-AI behavior — search falls back to structured search (§23.8), summarization is absent, transcription is absent — and nothing silently returns an empty result as though it had run.
- Spend is reported per account, per task class and per provider, and the report distinguishes a quote from an actual charge.

## Acceptance

- Interface fully translated in initial locales.
- Content translation requires explicit request and permission check.
- Cost quoted before submission.
- Human translation cannot be silently replaced by AI.
- Machine translation labeled in every surface.
- Translation review does not grant copyright permission.
- Private reader translations do not become public without explicit publication.
- Translator credit awards are auditable.
- Shared translation memory contains only opted-in approved content.

---

# 23. Milestone 18: Public API, Bots, Feeds, Push, Federation, and AI Providers

## 23.1 Public developer API

The application API is a supported interface, not an internal detail.

Deliver: OpenAPI specification, interactive documentation, scoped tokens, explicit acting pseud, cursor pagination, rate-limit documentation and response headers, idempotency conventions, versioning and deprecation policy, example clients, error-code reference, security reporting guidance.

Publish only supported endpoints. Administrative and experimental APIs are marked separately.

Third-party tools receive the same authorization, content, privacy, and rate-limit checks as the frontend.

## 23.2 Chat bots as thin REST clients

Bot-client framework and reference adapters for Discord, Telegram, Matrix.

Each adapter gets separate fixture and live-verification status.

Supported initial actions: search eligible public works, fetch public metadata, start an authorized private import, check job status, request a permitted export, save a bookmark, return a link to continue on Lorehaven, send appreciation to authors.

Linking:

```text
bot issues short-lived linking challenge → user opens Lorehaven
→ signs in on Lorehaven only → selects pseud and scopes → confirms
→ bot receives revocable limited authorization
```

Bots never receive the user's Lorehaven password.

Security: tokens stored securely by the bot deployment, no private library results in public channels, private actions require private delivery or Lorehaven link, destination/channel context checked before every response, revocation and unlinking, explicit disclosure that the chat provider receives submitted messages.

## 23.3 Notifications

```text
domain event → notification eligibility → in-app record → optional email/push job
```

Apply privacy, rating, and positivity restrictions before generating text and again where necessary before delayed delivery.

Support: mention alerts, replies (positive only by default), source-update notices, credential expiry, batch completion, digests, delivery failures, positive feedback received, cheer received, wishlist fulfillment, saved-search alerts (§14.2), and subscriptions: a reader may subscribe to a work, a series, a collection, a fandom or an author and receive a notification when eligible new content appears. Subscriptions are the notification-bearing form of a follow: per-pseud, private, pausable, and bounded in delivery frequency by the same digest and quiet-period machinery as every other notification. A subscription never exposes the subscriber: the subscribed author sees a subscriber count, never a list, and the count itself is display-optional.

## 23.4 Push

Request permission after relevant user action, store subscriptions per device, remove invalid subscriptions, generic lock-screen text by default, best-effort delivery, no sensitive excerpts by default.

## 23.5 RSS, Atom, OPDS, and sitemaps

Public feeds, scoped revocable private feeds, OPDS catalogs with eligible download links, documented token rotation and revocation.

Never log private feed tokens. Private feed URLs are bearer secrets and require explicit disclosure.

XML sitemap for search engine indexing includes only public, eligible content. Private, unlisted, and restricted works are never included. Sitemap generation respects the same eligibility rules as public browsing.

## 23.6 ActivityPub

Initial scope: public opted-in pseud actors, follow/unfollow, work announcements, updates, deletion notices.

Signature verification, retry, replay and abuse controls, SSRF defenses, remote actor cache, instance blocks, opt-out and deletion documentation.

Do not federate private imports, reading history, hidden account linkage, private ratings, or held comments.

Explain that remote copies may persist after deletion notice.

## 23.7 AI provider interface

Tasks: translation, summarization, grammar assistance, prompt assistance, embeddings, optional metadata suggestions, comment classification, optional natural-language search assistance.

Supported providers include Ollama, OpenAI-compatible APIs, and custom adapters. Administrator selects providers per task. Users may opt out of specific providers.

Requirements: disabled without configuration, explicit private-text consent, quoted costs, cancellation, no automatic publication, generated output distinguished from author text, same permissions and exclusions as ordinary search, provider-specific retention and data-use disclosure.

## 23.8 Natural-language search assist

Optional AI-assisted search parsing:

- User types a natural-language query.
- AI parses into the typed search AST (Milestone 15).
- The parsed AST is displayed to the user for review before execution.
- The user can edit the AST or execute as-is.
- Search results always come from the AST, never directly from AI generation.
- Costs credits per query.
- Requires an AI provider to be configured.
- Falls back to structured search if unavailable.

The AI never bypasses permission checks, since the AST always runs through standard search. Silent misinterpretation is mitigated by showing the parsed AST before execution.

## 23.9 Fic trailers, mood boards, and rich sharing

Authors create visual/aesthetic pages for their works: images, playlists, quotes. Displayed as an optional tab on the work page.

Open Graph, Twitter/X cards, and embed previews for every public **eligible** work URL with title, author, summary, fandom, cover image (§31.1).

Cover images use content-addressed storage, permission checks, and size limits.

## 23.10 Federation beyond announcements

§23.6 federates follows and announcements. These are the cross-instance capabilities that need the data model to be right before they can be built, recorded now so that a later milestone does not have to reshape what exists.

- **Cross-instance search:** a reader searches a peer's public corpus from this instance through a federated query protocol rather than by scraping the peer's search pages. Results carry the peer's identity, are labeled as remote, and obey the peer's own eligibility decisions, which this instance cannot override.
- **Cross-instance wishlists:** a wish (§18.5) may be published to peers, and a fulfillment from elsewhere records which instance it came from and links back. A wish is never silently claimed on an instance the wisher does not use.
- **Shared tag canonicalization:** canonical forms of fandom, character and relationship names (§15.11) may be exchanged between peers as proposals, applied by an instance's own quorum, and never imposed. A peer's canonicalization is a suggestion with provenance attached.
- **Instance trust relationships:** an instance may declare another trusted, blocked or limited, with the same block semantics a user has (§7.5) at instance scope. Blocking an instance stops federation with it and is recorded publicly enough that a reader can see why a remote instance's content is absent.
- **Transfer manifests between instances:** a preservation batch (§11.11) may name a peer as its source together with the peer's own record of permission, so attribution survives the transfer.

Everything here is deferred. What this section fixes is the shape: identity, provenance and permission travel with federated records, and no instance's policy, moderation decision or canonicalization is overridden by a peer's.

## 23.11 Compatibility surfaces

None ships, and this section fixes the shape for the same reason §23.10 does: so that the first one to be wanted is built as a surface with a boundary rather than as a second API quietly growing inside the first.

A compatibility surface is an interface that exists to look like something else — another service's API, so that a client written for it can be pointed here, or a retiring instance's interface, so that its readers can be moved. This instance has no such clients to keep working, which makes one a liability rather than a courtesy: it pins this project to a shape its own design did not choose, and that shape then has to be carried forward.

If one is ever needed, it must take this shape:

- **Separate, and named as such.** Its own route prefix, its own handler module, its own documentation. It is never this application's API wearing a different hat, and it never changes §3.3's envelope: a caller wanting a legacy payload shape gets it on the legacy prefix and nowhere else.
- **Provenance-marked.** Every response says which surface answered, so a client can tell the emulation from the real interface and a reader is never shown a number this instance did not compute.
- **Contract-pinned by test.** Whatever is being reproduced — an identifier derivation, a field mapping, a payload shape, a slug — is pinned by a test that reproduces the original algorithm, not by a comment asserting equivalence.
- **Incompatibilities documented as known.** Differences are listed where the surface lives and are part of that surface's specification rather than defects for a user to discover. A surface that is faithful except for the parts it names is usable; one that is silently unfaithful is a trap.
- **No privileged path.** It passes the same authorization, eligibility, privacy and rate-limit checks as any other request (§23.1). Compatibility is not an exemption, and standing in for another service does not make its callers trusted.
- **Deletable.** Nothing in the instance depends on it, so removing it removes the surface and nothing else.

An instance may decline to run any compatibility surface. The default is that none is configured, and an instance that runs none is complete rather than degraded.

- API examples execute against test instances.
- Token revocation affects bots promptly.
- Public channels never receive private results through fallback behavior.
- Feed restrictions match reader restrictions.
- Delayed notifications recheck access and positivity.
- AI-disabled operation remains complete.
- Federation does not leak held comments.
- Sharing cards render for public eligible works only.
- Natural-language search shows parsed AST before execution.
- Sitemap excludes private and restricted content.
- A compatibility surface, where configured, answers only on its own prefix, carries its documented incompatibilities, and passes the same checks as any other request.

---

# 24. Milestone 19: Administration, Statistics, Abuse Defense, Privacy, and Operations

## 24.1 Administration routes

```text
/admin
/admin/users
/admin/policies
/admin/discovery
/admin/importers
/admin/import-batches
/admin/preservation
/admin/jobs
/admin/storage
/admin/retention
/admin/metadata
/admin/positivity
/admin/translations
/admin/billing
/admin/extensions
/admin/federation
/admin/analytics
/admin/abuse
/admin/audit
/admin/health
/admin/backups
/admin/quorum
/admin/registration
/admin/flags
/admin/dmca
/admin/media
/admin/sharing
/admin/alerts
/admin/succession
```

Show memory and disk pressure, job backlog, failed imports, source incidents, converter availability, backup age, moderation backlog, positivity queue depth, translation queue depth, database health, integration status, search index lag, credential-expiry counts without secret details, retention and deletion backlog, registration application backlog, active shadowbans.

Ordinary dashboards do not expose individual reading histories.

## 24.2 Public statistics

Provide `/stats` with eligible aggregate metrics:

- Public works.
- Public chapters and words.
- New public works by period.
- Public appreciation counts.
- Explicitly public bookmark counts.
- Aggregate eligible views.
- Active public contributors (documented definition).
- Source support status and public incidents.
- Positive feedback delivered (aggregate).
- Translation coverage by language.

Do not count private imports as public archive holdings. Public "works read" metrics require separately permitted aggregate input, cohort suppression, clear definitions.

Each metric documents definition, refresh interval, coverage, exclusions, approximation, retention. Suppress small or sensitive cohorts.

## 24.3 Privacy-preserving usage analytics

First-party aggregate analytics without third-party tracking by default.

Separate product metrics, security logs, personal reading statistics, recommendation learning.

Potential metrics: approximate DAU/WAU/MAU, active versus view-only sessions, aggregate action timelines, import success rate, search success rate, export completion, offline-download failures, positivity filter accuracy metrics, translation completion rates, endpoint usage aggregates.

Rules: do not call IP addresses or persistent identifiers "non-PII", avoid cross-site identifiers and browser fingerprinting, use short-lived or rotating identifiers where needed, establish applicable legal basis, apply consent where required, document estimation limits, never retain raw search queries or reading paths merely to produce broad counts.

## 24.4 Abuse dashboard

Restrict detailed security telemetry to authorized security staff.

Show request bursts, failed-authentication bursts, expensive-operation rates, export-to-request ratios, source-domain pressure, repeated limit violations, temporary mitigations and expiry, repeated held-comment authors, extension resource abusers, active shadowbans.

IP addresses and related identifiers are personal/security data: short retention, access audit, masked ordinary display, justified reveal, no public leaderboard of addresses.

## 24.5 Layered abuse defense

Required baseline: resource-specific rate limits, account/client/network layers, concurrency limits, upload and extraction limits, session and token controls, source throttling, anomaly alerts.

Optional escalating controls: accessible honeypots, form-timing signals, targeted proof-of-work for registration and login only, targeted browser challenges, manual review or alternative verification.

These signals must not alone establish abuse.

Do not: block solely because JavaScript is disabled, treat assistive technology as automation, blanket-block headless browsers, require expensive proof-of-work for ordinary reading or browsing, penalize shared NAT users through a single narrow global IP bucket, use invasive fingerprinting by default.

Every challenge needs an accessible fallback and a way to recover from false positives.

## 24.6 A/B testing

Feature variant testing is available but constrained:

- Never test safety, privacy, or moderation features.
- Users are informed that experiments are running (in site policy).
- Opt-out is available in settings.
- Individual assignment is not exposed to the user.
- Experiments have defined start, end, and success metrics.
- Results are documented internally.
- Sensitive features (age policy, content eligibility, credentials) are excluded.

A/B testing is an administrative tool, not a user-facing feature.

## 24.7 Data export

Export account settings, pseuds, authored works, private library metadata, permitted private content copies, bookmarks and notes, ratings and reviews, history and personal statistics, saved views and recipes, relevant messages, credit history, consent and authorization records, extension installations, watches.

Exclude plaintext source credentials, active session secrets, API tokens, private feed tokens.

Use a job and expiring authenticated download.

## 24.8 Deletion

Distinguish account, pseud, work, eligible work orphaning, private library copy, source credential, history, rating/review, integration authorization, local offline data, extension purchase.

Document retention exceptions and backup expiry.

Deletion propagates to search indexes, recommendation signals, caches, pending notifications, derived personal statistics, blob references, integration grants, held comments authored by the deleted account.

Shared physical blobs remain only while valid references or documented cache retention justify them.

## 24.9 Backups

SQLite: backup API or consistent snapshot. Do not copy only the main live database while ignoring WAL state.

PostgreSQL: supported backup tooling, file-storage consistency strategy.

Include encrypted-secret recovery planning. Operators must understand whether restoring a database without encryption keys makes source credentials unrecoverable.

Restore tests read restored works and validate private-library authorization.

## 24.10 Upgrades

```text
check compatibility → maintenance/coordination → backup
→ install verified executable → migrate → health check → resume
```

Do not assume replacing the binary reverses migrations.

Include database schema compatibility, search reindex requirements, IndexedDB migration compatibility, extension host API compatibility, recipe/query AST migration, encryption-key version support, positivity classifier version compatibility.

## 24.11 Succession, caretaker mode, and break-glass

A self-hosted instance with one administrator is one absence away from being unadministrable. The mechanism is documented, sealed, and deliberately awkward to use.

- **A designated successor** is named by the administrator from accounts at or above a configured trust level. Naming an account grants nothing while the administrator is present.
- **Break-glass activation** requires the administrator to be unreachable for a configured period, attested by a second person at or above the configured trust level, and a sealed recovery credential held outside the database — in the deployment's secret store, never in a row this instance holds.
- **Caretaker mode** is what a successor gets: the site keeps running and moderation and quorum continue, but no feature flag can be enabled, no AI budget raised, no billing change made, no extension installed, and the configuration surface is read-only apart from restoring service. Every action taken in caretaker mode is logged and published in the modlog (§19.12).
- **A returning administrator reclaims control** by presenting their own credential, which ends caretaker mode. The record of what happened while they were away is the first thing shown to them.
- **Critical operational documentation** — how to restore from backup, how to rotate the storage key, how to reach the registrar and the payment processor, what the instance's legal obligations are — is sealed the same way and is part of the backup set (§24.9).
- **The administrator's taste profile has a defined end.** If the administrator is absent beyond the configured period, influence from the taste profile stops rather than continuing to steer discovery on an absent person's preferences; discovery falls back to the baseline engines (§16.1) and says so.
- **No break-glass path bypasses the audit trail.** The mechanism exists to keep the instance administrable, and its whole value is that its use is visible.

## 24.12 Proactive alerting

The dashboard (§24.1) shows current state to somebody who is looking at it. Alerts are for the hours when nobody is.

- Channels: email, webhook and a chat-bot endpoint, each configured separately with its own destination and severity floor.
- Severity: `informational | warning | critical`, with the floor applied per channel, so a channel subscribed to critical only does not receive the informational stream.
- Conditions alerted at minimum: storage pressure above a configured fraction; job queue depth, failure rate and stalled leases; AI provider error rate and budget thresholds (§22.11); payment webhook failures; source credential mass expiry and per-source incident onset (§11.8); backup age beyond the configured interval; positivity queue depth and moderation backlog; migration or schema mismatch; certificate expiry; and a source's robots policy changing to disallow what an adapter needs.
- **Deduplication and hysteresis:** one alert per condition per window, and an alert clears only after the condition has been below threshold for a sustained period, so a metric oscillating at a boundary does not produce a stream of messages and closures.
- **Every alert names what to do**, not only what happened, and links to the relevant administrator page.
- **A status surface** may be enabled, public or internal, showing a coarse health state with no internal detail: whether the instance is operating normally, degraded or down, and never a count, an address or error text.
- **A channel's own failure is itself an alerted condition**, because a silently broken alert path is worse than no alert path.

## 24.13 Account dormancy and data lifecycle

Accounts outlive attention. The policy is stated, notified, and reversible for as long as it can be.

- **Dormancy is a notification first.** An account with no sign-in for a configured period receives notice that it is becoming dormant, what that changes, and how to prevent it. Nothing is actioned before the notice and its grace period have both passed.
- **A dormant account keeps everything.** No work, chapter, comment, rating, review, series, collection or message is deleted or hidden by dormancy. The account stays readable and its works stay published.
- **What changes is quota:** storage for new imports, active watches, queued jobs and AI spend caps fall to the dormant floor, while everything already held is retained. Existing imports are not evicted to make room for the rule.
- **Source credentials expire normally and are not renewed.** An expired credential is cleared as the credential policy already requires (§11.6), so a dormant account's stored secret is removed rather than left indefinitely.
- **Drafts are preserved and are not counted against active quotas.** A returning author finds their drafts as they left them.
- **Deletion is never automatic.** An account reaching an extreme dormancy threshold is not deleted; it is reported to the administrator as eligible for review, and any deletion then follows §24.8 including its notices.
- **A return restores the previous quota tier** without a support request, and the notices stop.
- The policy states its relationship to the instance's retention and privacy obligations (§24.7, §24.8) explicitly, including whether dormant data is included in exports and backups, rather than leaving the question to be inferred.

## 24.14 AI crawlers and scraping posture

The instance serves pages to people and to search engines that send readers.
It does not serve free bulk text to anyone else by default.

- **`robots.txt` is generated, not static.** Public eligible content is
  declared to well-behaved crawlers, AI-training crawlers are disallowed by
  default, and the disallow list is operator configuration with the default
  stated in the operator docs. A crawler that ignores `robots.txt` is not a
  policy problem but an abuse one, and §24.5's layered defenses apply to it
  like any other automated traffic.
- **No bulk text endpoints.** There is no export, feed, API scope or sitemap
  variant that returns complete bodies at volume to an anonymous or
  broadly-scoped caller. Download quotas (§13), API rate limits (§23.1) and
  the feed-token rules (§23.5) are the boundary, and they are already
  per-identity and bounded.
- **Authors state their wishes; the instance states its capabilities.** A work
  carries an author-set `ai_training` assertion — allow, deny, unset — shown
  as metadata and exported with the work. The instance enforces what an
  instance can enforce (no bulk endpoints, robots defaults, API terms); it
  does not pretend an assertion binds a scraper that never asked. The
  assertion's honest description is "this author's stated preference,
  recorded", never "protected".
- **Instance terms say it once.** Scraping for AI training is not an accepted
  use of the public API or the public pages, and access granted to a client
  that does it is revoked. That is a terms-and-abuse statement enforced
  through §24.4–24.5, not a technical guarantee, and both halves are stated.

---

# 25. Milestone 20: Hardening and Release

## 25.1 Functional browser journeys

Automate:

1. Register → create pseud → publish → read.
2. Apply for invite-only registration → moderator approval → register.
3. Import → update → bookmark → export.
4. Add source credential → import → expire → renew.
5. Bibliography preview → partial batch → restart → resume.
6. Add author watch → new work appears → import.
7. Preservation dry run → review → approved publication.
8. Cross-post a work → verify destination.
9. Download → disconnect → read offline.
10. Rate privately → publish review → verify visibility.
11. Record history → set reading goal → view progress → pause → clear.
12. Search bound character attribute → exclude ship → filter by mood.
13. Search with typo → accept suggestion → results returned.
14. Search private body phrase → verify no cross-user leakage.
15. Save query → pin view → migrate query version.
16. Change engine → install recipe → opt out.
17. Post positive comment → verify delivery.
18. Post negative comment → verify held → moderator review → decision.
19. Author opts in to critique → constructive comment → delivered.
20. Forum read state → mention → digest → reconnect.
21. Report → quorum → appeal.
22. DMCA notice → content restricted → counter-notice → quorum reversal.
23. Earn credits → reserve → capture/refund.
24. Request AI translation → cost quote → review → publish.
25. Install extension → deny permission → fork → uninstall.
26. Install paid extension → verify entitlement.
27. Configure webhook → trigger event → verify delivery.
28. Link bot → import privately → revoke.
29. Subscribe → verify quota changes → cancel.
30. Wishlist item → vote → claim → fulfill.
31. Natural-language search → parsed AST shown → edit → execute.
32. Export account → delete account.
33. Backup → restore to a clean instance.
34. Complete critical journeys in both initial locales.
35. Log in → earn daily credit → read chapters → earn action credits → finish work → earn completion credit → verify author earned credits.
36. View leaderboard → verify own rank → opt out of public display → verify hidden.
37. Earn badge → verify "Awarded ×N" display → verify global count in catalog.
38. Maintain reading streak → miss day → verify neutral reset message → use streak freeze → verify streak preserved.
39. Publish work in admin-taste category → verify author credits include silent demand multiplier → verify no taste-related label appears anywhere.
40. Suggest a feature → verify the raw text is stored → verify it joins an existing card or opens one → enter the arena → name best and worst → verify every implied rating moved.
41. Curator moves a card from Ideas to Up Next → verify the arena stops drawing it → verify the move is in the log with the actor and reason → verify the rating is unchanged.
42. Publish a changelog entry for a shipped card → verify the card links to the entry → verify the entry links to the card.
43. Import a fan film from a supported video platform → verify attribution, description, duration and tags → curate the tags → publish → verify it appears in search and on the fandom page.
44. Set an instance to reference-only → attempt a media upload → verify the refusal names the policy → import the same media by reference → verify no bytes were stored.
45. Open a media work with no transcript → verify the page says so → attach a machine transcript → verify it is labeled → replace it with the author's own → verify the machine revision is retained.
46. Share a public work → verify the card renders → request the card URL unauthenticated → verify it is indistinguishable between an unlisted work and a non-existent one.
47. Post a link to this instance's own work in the forum → verify the preview appears with no outbound request → post a link to an unknown host → verify it renders as a plain link.
48. Make a work private → verify a previously cached preview stops being served.
49. Register a new account → pick fandoms, moods, formats and content notes → verify the first session ends with a work opened and the reason stated.
50. Author enables tips on a work → reader tips in credits and in money → verify the credit ledger and the earnings ledger record them separately → verify no ranking change for the priced work.
51. Author sets early access on a chapter → paying reader reads it before `public_at` → non-paying reader sees the honest paywall state → after `public_at` everyone reads it → verify the paying reader's access persists after the work becomes free.
52. Reader subscribes to a work → author publishes a chapter → verify one notification → verify the author cannot see the subscriber list.
53. Save a search → enable its alert → a matching work is published → verify the notification names the view and respects the reader's permissions.
54. Change the accent colour → verify it applies to both layout presets → export the appearance bundle → import it on a second account → verify reader settings and dashboard layout carried across → set custom reader CSS on one work → verify it is scoped to the reading surface and absent elsewhere.
55. Author earns above the graduated cap bands → verify the overflow lands in Pool B → verify /api/v1/meta shows the median, cap value, and overflow.
56. Author declares a work `generated` → verify tips still credit Pool A → verify Pool B distributions exclude the work → verify the work page shows the declaration.
57. Author declares a work `co-written` → verify the Pool B numerator applies the 0.3× multiplier.
58. Subscriber reads author A for 30 minutes and author B for 10 → verify Pool A attribution splits 75/25 → verify the earnings ledger shows the reading-time weights.
59. Operator changes the cap multiplier → verify /api/v1/meta shows the pending change and its effective date 90 days out → verify the old multiplier still applies until then.
60. New account (15 days old, TL0) meets no other exclusion → verify Pool B excludes it → verify the account sees the reason.
61. Browse `/fandoms/HP` → `for-you` first → switch to `new` → paginate to the last page → the last
    page contains a work `for-you` ranked 500th → verify the reachable set and both counts are
    identical under the two sorts.
62. Anonymous visitor on an instance with undeclared topics → verify the browse default is neutral
    and no reason field names a theme term.
63. Operator declares one public topic → verify `/api/v1/meta` names it, the mechanism becomes
    documented, and a reader may see a coarse bucket for their own weight; declare it private
    instead → verify the bucket disappears and reasons degrade to one undifferentiated line.
64. Operator configures a cohort taste source of 3 accounts → verify startup refuses with a named
    reason; configure 5 → verify the source is accepted, and after one member withdraws the profile
    recomputes and no other reader's dial value changes.
65. Two readers (one aligned, one not) upvote the same wishlist item → verify the demand score moves
    differently for each → verify neither reader can see any weight, component or attribution.
66. Twenty aligned accounts upvote one bounty while two hundred readers upvote another → verify the
    unweighted majority is not reversed, and the disagreement routes to §19.4 quorum review.
67. Operator funds a large bounty with purchased credits → verify the bounty's fulfillment priority
    rises and its funder's demand weight does not move at all.
68. Operator sets `demand_diversity_percent` to 20 → verify at least 20% of surfaced demand carried
    no boost; set it to 0, or set `taste_floor × taste_ceiling > 1.0` → verify startup refuses both.
69. Two readers with identical contribution records in disjoint fandoms vote on the same item in
    fandom A → verify their weights differ there and match on an item in neither fandom.
70. One author publishes twelve mediocre works, another publishes one excellent work → verify the
    §9.7.4 quality signals separate them and the excellent work wins `top`, and no surface ranks the
    twelve by count.
71. Operator attempts a leaderboard category whose metric is words written or hours online, or an
    all-time placement → verify neither can be configured (§9.7.5, §28.10).
72. Reader contributes heavily, then stops for six months → verify their demand weight decays below
    a currently-contributing reader's, and that nothing anywhere accrued a lifetime tally.
73. Reader wins first place in a weekly leaderboard category → verify the reward is paid once, the
    next period starts empty, and no level, XP or all-time board exists to display the win.
74. Every §43.1 surface is probed with an unknown `sort` value, then with `for-you` while signed out
    on a private-topic instance → verify a named error in the first case and a neutral order in the
    second.

## 25.2 Security tests

Cover stored XSS, CSRF, SSRF and DNS rebinding, credential forwarding across redirects, file traversal, malicious archives and decompression bombs, object-level authorization, pseud isolation, search snippet and count leakage, shared-cache existence leakage, session/token revocation, credit races, webhook replay, plugin exhaustion, CSS resource exfiltration, private cache leakage, bot channel-context errors, email relay abuse, feed-token logging, retention and deletion failures, positivity filter bypass attempts, translation permission bypass, extension purchase entitlement bypass, shadowban visibility leaks, DMCA workflow abuse. Cover link-unfurl SSRF and outbound-request absence, media embed provider allow-list bypass, media reference leaking a private work, share-card and preview existence leakage for ineligible works, transcript leakage of a restricted work.

## 25.3 Accessibility

Automate what can be automated, then manually test keyboard-only navigation, screen-reader reading and forms, dialogs, editor, reader settings, command palette, dashboard reordering alternatives, query-builder errors, zoom and mobile layouts, reduced motion, contrast, challenge fallback, both appearance presets, comment interface with author feedback preferences.

## 25.4 Performance

Seed: 10,000 works, 1,000 accounts, realistic chapters and metadata, large works, dense tags, private imports, body-search documents, forum and bookmark activity, batch-import history, translation records, held comments, extension installations.

Benchmark idle memory, read latency, metadata and body search, fuzzy suggestion latency, progress writes, concurrent browsing, one heavy conversion, one AI translation, import backlog, bibliography enumeration, search indexing, extension execution, real-time catch-up, positivity classification throughput, disk growth and cache eviction.

Define "100 concurrent users" through a reproducible request mix and think time.

Record actual results. If a 1 GB profile fails, reduce budgets or mark the workload unsupported.

## 25.5 Platform and integration evidence

Address x86-64, ARM64, SQLite, PostgreSQL, supported browsers, PWA limitations by browser, converter versions, SMTP delivery, each website adapter, each bot adapter, each AI provider, federation peers used in tests.

Fixture success is not live verification.

## 25.6 Cognitive accessibility

Keyboard, screen-reader and contrast requirements are verified already (§25.3). Cognitive accessibility is a separate obligation and is tested separately, because a page can pass every automated check and still be unusable to a reader who has lost their place.

- **Predictable navigation.** Primary navigation, the position of a work page's actions, and the shape of a list are consistent across routes. A layout that moves between pages is a defect.
- **Controls do what their label says**, and a destructive control is never adjacent to a benign one without a confirmation or an undo.
- **Progressive disclosure on complex surfaces.** Advanced search filters, dashboard composition, the recipe builder and the taxonomy editor show their essential controls first and reveal the rest deliberately, and nothing a reader needs to finish the task is hidden.
- **Plain language in errors and help.** An error states what happened, what it means and what to do; it never leads with a code, and a code is something a reader can quote in a report rather than the message itself. Technical detail moves behind a disclosure instead of being removed.
- **Sensory controls beyond reduced motion:** a reduced-animation setting that removes transitions rather than shortening them, a lower-contrast-but-still-compliant preset, and a simplified page preset that drops decorative imagery and secondary panels.
- **Time and attention:** no countdown that expires a reader's work, no session timeout that discards an unsaved draft (§8.3 already preserves local text, and this states the expectation at the interface level), and no infinite scroll on a surface where a reader must be able to reach a footer.
- **Reading-position orientation** on long pages: which work, which chapter or unit, and how much remains, stated in text.
- Cognitive accessibility is part of the milestone's manual pass (§25.3's list) rather than a separate deferred item.

---

# 26. Route and API Ownership

| Surface | Frontend routes | API owner |
|---|---|---|
| Discovery | `/`, `/discover`, `/blind-date`, `/dashboard`, `/surprise` | discovery |
| Onboarding | `/welcome` | identity/discovery |
| Fandom pages | `/fandoms/:id` | discovery |
| Recipes | `/recipes/*` | discovery/extensions |
| Search | `/search`, `/search/advanced`, `/find-fic` | search |
| People | `/people`, `/u/:handle` | identity/content |
| Reader | `/works/*`, `/series/*` | content/library |
| Media works | `/watch/*`, `/listen/*`, `/media/*` | content/imports |
| Writing | `/write/*` | content |
| Imports/library | `/library/*` | imports/library |
| Watches | `/library/watches` | imports/library |
| Source status | `/sources`, `/sources/:id` | imports |
| Cross-posting | `/write/:id/cross-post` | imports |
| Community | `/forum/*`, `/groups/*`, `/messages/*` | community |
| Sharing | `/works/:id/card`, `/api/v1/unfurl` | content |
| Events | `/challenges/*`, `/requests/*`, `/wishlists/*` | community |
| Translation | `/translations/*` | translation |
| Governance | `/trust/*`, `/moderation/*`, `/curation/*`, `/appeals/*` | governance |
| Roadmap | `/roadmap/*`, `/roadmap/board`, `/roadmap/changelog` | community |
| Credits/billing | `/credits/*`, `/billing`, `/subscribe` | economy |
| Marketplace | `/marketplace/*` | extensions |
| Webhooks | `/settings/webhooks` | extensions |
| Help | `/help/*` | app/documentation |
| Developer API | `/developers/*`, `/api/v1/*` | relevant module |
| Statistics | `/stats` | restricted aggregate service |
| Settings | `/settings/*` | relevant module |
| Administration | `/admin/*` | restricted administration |
| Bot clients | external clients, linking under `/settings/integrations` | integrations |

Generate an endpoint inventory from implemented routes and compare with requirements in CI.

For every endpoint document authentication, acting pseud, authorization, request schema, response schema, error codes, rate limits, idempotency, privacy classification, cache policy, retention implications, public API stability status, positivity filter application, credit cost.

---

# 27. Tutorial Delivery Plan

Each chapter contains:

1. Starting checkpoint.
2. What will work by the end.
3. Concepts introduced.
4. Commands.
5. Exact file changes.
6. Important code explanations.
7. Tests.
8. Expected UI behavior.
9. Troubleshooting.
10. Commit/checkpoint.
11. Verification status.
12. Privacy and operational consequences.

Suggested checkpoints:

```text
v0.01-running-app
v0.02-design-system-i18n-help-slots
v0.03-identity-registration
v0.04-publishing-feedback-preferences
v0.05-reader-reactions-history-goals
v0.06-jobs-storage-secrets
v0.07-importing-watches-preservation-crosspost
v0.08-positivity-filter
v0.09-exports-offline
v0.10-library-saved-views
v0.11-search-taxonomy-mood-fuzzy
v0.12-discovery-recipes-dashboard
v0.13-community-presence
v0.14-events-wishlists
v0.15-governance-dmca
v0.16-economy
v0.17-marketplace-webhooks
v0.18-translation
v0.19-integrations-ai-search
v0.20-operations
v1.0-release
```

The tutorial, contextual help, API documentation, and operator documentation describe the same implemented behavior.

---

# 28. Final Completion Checklist

## 28.1 Customization (Priority 1)

- [ ] Extension slot architecture from Milestone 1.
- [ ] Themes and layout presets installable from marketplace.
- [ ] Recommendation engines and recipes installable.
- [ ] Widgets composable per pseud with multiple dashboard views.
- [ ] Extension permissions require user consent.
- [ ] Paid extensions honor entitlements.
- [ ] Reset and safe mode work with broken extensions.
- [ ] Webhooks allow event-driven integration.
- [ ] Custom CSS is sandboxed against exfiltration.
- [ ] Navigation customization respects safety controls.

## 28.2 Available fiction (Priority 2)

- [ ] Paste URL, drag file, and raw text import.
- [ ] Author bibliography batch import.
- [ ] Author watches with rate-limited scheduling.
- [ ] Cross-posting to external sites.
- [ ] Preservation batches with attribution.
- [ ] Source credentials encrypted and scoped.
- [ ] Runtime source health separate from support status.
- [ ] Completion rate as quality signal.
- [ ] Mood/tone search.
- [ ] Fuzzy matching with confirmation.
- [ ] Length histogram filter.
- [ ] Editorial curator picks.
- [ ] Fandom landing pages.
- [ ] Bookmark CSV import/export.
- [ ] Non-written works — video, audio, comic, zine — are importable, searchable and readable under the instance's media policy.
- [ ] Fan-film aggregation records attribution, description, tags, characters and duration, and a reference-only instance stores no media bytes.
- [ ] Every public work URL renders a share card, and a link posted in the community previews internally without an outbound request.
- [ ] New voices and first works are surfaced by a bounded, labeled discovery slot.
- [ ] Activity status distinguishes active, slow, dormant and concluded without penalizing any of them.
- [ ] Content notes are a separate axis from tags, reader-configurable and quorum-suggestible.
- [ ] Sorts are an explicit menu, exact sorts are exact, and random paging is stable within a request window.
- [ ] Saved-search alerts run with the reader's permissions at run time, are pausable, and never reveal matching activity to authors.
- [ ] Author notes, footnotes and endnotes are editor-native blocks, collapsible in the reader, excluded from word counts, and survive export.
- [ ] Text-to-speech reads any chapter, prefers a published podfic unit over generated speech, and stores nothing.
- [ ] A year-in-review page exists, is private by default, and shares history's deletion behavior.
- [ ] Reader layout modes and typography are saved per work and per reader, not per browser.
- [ ] Custom reader CSS is opt-in, sanitized, scoped to the reading surface, and off by default.
- [ ] Accent colour is choosable independently of themes, and the appearance bundle imports and exports as one file.
- [ ] Recipes are per-surface, forkable from shipped first-party recipes, diffable before install, and overridable for-now without editing anything saved.
- [ ] The administrator-influence control is a reader-settable dial whose zero is honored everywhere influence flows, and feed reasons name the reader's own matched terms.
- [ ] Subscriptions to works, series, collections, fandoms and authors deliver update notifications without exposing the subscriber list.

## 28.3 Positive feedback (Priority 3)

- [ ] Positivity filter classifies all author-facing comments.
- [ ] Destructive comments never reach author without explicit opt-in.
- [ ] Constructive critique opt-in per work.
- [ ] Quick reactions bypass filter as pre-classified positive.
- [ ] Anonymous appreciation notes.
- [ ] Reading milestone celebrations.
- [ ] Author feedback preferences per work.
- [ ] Moderator quorum reviews held comments.
- [ ] Repeated negativity from an account triggers rate limits.
- [ ] No public downvote counts or negative review aggregates.
- [ ] Ollama supported as classification provider.

## 28.4 Admin-enjoyable without monothematic (Priority 4)

- [ ] Admin taste profile drives writing prompts and recommendations.
- [ ] Diversity budget reserved in recommendations.
- [ ] Blind-spot suggestions.
- [ ] Surprise-me mode.
- [ ] Cross-fandom dynamic matching.
- [ ] Admin wishlist items appear without ranking advantage.
- [ ] Temporary taste boosts.
- [ ] Meaningful opt-out affects all discovery surfaces.
- [ ] Request candidates match user's own content only.

## 28.5 Trust-level self-governance (Priority 5)

- [ ] TL0–TL6 trust levels based on reviewed conduct.
- [ ] Higher trust grants moderation eligibility.
- [ ] Bootstrap mode labeled honestly.
- [ ] Bootstrap-to-community transition workflow.
- [ ] Administrator override logged with reason.
- [ ] Trust cannot be purchased.
- [ ] Credits and gamification rewards never advance trust level.
- [ ] Leaderboards exist for engagement but do not determine governance authority.

## 28.6 Quorum-based moderation (Priority 6)

- [ ] Two-reviewer minimum for routine curation.
- [ ] Three-reviewer minimum for high-impact changes.
- [ ] Appeals exclude original decision-makers.
- [ ] Positivity classification overrides quorum-reviewed.
- [ ] Extension approvals require independent quorum.
- [ ] Process feedback advisory only.
- [ ] Shadowbans require quorum and expire automatically.
- [ ] DMCA counter-notices route to quorum review.

## 28.7 Translation (Priority 7)

- [ ] Interface fully translated in initial locales.
- [ ] Content translation on demand with credit cost.
- [ ] Human translations override AI translations.
- [ ] Machine translations clearly labeled.
- [ ] Author permission per work.
- [ ] Translator credit awards.
- [ ] Optional comment translation.
- [ ] Shared translation memory with opt-in contribution.

## 28.8 Revenue (Priority 8)

- [ ] Credit ledger with balanced entries.
- [ ] Free tier retains core functionality.
- [ ] Subscription tiers grant quota, not authority.
- [ ] Marketplace revenue split with developers.
- [ ] Standard queues cannot be starved by paid load.
- [ ] Wishlist bounties escrow correctly.
- [ ] Webhook idempotency.

## 28.9 Foundational protections

- [ ] Private libraries and reading history stay private.
- [ ] Pseud linkage remains hidden.
- [ ] Source credentials encrypted and scoped.
- [ ] Extensions have bounded execution at every level.
- [ ] Age policy enforced server-side.
- [ ] Child safety controls operate independently of paid features.
- [ ] Data export excludes secrets.
- [ ] Deletion propagates across systems.
- [ ] Accessibility manually verified.
- [ ] Documentation matches delivered repository.
- [ ] Presence and typing indicators opt-in per pseud.
- [ ] Reading goals and streaks produce no coercive messaging, no guilt-framed loss notifications, and no credit multipliers tied to streak length.
- [ ] Gamification rewards never reveal the administrator's taste profile through any label, multiplier name, or breakdown.
- [ ] Badge counts display "Awarded ×N" per user and global rarity.
- [ ] An instance's media hosting policy is enforced with no per-work, per-user or per-peer override.
- [ ] No embed markup authored by a user, extension or peer is rendered; only the configured provider allow-list is.
- [ ] Unfurling makes no outbound request for an unknown host.
- [ ] A share card or preview never reveals a work the viewer may not read, and never differs between "not found" and "no access".
- [ ] AI spend has a ceiling that no request, subscriber or job batch can raise.

## 28.10 Deliberately not adopted

- **XP and level progression.** Not adopted. A lifetime personal counter — XP, levels, points — is refused for the same reason this section's metric rule exists: it compounds, so it ends up rewarding tenure and volume, it demands a source for every action, and it turns recognition into a race (§9.7.1). What replaces it is episodic and needs no new vocabulary: weekly and monthly leaderboards (§9.7.5) and badges (§9.7.6).
- **Volume-based leaderboards.** No "most words written," "most posts," or "most kudos given" categories, and no operator-added category whose metric is volume — for every category, including a per-public-topic one (§0.4.1), the metric is a quality or theme signal. A volume metric in a leaderboard is a specification error, not a configuration choice. No category is all-time: a placement expires with its window (§9.7.5).
- **Reputation scores.** Not adopted. Three things are refused: a lifetime personal total (XP, levels, points); a score that spans surfaces and compounds; and any score at all — visible or not — that determines trust or governance authority. A per-surface signal that decays on a schedule, confers nothing and weights nothing — §35.2's forum karma — is the boundary case, and it stays what it already is: it displays and does nothing.
- **Coercive streak mechanics.** No guilt-framed loss notifications, no streak-length credit multipliers, no "your streak will break" push notifications. Streaks are cosmetic with optional freezes.
- **Purchased trust or governance authority.** Credits buy compute priority, not moderation power or search ranking.
- **Visible admin taste influence.** The demand multiplier affecting author credits is never labeled, broken down, or hinted at in any user-facing surface. Authors see quality bonuses based on reader behavior only.
- **A promised source count.** Counts follow verified adapter support.
- **A permanent archive of every fetched body.** Shared infrastructure may deduplicate; public preservation requires explicit workflow.
- **Credentials as an SSRF exception.** Internal-network integrations require separate configuration.
- **Popularity-based moderation verdicts.** Process feedback is advisory.
- **Invasive or exclusionary bot detection.** No default fingerprinting, blanket headless-browser bans, compulsory JavaScript for reading.
- **Coercive reading analytics.** No mandatory streaks, no guilt-framed loss warnings, no leaderboard category ranking reading volume or streak length, and no credit multiplier tied to streak length.
- **Automatic publication of AI outputs.** All translations, suggestions, and generated content require review.
- **Purchased visibility or authority.** No paid ranking, trust, or moderation.
- **Silent public visibility of hidden comments.** If a comment is hidden from the author, it is hidden from the public work page.
- **Scripted recipes.** Recipes are declarative configuration; scripting lives in the WASM extension sandbox.
- **Multi-tenancy.** One instance per deployment. Multiple communities run multiple instances.
- **Community suggestions of other users' social links.** Only the author adds their own social links.
- **NodeBB import.** Not applicable to a new platform.
- **Author social proposals as a public queue.** Suggestions may be sent privately to the author, not published.
- **A roadmap that binds the operator.** The board is advice. No rating threshold ships anything, and no ballot overturns a decision that has been made.
- **Purchased roadmap position.** Credits, subscriptions, bounties and marketplace revenue buy no rating and no card movement.
- **Published individual ballots.** Ratings are public; who voted for what is not. A vote that can be seen is a vote that can be pressured.
- **A ranked list of contributors.** The board ranks ideas and never people.
- **Automatic promotion across a threshold.** Crossing a rating promotes nothing; a person decides, in the open.
- **Re-hosting third-party media.** An instance that references media stores metadata and a reference; it never mirrors a video or audio file it was not given permission to host.
- **Storing media an instance has not agreed to host.** The hosting policy is the operator's, stated once, and no upload, import, batch, cache or peer raises it.
- **Author-authored embed markup.** A player is a configured provider on a record, never a frame, script or object inside a document.
- **Unfurling arbitrary URLs.** Only this instance's own content and sources with an adapter are resolved; a preview is never an outbound request aimed by whoever wrote the link.
- **Autoplay, and tracking before consent, in media embeds.**
- **Inferring completion from a third-party player's playback position.**
- **AI spend without a ceiling.** Every task class has a budget, and exhausting it degrades to a named non-AI behavior rather than to silence.
- **Automatic deletion of dormant accounts.** Dormancy changes quota and ends credentials; it never removes what a person wrote.
- **A card or preview that reveals a work to someone who may not read it.**
- **Media hosting as a per-work decision.** "This instance does not host media" and "this media is a work on this instance" must both be expressible, and only an instance-level policy expresses both.
- **A compatibility surface for another service's API by default.** None ships. If an instance ever needs one, §23.11 fixes its shape: a separate prefix, provenance-marked, pinned by tests, with its incompatibilities documented and no privileged path.
- **Treating an unreadable page as an outage.** §11.14 separates a source's health from the quality of what it returned; one challenge page is a rejected candidate with a reason, not a degraded source.
- **Body retention as a per-work decision.** "This instance holds the words" and "this instance knows this work exists and points at it" must both be expressible, and only an instance-level setting expresses both without making every import a decision (§11.15).
- **An aggregating instance that caches "while it is there".** An instance set to `aggregate` stores no body from any path, including a cache fill that would have been convenient.
- **A caching instance that downgrades a failed fetch into a link.** A body that could not be fetched is a failed, retryable import, not a reclassification of the work.
- **Unmoderated guest commenting.** Comments require an account; anonymous appreciation notes (§8.4) are the anonymous channel, because holding and reviewing destructive content (§12) needs an accountable identity behind the text.

## 28.11 Gamification

- [ ] Daily login credits award correctly.
- [ ] Action credits respect daily caps and anti-gaming thresholds.
- [ ] Completion credits enforce minimum time-on-page.
- [ ] Author credits include quality multiplier with visible breakdown.
- [ ] Author credits include demand multiplier with no visible trace of admin taste.
- [ ] Leaderboards rotate on schedule and pay rewards.
- [ ] Leaderboard opt-out hides user from public display.
- [ ] Badges display "Awarded ×N" and global rarity count.
- [ ] Streak resets produce neutral messaging.
- [ ] Streak freezes deduct credits and preserve streak.
- [ ] Seasonal events run with boosted multipliers and unique badges.
- [ ] Credit earnings never affect trust level.
- [ ] Same-account pseuds cannot farm each other's author credits.
- [ ] New/low-trust accounts cannot farm author credits through reactions.
- [ ] No XP, level, lifetime point total or cross-surface reputation score exists on any surface; the only permitted per-surface signal is one that decays and confers nothing (§9.7.1, §35.2).
- [ ] No leaderboard category's metric is volume, and no category is all-time (§9.7.5, §28.10).
- [ ] A badge count never sums into a rank, gates a feature or orders a leaderboard (§9.7.6).
- [ ] Under every `sort` value, the reachable set for one filter is identical (§43.3).
- [ ] No demand weight, weight component or weight-derived label appears in any response, export, error or log line (§16.16.2).
- [ ] A weighted demand result never silently overturns the unweighted one (§16.16.3).

## 28.12 Community roadmap

- [ ] A suggestion is stored verbatim before any interpretation of it.
- [ ] A suggestion is never lost because the embedding provider was unavailable.
- [ ] Daily suggestion limit applies per identity, and signing out does not reset it.
- [ ] A card's representative text is written by its first suggester and never rewritten by later joiners.
- [ ] A card carries the requests that motivated it as evidence.
- [ ] Every card starts at the configured rating and is updated by pairwise comparison, not by a count.
- [ ] A best-worst answer is translated into the full set of pairwise updates, not into one.
- [ ] Rating updates are symmetric: a winner's gain equals the loser's loss.
- [ ] The arena draws least-compared cards first so a new idea is not last for lack of exposure.
- [ ] A pair a voter has already judged is not shown to them again.
- [ ] Every stage on the board is defined as data, and every column sorts by rating descending.
- [ ] No column offers a sort mode that ignores the rating.
- [ ] Moving a card out of Ideas freezes its eligibility according to configuration, and the platform's configuration is stated on the board.
- [ ] Voting requires the configured trust level, and a refusal explains itself.
- [ ] Moving a card is gated on trust level, never on a role.
- [ ] Moving a card never changes its rating.
- [ ] Every move is recorded with its actor, its from and to stage, and a reason.
- [ ] No rating threshold ships a feature; promotion is always a person's decision.
- [ ] The changelog entry for a shipped card links to the card, and the card links back.
- [ ] Only published changelog entries are listed.
- [ ] Who voted on what is not a surface anywhere in the product.

## 28.13 Non-written works and media

- [ ] A work carries a format, and a non-written work is a full citizen of search, collections, series, challenges, comments and federation.
- [ ] An instance states its media hosting policy once, and every surface honors it.
- [ ] A fan film imports with attribution, description, tags, characters and duration, and never as a bare URL.
- [ ] A media series imports as one work with one unit per episode, reconciled against the source's stated count.
- [ ] No player frame is stored or rendered from user, extension or peer markup.
- [ ] A work with no transcript says so; a machine transcript is labeled and replaceable.
- [ ] Unreachable media is marked, not deleted.

## 28.14 Sharing and link previews

- [ ] Every public eligible work URL renders a card and the metadata that points at it.
- [ ] No card or preview exists for a work the viewer may not read, and the markup is identical to the non-existent case.
- [ ] A link to this instance's own work previews with no outbound request.
- [ ] A link to an unknown host is a plain link and triggers no fetch.
- [ ] An author can disable unfurling, and the setting is honored everywhere.
- [ ] Sharing changes no ranking signal and awards no credits.

## 28.15 Monetization, subscriptions, and scraping posture

- [ ] Work monetization follows the instance's eligibility setting, and an imported work is never monetizable in `original` mode.
- [ ] Rights assertions are demanded when `any-with-assertion` is enabled and re-demanded on price changes.
- [ ] Money earnings and credit transfers are separate ledgers, and credits are never convertible to money by the platform.
- [ ] Early-access chapters unlock permanently at `public_at`, and no previously-free chapter is ever locked retroactively.
- [ ] Entitlements survive pseud switching and pricing changes.
- [ ] Purchases, tips and patronages are private; a supporters list is opt-in and the author sees totals, never a roster.
- [ ] Paid works gain no ranking, recommendation or moderation advantage, and their comments pass the positivity filter unchanged.
- [ ] Self-purchases and same-account pseud transfers are refused.
- [ ] Subscriptions meter resource-intensive features without removing the free tier's reduced form of them.
- [ ] AI-training crawlers are disallowed by default in generated `robots.txt`, bulk text endpoints do not exist, and the author `ai_training` assertion is displayed and exported as a stated preference rather than advertised as protection.

---

# 29. Community Roadmap, Feature Consensus, and Changelog

Readers propose what the platform should build next, the community ranks the proposals against each other, and an operator moves cards across a board and ships them. The ranking does not replace the operator's judgement; it makes the community's judgement legible before the operator exercises theirs. A wish list sorted by nothing tells an operator what people said. A ranked board tells them what people chose when they had to choose, which is a different and more useful thing.

Everything in this section is subject to §0.3. Ranking here is advice, never authority: it cannot purchase, confer, or override anything the foundational protections reserve.

## 29.1 Suggestions and clustering

A suggestion is 1–1000 characters of free text.

- **The raw text is stored before it is interpreted.** It is the reader's own words and the record of what they asked for; nothing derived from it may take its place.
- **Rate limit: three suggestions per identity per rolling day.** The identity is the acting pseud for a signed-in reader and a client identifier for a guest, so signing out does not clear the allowance. The limit exists because a suggestion system without one becomes a channel for one determined voice to look like a crowd.
- **Clustering.** The text is embedded and compared against the cards still in Ideas. Within the configured distance threshold it joins that card as another voice for the same idea; beyond it, it opens a new card seeded with its own text. Clustering is what keeps a board of forty cards from being a board of four hundred, and it is what makes a card's rating mean something: one idea, ranked once.
- **A card's representative text is written once, by whoever named the idea first, and later joiners never rewrite it.** A text that drifted toward the newest suggestion would make a card's name a function of arrival order rather than of the idea, and a reader who joined a card called "Better search filters" would find it renamed tomorrow.
- **Embedding is best-effort, and its failure is not the reader's.** When the embedding provider is unavailable the suggestion is stored unclustered and the reader is told it was recorded. A suggestion is never lost because a model was down, and the stored raw text is what makes the idea recoverable — and clusterable — once the provider returns.
- **A card carries the requests that motivated it** as evidence, so its existence is the record of a real demand rather than an operator's assertion that one exists.

## 29.2 Ranking: pairwise comparison, not vote counts

Cards are ranked by a rating derived from best-worst comparison (MaxDiff) rather than by an upvote count.

- Every card starts at the configured seed rating with the configured K-factor, and the values are configuration, not constants in code.
- A voter is shown four cards and names the best and the worst. That one act becomes the full set of virtual one-to-one matches implied by it: the best beats each of the other three, the worst loses to each of the other three, and each of the two between them beats the worst, loses to the best, and draws the other.
- **Why not upvotes.** An upvote measures how many people saw an idea; it does not measure how much they prefer it. Two popular but interchangeable ideas both accumulate votes and neither is distinguished, and an idea that is widely liked but never anyone's favourite outranks the one a community actually wants built. Forcing a choice produces an ordering, which is the only thing a roadmap needs.
- **The update is symmetric.** A winner gains exactly what the loser drops, so a board cannot inflate as votes accumulate and a card's rating means the same thing in a quiet month as in a busy one.
- **The arena draws the least-compared cards first**, with a random tiebreak among them. A card's position must reflect preference rather than exposure: without this, a new idea sits last because nobody has seen it, and stays there because nobody sees a card that is last.
- A pairing a voter has already judged is not shown to them again.

## 29.3 The board, its stages, and what freezes

A card is in exactly one stage. Stages are defined as data — identifier, label, colour, board position, and whether they are open to comparison — so the board's shape is configuration rather than a list hard-coded into the interface. Board position determines left-to-right order, and the shape a deployment ships is:

| Position | Stage | Label | Open to comparison |
|---|---|---|---|
| 1 | `up_next` | Up Next | no, by default |
| 2 | `in_progress` | In Progress | no, by default |
| 3 | `finished` | Finished (not yet shipped) | no, by default |
| 4 | `shipped` | Shipped | no, by default |
| 5 | `medium_term` | Medium-term | no, by default |
| 6 | `long_term` | Long-term | no, by default |
| 7 | `idea` | Ideas | yes |
| 8 | `rejected` | Rejected | no |

- **Moving a card out of Ideas freezes it by default**, so a decision the operator has already made cannot be dragged around afterwards by traffic that arrives later. A deployment may open every stage except Rejected, so consensus keeps accumulating on planned work; whichever it does, the board says which, because a reader who cannot tell whether their vote still counts has been told nothing.
- **Every column sorts by rating, descending, and no column offers a mode that ignores it.** A "popular" sort beside a rating sort would mean two answers to the same question, and the one people clicked would become the real one.
- **The rating is shown on the card.** A board that ranks by community preference and hides the preference asks readers to take the ordering on faith.
- Categories (search, scraper, social, reader, admin, recs, general, and their labels) are defined as data as well and filter across all columns rather than within one.
- Rejected is reachable and carries a reason. A suggestion system that cannot say no teaches readers that their ideas were not read.

## 29.4 Who may vote, who may move, and what a move means

- **Voting requires the configured minimum trust level** (§19.1), and a reader below it is refused with an explanation rather than shown a ballot that will be discarded.
- **Moving a card is gated on trust level, not on a role.** The platform has no role column; the gate is the same trust ladder that governs every other curation tool, and it is set high enough to be a deliberate appointment rather than a consequence of participation.
- **A move never changes a card's rating.** The community's ordering and the operator's decision are separate facts, and a system that adjusted the rating to match the stage would have quietly replaced the first with the second.
- **Every move is recorded** with its actor, the from and to stage, and a reason, in the same public log that moderation actions use (§19.12). A roadmap whose cards move with no record invites the belief that they move for reasons nobody will name.
- **No rating threshold ships a feature.** Crossing a number promotes nothing; promotion is always a person's decision, taken in the open.

## 29.5 Changelog

Shipped work is announced, and the announcement is linked to the demand that produced it.

- An entry has a date, a title, a body, one of three kinds — New, Improved, Fixed — and an optional link to the card it shipped.
- Entries have a draft and a published state, and only published entries are listed.
- **The link is bidirectional**: a shipped card points at the entry that shipped it and the entry points at the card. The purpose is that a reader who voted for something can find out what became of it without asking, which is the difference between a roadmap and a suggestion box.

## 29.6 Privacy

- **Which reader voted on what is not a surface anywhere in the product.** Ratings are public; individual ballots are not. A visible vote is a vote that can be socially pressured, and a roadmap where readers can see each other's choices ranks the ideas that are safe to endorse rather than the ones people want.
- A suggestion is attributed to a pseud and the pseud's linkage rules apply as everywhere else (§7.2). A reader can see their own suggestions; nobody can see another reader's ballot.
- The board is not a place for the demand multiplier or the administrator's taste profile to be inferred (§0.3). Board order derives from reader comparisons and from nothing else.

## 29.7 Data model

| Table | Holds |
|---|---|
| `feature_clusters` | One card: representative text, its embedding, its rating, how many comparisons it has taken part in, how often it was named best and worst, its stage, its category, and its provenance. |
| `feature_suggestions` | One raw suggestion: its text, its identity, when it was made, and the card it landed on or none if it could not be clustered. |
| `feature_votes` | One ballot: the cards it compared, which was named best and worst, and who cast it — retained so a voter is not shown the same pairing twice, and not exposed. |
| `roadmap_stage_events` | One move: the card, the from and to stage, the actor, the reason, and when. |
| `roadmap_changelog` | One entry: title, body, kind, the card it shipped, its author, and its published state and moment. |

Counts and ratings are ordinary columns, not derived views: a rating that had to be recomputed from every ballot would make the board's cost grow with the community's engagement, which is the opposite of what a growing community needs.

## 29.8 API

| Method | Path | Purpose | Minimum trust |
|---|---|---|---|
| POST | `/api/v1/roadmap/suggestions` | Record a suggestion and cluster it if possible. | any reader (rate limited) |
| GET | `/api/v1/roadmap/arena` | Return the cards to compare next. | any reader |
| POST | `/api/v1/roadmap/arena/votes` | Record a best-worst ballot and apply its rating updates. | voting minimum |
| GET | `/api/v1/roadmap/features` | List cards by stage and category, rated descending. | any reader |
| GET | `/api/v1/roadmap/board` | Every column, grouped and rated descending. | any reader |
| PATCH | `/api/v1/roadmap/features/{id}` | Move a card between stages with a reason. | curation minimum |
| GET | `/api/v1/roadmap/changelog` | Published entries, newest first. | any reader |
| POST | `/api/v1/roadmap/changelog` | Write an entry, optionally linked to a card. | curation minimum |

## 29.9 Acceptance

- A suggestion is readable in its own words after a clustering failure, and clusters correctly once embedding succeeds.
- A best-worst ballot moves all the ratings the pairing implies, and the sum of gains equals the sum of losses.
- A card moved out of Ideas is no longer drawn by the arena under the freezing configuration, and remains available to it under the open one.
- A reader below the voting minimum is refused, and the refusal explains the requirement.
- A reader below the curation minimum cannot move a card, and the attempt is recorded rather than silently dropped.
- The board renders every stage in its configured order, each column rated descending, with no alternative sort.
- A move appears in the log with its actor and reason, and the card's rating is unchanged by it.
- A published changelog entry and the card it shipped link to each other.
- No request in this section returns a ballot cast by another reader.

---

# 30. Non-written Works, Media, and Aggregation

Written fiction is one format among several. Fan films, podfics, fan comics and zine scans are part of the same fandom, and an instance that catalogues only prose is not an archive of fandom — it is an archive of one corner of it. This milestone makes format a first-class property of a work, **without requiring any instance to store media it does not want to store**.

## 30.1 What counts as a work

Every work carries a format:

```text
Format: prose | poetry | fan_film | podfic | audiobook | fan_comic | zine_scan | interactive | other
```

A work's format determines how its units are rendered and which capabilities apply. It never determines eligibility for search, ratings, collections, series, challenges, comments or federation. A podfic is a work. A fan film is a work.

A non-written work is the same kind of record as a written one: owning pseud, contributors, tags, fandoms, characters, relationships, content notes, rating, visibility, lifecycle, completion, series and collection membership, comments, reactions, bookmarks and credit events. The differences are the unit type and the hosting policy below.

## 30.2 Hosting policy is an instance decision, stated once

Each instance declares what it will store. The setting applies to the instance, never to a request, a work or an uploader:

```text
Media hosting: host | reference | catalogue
```

- **`host`** — media bytes are uploaded, stored content-addressed, and served from this instance, under the same retention, backup and deletion rules as any other object.
- **`reference`** — metadata is stored and the media is played or opened at its origin. No bytes are ever fetched into this instance's storage: not by an import, not by a preservation batch, not by a cache, not by a federated announcement.
- **`catalogue`** — metadata only, with no playback shell embedded at all. The work page presents the work and links out.

The setting is the operator's, and an instance that chooses `reference` or `catalogue` is a complete instance: every other feature in this specification behaves identically. No work, contributor, importer, extension or federated peer may raise the setting on its own behalf. An instance set to `reference` refuses an upload rather than silently storing a file, and the refusal names the instance's policy.

This is why the preference is an instance setting and not a per-work flag. "My instance does not host media, and the fan film it aggregates is still a work" is the combination to be expressible, and a per-work flag cannot express it without turning every work into a decision somebody has to make.

Media bytes are one axis; the text of an imported work is another. This section governs the bytes and §11.15 governs the text, and the two are independent: an instance may host media while aggregating text, or hold nothing but text. They follow one principle, stated in both places because it is the same principle — the operator decides it once, and no work, uploader, importer, extension or federated peer raises it.

**The two settings deliberately do not share a vocabulary, or a number of states.** Media needed a third value: `catalogue` renders no player shell at all, which is a different thing from playing at the origin, because media has a shell to omit. Text has no shell to omit — a work whose body this instance does not hold is read at its origin in the reader's own browser, and there is nothing here to render or to refrain from rendering. A third text value would be a state with no behaviour behind it, and one shared vocabulary would mean either giving text a mode it cannot act on or taking `catalogue` away from an operator who needs it. The names differ because the choices differ.

## 30.3 Media references, and where the metadata comes from

When hosting is `reference`, a media work is an external reference: a canonical URL, the source it belongs to, and the metadata this instance holds about it.

For a site with an adapter (§11.1) the adapter extracts the metadata and the reference is an ordinary imported record. For a fan film on a video platform, the adapter reads the public page and records:

- Title, as the author gave it.
- Channel or uploader, recorded as the work's attribution — never merged into a local account by matching names or handles (§11.11).
- Canonical URL and the platform's own identifier for the item.
- Duration, where the platform states one.
- Publication date, or **no date**. A guessed date is worse than an absent one.
- Description, sanitized and stored as the work's summary, with the boilerplate platforms append to descriptions removed rather than displayed.
- Thumbnail reference, subject to §30.6.
- Tags, fandoms, characters, relationships and content notes (§15.16), drawn from the description or the platform's own fields and **verified or corrected before the work is public** — by the original author if they are on this instance, otherwise by a curator. A fan film's tags are frequently empty, wrong, or a keyword wall, and an uncurated import that displays them is presenting noise as taxonomy.
- A transcript or caption track reference where one exists.

A record with only a URL is not a work. A media work becomes public only when it carries attribution, a description or a transcript, at least one fandom, and a format.

## 30.4 Units

A work's chapters generalize to units. A unit is text or media, and one work may contain both — a podfic with a text version, a fan film series with written episode notes, a comic with the author's commentary.

Each unit records its position, its title, its runtime where it has one, its own external reference or storage key, and an optional transcript.

An imported media series — a fan film in episodes, a podfic in parts — imports as one work with one unit per episode or part, discovered from the source's own listing the way chapter lists are walked (§11.7). The count is reconciled against the source's stated total, and a disagreement refuses the import naming both numbers.

## 30.5 Playback and embedding

Playback is served by this instance's own player shell. Third-party embed markup is never accepted from an author, an importer, an extension or a federated payload, and the work document schema (§8.3) stays free of frames, scripts and embedded objects: a media unit is a field on a record, never markup inside a chapter.

The shell renders a provider drawn from an allow-list that is configuration rather than code. The initial providers are those the initial adapters support; adding one is an operator decision recorded in the ADR set (§1.4).

Rules that hold for every provider:

- No autoplay, and no audio before a reader-initiated action.
- No third-party cookies or trackers before consent; a privacy-preserving embed host is used where the provider offers one.
- No tracking parameter is forwarded, and no referrer beyond the origin is sent.
- The player is replaced by a plain link when the provider is unreachable, when the reader has disabled embeds, or when the instance's hosting policy is `catalogue`. A work never becomes unreadable because an embed failed.
- A reader-level setting disables embedded players everywhere, and it is honored with no per-work exception.

## 30.6 Thumbnails, covers and caching

Thumbnail and cover references are stored, not mirrored, unless hosting is `host`. Displaying a platform thumbnail uses the provider's own image URL with a size hint, and sends nothing about the reader beyond what a plain image request carries.

Metadata extracted from a platform is cached with a short retention and refreshed through the job queue on a schedule derived from the instance's crawl policy. The media itself is never cached, never proxied, and never used to serve a byte this instance has no permission to serve. A cache miss re-fetches the page, not the media.

Search indexes hold metadata only. A private or restricted work's media reference never lands in a shared cache (§10.4).

## 30.7 Discovery, search and eligibility

Format is a filter and a facet everywhere works are filtered: search, saved views, the library, collections, series, challenges, recommendations and statistics.

Discovery is not prose-only by default. A reader who has expressed no format preference meets media works in proportion to their presence in the eligible corpus — neither buried beneath it nor promoted above it.

A media work takes part in mood search, tag search, the people directory, fandom landing pages and the recommendation engines on the same terms as a written one. The engine inputs that assume text are not applied to a work that has no words: runtime substitutes for length, and the length histogram (§15.15) uses runtime for these works rather than omitting them. Completion for a media work means the reader reached the end of the last unit, recorded by the same explicit action a written work uses (§9.8), and **never inferred from a third-party player's playback position**.

## 30.8 Accessibility

A media work states whether a transcript, a caption track, or neither is available, and the work page says which. "No transcript available" is displayed rather than omitted, because a reader who needs one is deciding whether to start.

Where a transcript exists it is readable as text on the work page and is included in full-text search for that work, under the same privacy rules as any other body text.

Auto-transcription is an AI task: it costs credits, quotes before it runs, respects the budget guardrails (§22.11), is labeled as machine-produced (§22.6), and is never presented as the author's own words. An author may replace a machine transcript with their own; the machine version is then retained as a revision rather than overwritten.

A work with neither a transcript nor a caption track is listable and searchable, and is marked in the accessibility facets so a reader filtering for transcripts can exclude it deliberately rather than by accident.

## 30.9 Aggregating media from other instances

A federated media work is announced by reference. The receiving instance applies its own hosting policy: an instance set to `host` may fetch and store media it is permitted to fetch, one set to `reference` stores the announcement and the metadata and plays at the origin, and one set to `catalogue` lists it and links out.

No instance's policy changes another's. A peer that hosts media does not cause a reference-only instance to store bytes, and a reference-only instance does not cause a peer to stop announcing.

## 30.10 Legal and takedown

Where this instance does not host the media, a takedown concerning the media itself is answered by stating that this instance holds metadata and a reference, identifying the host, and complying with any requirement that the listing itself be removed. §19.11 applies unchanged, including the counter-notice path and the modlog.

Where this instance hosts the media, the ordinary copyright workflow applies to it exactly as to uploaded text.

A media reference whose origin becomes unreachable is marked as such rather than deleted, and the work remains in the archive as a record that it existed — the same treatment §11.13 gives a vanished source.

## 30.11 API

```text
GET    /api/v1/works/:id/units
POST   /api/v1/works/:id/units
PATCH  /api/v1/units/:id
DELETE /api/v1/units/:id

POST   /api/v1/media/references
GET    /api/v1/media/references/:id
POST   /api/v1/media/transcripts/:unitId
GET    /api/v1/works/:id/transcript

GET    /api/v1/admin/media/policy
PATCH  /api/v1/admin/media/policy
```

## 30.12 Data model

| Table | Important fields |
|---|---|
| `work_units` | work_id, position, unit_kind, title, runtime_seconds, media_reference_id, chapter_id |
| `media_references` | canonical_url, source_key, provider, provider_id, duration_seconds, published_at, transcript_blob_id, availability, last_checked_at |
| `unit_transcripts` | unit_id, origin (author / machine / provided), language, blob_id, revision |
| `instance_media_policy` | hosting_mode, provider_allowlist, updated_by, updated_at |

`works` gains `format`. `chapters` remains the text unit and is referenced by `work_units` rather than replaced, so existing content and every path that reads a chapter keep working.

Where the instance hosts media, a unit's bytes live in `content_blobs` (§4.4) under a media retention class, exactly as an imported chapter's text does. `media_assets` (§4.3) is unaffected and remains what it is: assets an author uploads *about* a work, such as a cover or a mood-board image. That is a different question from media that *is* the work, and the two are deliberately not merged.

## 30.13 Acceptance

- A fan film on a supported video platform imports with title, attribution, description, duration and a canonical link, and is public only once it carries a fandom, a description and a format.
- An instance set to `reference` refuses an upload and names its policy, and no import, batch, cache or federated announcement stores media bytes on it.
- An instance set to `catalogue` renders no player and links out, and the work page remains complete.
- A media work appears in search, mood search, fandom landing pages and collections, and filters by format.
- A media work is excluded from word-count and reading-time filters, and the length histogram uses its runtime.
- Completion for a media work is recorded by an explicit reader action and never inferred from a third-party player.
- A media series imports as one work with one unit per episode, and a count disagreement refuses the import naming both numbers.
- No embed markup authored by a user, an extension or a peer is ever rendered; only the configured provider allow-list is.
- No stored page or payload contains a player frame, and the editor document schema is unchanged.
- A work with no transcript says so, and a work with one is full-text searchable through it.
- A machine transcript is labeled as machine-produced and can be replaced by the author without destroying the machine revision.
- Disabling embeds instance-wide leaves every media work readable as a link.
- Unreachable media is marked, not deleted, and the work remains in the archive.
- A takedown of non-hosted media identifies the host, removes the listing if required, and follows the §19.11 workflow including the counter-notice.

---

# 31. Sharing: Fic Cards, Unfurling, and Link Previews

Fiction is shared by link, and a link that shows a title, an author and a fandom travels where a bare URL does not. This milestone makes every shareable work render a card, and makes a link posted inside the community resolve to the work it names.

## 31.1 The fic card

Every public eligible work URL renders a card image, server-side and deterministically, for social platforms and for direct download.

The card carries: title; the authors as displayed; fandom; format; rating; completion state; length as word count or runtime; up to the configured number of content notes; the instance name and base URL; and a short link that resolves to the work.

The card is generated from the work record at request time and cached by content hash and template version, so it is stable while the work is unchanged and regenerated exactly when the work changes.

Rules:

- **Public eligible works only.** A draft, a withdrawn work, a restricted work, an unlisted work, and a work the requester may not read render no card, and the response for a non-eligible identifier is identical to the response for a non-existent one, so the endpoint is not an existence oracle (§25.2).
- **The summary appears only if the author allows it**, truncated rather than reformatted. The author controls the cover image, the summary, the content notes, and whether co-authors are named.
- **A card never un-masks an author.** A work published under a pseud the author has hidden does not name it.
- **No reader data on the card.** No reading counts, no reader identities, and no per-viewer variation that could reveal who looked.
- The image is produced by this instance, and no third-party card service receives the work's text.
- A text-only fallback describes the work for platforms that do not render images, under the same eligibility rules.

## 31.2 Sharing from the interface

A work page and the end-of-work page (§9.10) offer: copy link, open the card image, download the card, and a share action that opens the platform's own share URL for the reader's choice. **The instance never posts to a platform on a reader's behalf.** A reader's own note may accompany a shared link, bounded in length and added by the platform's composer rather than by this instance.

## 31.3 Unfurling inside the community

When a post, comment, message or profile field contains a URL, the server may resolve it into a preview card at render time. Previews are rendered server-side, so they appear without JavaScript and to every client, including the API and the federated representation.

**What is resolved, and what is not:**

- **A URL on this instance is rendered from internal data, with no outbound request.** The preview links internally and shows the work, series, collection or thread it names, respecting the reader's access: a work the reader may not read produces a preview that says a work exists and offers the link, never its metadata. This is the common case and the one that matters most — a link to a work here never leaves the instance to render, so there is nothing to scrape and nothing to leak.
- **A URL on a known source** (a site with an adapter, §11.1) may be rendered from a cached metadata record, with attribution and a link to the origin. The fetch that fills that record happens through the job queue under the source's robots and pacing rules (§11.5) and the revision cache (§10.4) — never inline, on request, in the rendering path.
- **Every other URL is a plain link.** No outbound request is made for it, ever.

That last rule is the security-relevant one and it is not a limitation awaiting a later milestone. Fetching an arbitrary URL to build a preview is an outbound request that any signed-in account can aim wherever it likes; it is the same hazard the import path closes with a pinned resolver and an explicit policy (§11.5), and a rendering path cannot apply that policy and remain a rendering path. The set of unfurlable origins is the set this instance already trusts: its own content, and sources it has an adapter for.

Further rules:

- Previews are cached per URL with the same retention as the metadata they show, and a preview of a work that has become private is invalidated rather than served.
- A preview never renders the body text of a work, a held comment, a private note, a private-library entry, a reading position or any personal statistic.
- A preview of a remote Lorehaven instance's work uses that instance's own published card and metadata. One instance never scrapes another's pages to build one (§23.6).
- A preview never appears for content whose eligibility the viewer fails, and the rendered markup is identical between "does not exist" and "you may not see it".
- **An author may disable unfurling of their works**, after which the link renders as a plain link everywhere on the instance. The setting is recorded, so a later reader can see why a preview is absent.
- A shared preview is not an endorsement and is never counted as one: it feeds no ranking signal and no credit event.

## 31.4 API

```text
GET  /api/v1/works/:id/card.png
GET  /api/v1/works/:id/card.svg
GET  /works/:id/card
POST /api/v1/unfurl
GET  /api/v1/admin/sharing
PATCH /api/v1/admin/sharing
```

## 31.5 Acceptance

- A public eligible work's URL renders a card image and the Open Graph and Twitter/X metadata that point at it (§23.9).
- A draft, unlisted, restricted or withdrawn work renders no card, and its response is indistinguishable from a non-existent work's.
- A card omits the summary when the author has not allowed it, and omits a hidden pseud.
- No card or preview carries a reading count, a reader identity or any per-viewer variation.
- A link to a work on this instance renders a preview with no outbound request, verified by network assertion.
- A URL pointing at an unknown host makes no outbound request when a post containing it is rendered.
- A URL on a known source renders from cache, and the fetch that fills that cache runs through the queue under the source's pacing rules.
- A viewer without access sees a preview that reveals no metadata, and the markup matches the non-existent case.
- A preview of a work that has become private is invalidated and no longer served from cache.
- An author who disables unfurling sees plain links in the forum, in the preview and in the federated representation.
- A shared preview changes no ranking signal and awards no credits.
- A remote instance's work is previewed from its own published card, with no scrape of its pages.

---

# 32. Generalized Media Platform: Creators, Distributors, Collections, and the Media Query API

Lorehaven generalizes from "a fanfiction platform that also hosts media" to a
platform for written and recorded media of any kind — fiction, poetry, essays,
books, translations, audio, video, comics, scans, and preserved archive items —
such that it can stand in for AO3, FanFiction.net, Wattpad, StoryGraph,
Goodreads, the Internet Archive, Literotica and their peers. Every
differentiator in §0 is a default, never a casualty: positivity-first
feedback, trust-level governance, reader content controls, privacy,
quality-first curation, self-hosting. Design rationale lives in the review
note (`secondbrain: 10-projects-lorehaven-platform-redesign-spec.md`) and
ADR 0019; this section is normative.

"Drop-in replacement" means three guarantees, never protocol emulation of
another site's private API:

1. **Data in.** Content migrates from peer sites through adapters (§11.1)
   and site APIs where they exist.
2. **Feature parity.** Every user-facing behavior a person relies on at
   those sites exists here in generalized form.
3. **Data out.** Everything the UI can show, the API can return: filtered,
   paginated, subscribed, and exportable. Scraping this instance is
   impossible by construction; scraping exists only in the ingestion
   direction, toward other sites.

## 32.1 Milestone 22 — media entity model

**Implement.** Migration 0024 (both dialects, per ADR 0004): `creators`
(local pseuds and external records; an external creator is never merged
into a local account by name or handle, §11.11; verification is quorum,
§19), `media_creators` attribution edges, `distributors` and
`distributorships` (who made it available, and how: published, hosted,
mirrored, preserved, narrated, translated, reprinted), `media_collections`
and `media_collection_items` (one typed-membership model for series,
anthologies, reading lists, archive collections, challenge anthologies,
preserved batches — M13 event collections keep their own tables),
`media_editions` (publication history; revisions stay the editing history),
`media_rights` (license, rights statement, lending class — `ai_training`
stays on works, §24.14), `quality_signals` (typed, sourced, recomputable,
0..=1000), and `works.format` defaulting to `prose` (§30.1's taxonomy,
widened: prose, poetry, essay, article, book, fanwork, translation, podfic,
audiobook, fan_film, video, fan_comic, comic, zine_scan, image,
interactive, dataset, other).

Domain rules in `lorehaven-domain::media`: the vocabularies above with
`FromStr` round trips, creator-record consistency, quality-signal bounds.

**Acceptance.**
- 0024 applies on SQLite and PostgreSQL and the parity check passes.
- A creator record that is neither properly local nor properly external is
  refused at the edge.
- Pre-M22 works read as `format = prose` everywhere format is displayed.

## 32.2 Milestone 23 — the media query engine and API

**Implement.** One query engine behind seven doors, all returning the same
`MediaRecord` shape (id, format, title, creators, distributorships,
collections, canon, rating, warnings, dates, quality, language, status,
length/runtime, transcript availability, rights, files, canonical URL):

```text
GET  /api/v1/media                      GET  /api/v1/media/{id}
GET  /api/v1/media/{id}/files           GET  /api/v1/media/{id}/editions
GET  /api/v1/creators[/{id}][/media]    GET  /api/v1/distributors[/{id}][/media]
GET  /api/v1/media-collections[/{id}][/media]
GET  /api/v1/canons/{id}/media          GET  /api/v1/spaces/{id}/media
POST /api/v1/media/query                (complex queries as JSON)
```

Filter dimensions: quality (`min_quality`, individual signals,
`min_creator_trust`), dates (published/updated/ingested), format and media
type, rating, warnings and content notes (§15.16), completion status,
language, length or runtime, canon(s) and crossover, namespaced tags, mood
(§15), license, transcript availability (§30.8), preservation status; and
for authenticated callers, library-local filters (§14). Cursor-paginated,
stable-sorted, ETag/304 per query. Every query renders as Atom/RSS (§23.5)
and as an OPDS acquisition feed; every query can be watched by a webhook
(§21) and exported as a grant-gated bundle (§13, fair-queued §20). Public
media pages embed JSON-LD (`CreativeWork` family); `?format=dc` returns
Dublin Core.

**The ranking philosophy is filters, not scores.** A composite quality
score is instance configuration (weights documented, never purchasable,
§0.3); it powers filters and sorts but is not displayed as a public
leaderboard unless the operator chooses.

**Acceptance.**
- "All media by creator X", "all media in collection Y", and "all media in
  canon Z" are single authenticated-or-public calls, filtered by quality
  and date, cursor-stable.
- A query's ETag answers 304 when nothing eligible changed.
- A query renders as OPDS and downloads into an e-reader client without a
  bespoke client implementation.
- No response leaks media the caller is not eligible for (§7.6 on every
  door, including collection and canon scopes).
- Bulk export of a large query completes through the job queue, is rate
  limited and fair-queued, and never bypasses download grants.

## 32.3 Milestone 24 — site parity features

**Implement.** Anchored comments on any unit position — paragraph offsets
for text, timestamps for media (§12, §30.4); orphaning (a creator detaches
and the media persists under its provenance, pairing with succession
§24.15); a creator dashboard of aggregate, privacy-preserving, positivity-
framed statistics (§24.3 rules; no public shaming numbers); half-star
rating granularity as a reader setting (§9); shelf import from
Goodreads/StoryGraph CSV and works import from the Wattpad and AO3 APIs as
adapters (§11.1, ingestion only); bulk manuscript import (doc/epub/txt)
into the editor (§8); per-format reading goals (words, minutes, items).

**Acceptance.**
- An orphaned work keeps its comments, stats and editions and loses no
  eligibility; the orphaning is reversible only by quorum.
- A StoryGraph CSV import produces library states and reviews that respect
  the reader's existing ratings and dates, and refuses rows it cannot map,
  naming them.
- Anchored comments on a media unit address a timestamp, and on a text
  unit a paragraph, and both flow through the positivity filter (§12).

## 32.4 Milestone 25 — archive mode

**Implement.** Derivative pipeline over `content_blobs` (EPUB/PDF/text
renditions, OCR for scans, transcode for uploaded media) as jobs (§10.4);
full-text search joining transcripts and OCR text (§15, §30.8);
public-domain collections driven by the rights field; optional controlled
digital lending — **operator decision, default off** — with copy caps, loan
expiry, and revocation; IA-style item metadata export (Dublin Core,
§32.2); vanished-source marking (§30.10) extended to derivatives.

**Acceptance.**
- An instance with lending off serves rights metadata and refuses loan
  requests naming the policy.
- A loan grants one reader a bounded window, expires, revocates, and never
  multiplies copies beyond the configured cap.
- A scanned item with OCR text is full-text searchable and the OCR is
  labeled as machine-produced (§22.6).
- Every derivative records its parent blob checksum and re-verifies on a
  schedule.

## 32.5 Milestone 26 — adult content and audio parity

**Implement.** Adult category taxonomy as ordinary canon/tag data behind
the existing age and content-eligibility gates (§7.3, §7.6) — never
leaking into unauthenticated surfaces or feeds; author-approved TTS
narration as an edition (`narration`), machine audio labeled per §22.6/§30.8;
timestamp-anchored comments already covered by M24; gallery mechanics
(§21) applied to illustrated works.

**Acceptance.**
- An anonymous, underage, or opted-out reader meets zero adult items in
  any door, feed, OPDS catalog or search result.
- A narration edition is a first-class edition: it appears in editions
  lists, downloads as audio, and credits its narrator via
  `media_creators`.
- A TTS-generated narration is labeled machine-produced and is replaceable
  by an author recording without destroying the machine edition.

## 32.6 What this section deliberately does not do

- It does not emulate other sites' API shapes; interop is via OPDS,
  Dublin Core, JSON-LD, Atom and ActivityPub announcements (§23.6).
- It does not merge M13 event collections into `media_collections`; the
  event model stays, and a challenge anthology links across via
  `media_collections.challenge_anthology`.
- It does not display composite quality scores publicly by default, and no
  credit, payment or trust level can move any ranking signal (§0.3).
- It does not federate queries (§23.6 stays announce/notify); remote
  queries would be a scraping vector wearing a protocol.

# 33. Consent, Integrity, and Transparency (draft v1, spec-only)

**Nothing in this section is implemented.** Milestones 27–29 were promoted on 2026-09-19 from a cross-project review (the vault notes `[[gravity-lorehaven-feature-crosswalk]]` and `[[lorehaven-jev-system-one-ideas]]`) after checking each item against §0–§32 rather than assuming it absent: the community roadmap and best-worst ballots already exist (§28.12, §29), canonicalization infrastructure already exists (§15.11), and a pluggable classifier with confidence scores is already anticipated (§12.2). What follows is what survived that check.

## 33.1 Milestone 27 — Permission statements, derivative lineage, and the exclusion registry

**Implement.** A **permission statement** on every work and every creator, covering podfic, translation, remix/fork, continuation, redistribution and AI training, each `yes | ask | no | unstated` and defaulting to `unstated`; editable only by the owner, and surviving orphaning and account deletion. A **derivative lineage** edge written whenever one work derives from another, carrying a relationship kind (`translation | podfic | remix | continuation | inspired_by`) and provenance; an imported work whose source states a parent carries the edge, and a parent lists its children. An **exclusion registry** naming external creators and works that must not be imported, narrated, translated, remixed or announced on this instance. Enforcement sits at every door that can create a derivative — the M25 derivative pipeline, TTS narration (M26), the M6 import adapters, the remix path, and ActivityPub announcements (§23.6) — enforced by the server and named in the refusal, never by interface alone.

**Acceptance.**
- A work stating `podfic: no` refuses a narration-edition request and names the statement; `ask` routes the request to the author rather than refusing it in the author's name.
- A remix records its parent and the parent lists the child; the edge survives orphaning (§32.3), and deleting a parent never deletes a child.
- An imported work whose source publishes a statement carries it with provenance; the instance never upgrades `unstated` to `yes`, and a source's `no` is honoured before the bytes are stored.
- A creator or work on the exclusion registry is refused at import by name.

**Deliberately out of scope:** the statement is not a licence (rights remain §32's), not a moderation tool, and not retroactive — it gates the creation of new derivatives, never the existence of existing ones.

## 33.2 Milestone 28 — Rating signal integrity

**Implement.** Trust-weighted aggregation for the ratings and reactions that feed quality signals — a fresh account's first ratings weigh less, and weights come from trust alone, never from credits, subscriptions, bounties or marketplace revenue (§0.3). Anomaly detection over the ratings stream: burst detection per work and per reviewer cohort, and per-account rating-profile outliers, feeding the existing §24.5 anomaly alerts and the §19.3 report queue rather than a parallel pipeline. Brigade early warning that marks a work's aggregate **contested** — visible to curators, never published as a public score — and pauses that work's promotion into recommendation surfaces until a quorum clears it. The honesty rules: a reader always sees their own rating unchanged, an aggregate display states when a cohort guard is suppressing a number (§24.3's k-anonymity pattern), and no individual rating is ever publicly attributed to its reader.

**Acceptance.**
- A rating burst from a fresh cohort raises a contested mark and stops promotion without hiding the work from any reader.
- Trust weighting changes the aggregate while every individual rating stays visible to the reader who made it.
- A contested work clears through quorum and resumes promotion; the clearance appears in the public log (§19.12).
- A test asserts that credits, subscriptions, bounties and marketplace revenue can change no rating weight.

**Extends:** §9.5 ratings, §9.7.8 anti-gaming (the credits half exists; this is the ratings half), §19.13, §24.5.

## 33.3 Milestone 29 — Recommendation transparency and curation labour

**Implement.** (a) **"Why am I seeing this"** on every recommendation slot in the M11 discovery surfaces and recipe dashboards: the reader-side reasons that produced the slot — taste signals, filters, the recipe stage, and the comparison that seeded it (§29.2's arena language where a preference produced it). The explanation must never reveal the administrator's taste multiplier or its breakdown (§0.3, §24.3); operator influence appears as one undifferentiated "instance curation" line. (b) A **private attention report**, off by default: what the reader read, what their own filters changed, what their own settings held back, and what instance curation did — aggregate only. (c) **Curation labour**: a tag-wrangling queue that turns canonicalization (§15.11) into visible trust-gated work — proposals (alias, merge, namespace move, canonical rename) enter a queue, trust levels gate who may propose and who may approve (§19.1), merges keep their history and stay reversible, and wrangling earns the same credits and reputation as any other curation, with the §0.3 refusal that no credit buys wrangling authority and no wrangling authority buys ranking.

**Acceptance.**
- Every recommended slot can name its reader-side reasons, and a test asserts no explanation path can surface the admin multiplier.
- The attention report is private to its reader, includes at least one "held back by your own settings" line, and stays disabled until the reader enables it.
- A tag merge requires the configured trust level, records its approver, is reversible, and appears in the public log (§19.12).
- No wrangling proposal or vote is visible in another user's surface.

# 34. Decision Services (calibrated classifiers)

Cross-cutting contract rather than a milestone: it changes how §12.2, §22.8, §11.14, §11.10, M21's saved-search alerts and §24.14 are implemented, and it exists so those features stop each inventing their own threshold and their own silent failure mode. §12.2 already anticipates "an optional AI classifier returning a confidence score"; this section states the obligations that confidence carries.

**34.1 The contract.** A decision service answers a declared question with a member of a declared answer set plus a calibrated probability — `decide(task, state) → { label, confidence }` — and never returns prose. Structured output is a schema guarantee rather than a hope: an answer outside the set is not representable, which is what makes a decision safe to place deep in a chain where a hallucination is a page failure rather than a bad paragraph. The answer set is declared per task, so a question with more than a few hundred options is narrowed by retrieval first (the §15 index), then decided.

**34.2 Providers are pluggable and optional.** §12.2's optional classifier generalises to a `DecisionProvider` configured per task: a hosted service, a local model (the Ollama path §12.2 already names), or none. With none configured, every task falls back to its deterministic path — §12.2's rules and heuristics with the wider ambiguous band — and the fallback is disclosed on `/api/v1/meta` beside the active content policy. A hosted-only task is refused by design: self-hosting (§2.4) means no task may require a third party for the instance to keep working.

**34.3 Thresholds, tail owners, and what is never decided.** Each task declares a confidence threshold and names the human path that owns the uncertain tail (§12.5 quorum, the §19.3 report queue, the curator queue), and the instance counts escalations — a classifier routing half a comment stream to moderators has automated nothing. **Never decided here:** sanctions (§19.6), shadowban (§19.7), DMCA notices (§19.11), money (§20.9), trust levels (§19.1), and the AI-training assertion as a verdict (M21 keeps it a signal). These are human by construction, and the decision surface cannot express them.

**34.4 Declared tasks (v1).**
- `positivity_class` — the §12.1 classes with calibrated confidence, so §12.3's delivery rules become threshold rules (high-confidence appreciation delivers; critique delivers only where the author opted in; the ambiguous band routes to §12.5 quorum exactly as §12.2 already states). This is the flagship: the pipeline is already specified; this is the confidence that lets delivery be trusted to it.
- `translation_quality` — p(adequate) gating auto-published machine translation, which turns §22.5's "AI never overrides human" into a threshold with human-version precedence.
- `import_outcome` — per-item import classification (§11.14) in the M5 worker, with the low-confidence tail to the batch review UI.
- `identity_match` — p(same creator) over retrieved candidates for §11.10 cross-source identity; above threshold it *suggests*, and never merges (the no-force-merge rule stands).
- `alert_match` — p(matches saved search) for M21's alerts, evaluated per new work across that reader's saved searches; a per-user threshold decides whether an alert is sent, which is what makes the alert queue affordable.
- `mood_class` — §15.8 mood/tone labels for new units and as a corpus backfill: one worker pass that turns a planned search feature on.
- `language_id` — per-unit language for transcripts and media (§30.8): a bounded choice among languages, which is exactly this shape.
- `crawler_signal` — behavioural p(crawler) at the edge for §24.14, as a second line behind the deterministic rules the posture already states.

**34.5 Batch passes produce signals, never verdicts.** The same contract run by the worker over the corpus (backfills, reindex passes). A backfilled label is metadata: it never retro-hides content, never changes an eligibility verdict on its own, and a re-run may change it.

**34.6 Audit and disclosure.** Every automated decision that changes what a reader sees is recorded — task, label, confidence, model identity and version, timestamp — operator-only, hashes and labels rather than copied text, consistent with §24.3's privacy rules and §0.3's ban on hidden influence. Where an explanation surface exists (M29's "why am I seeing this"), it may cite a decision's task and label; it may never expose the operator's weights.

**Acceptance.**
- With no provider configured, §12's classification runs on the deterministic path, the wider ambiguous band reaches quorum, and `/api/v1/meta` says so.
- No task can be invoked for a sanction, shadowban, DMCA case, payout, trust level or AI-training verdict; a test asserts the surface cannot express them.
- A comment's class and confidence are recorded, and a published delivery decision can name the class that produced it.
- Every task defaults to disabled, and an instance with all tasks disabled behaves exactly as §12.2's rules path describes.

---


---

# 35. Forum as a First-Class Surface (Milestones 31–35, repo)

The forum is not a feature bolted onto a fiction site; it is a first-class
surface of the creative ecosystem. Two design decisions drive everything in
this section:

1. **One conversation, one place.** Quick reactions and real discussion are
   different behaviors pretending to be one. Reactions are signal, not
   conversation; text discussion belongs in exactly one place — a linked
   forum thread with full infrastructure (threading, search, moderation,
   typed votes, follow/notify, federation).
2. **Typed votes with meta-moderation** (Slashdot-style) replace the generic
   like/reaction model on community surfaces, and become the positivity
   infrastructure for the work page: `well-written` and `insightful` are
   inherently positive signals; abuse is handled by meta-moderation, not by a
   text classifier on a one-click surface.

Everything here is subject to §0.3. Votes and karma never purchase trust,
moderation authority, or search ranking.

## 35.0 Work discussion modes

Each work carries a discussion mode. The instance sets a default; each author
may override per work; moderators may not override authors.

```rust
enum WorkDiscussionMode {
    ThreadOnly,    // default: typed-vote reaction bar on the work page,
                   // all text discussion in the linked forum thread
    CommentsOnly,  // inline work/chapter comments, no forum link (legacy)
    Both,          // both surfaces, admin-warned: splits conversation
}
```

- **`ThreadOnly`** — the work page shows the typed-vote reaction bar and a
  "Discuss" link to the linked thread (§35.1). No text input on the work
  page. The author follows one thread, gets one notification stream, and
  replies with full formatting and context.
- **`CommentsOnly`** — preserves the §17.1 comment surface and its positivity
  gate unchanged, for admins who prefer the classic model and do not care
  about federation of discussion.
- **`Both`** — offered as the escape hatch; the admin UI states plainly that
  it splits conversation across two surfaces.

Migration path for instances with existing comments: ship threads + reaction
bar first with `CommentsOnly` unchanged for existing works; per-work opt-in
to `ThreadOnly`; flip the instance default when the forum is mature; provide
a batch tool that converts existing comment threads into forum topics
(preserving authorship, body, and timestamps) for authors who switch.
`CommentsOnly` remains supported indefinitely for federation compatibility.

When a work federates, its discussion mode travels with the metadata so a
remote instance knows how to render the discussion surface. A `ThreadOnly`
work arriving at a `CommentsOnly` instance may show a link back to the origin
thread; local mirroring of remote forum posts into a comment thread is
best-effort and never authoritative.

The positivity pipeline (§12) still applies to every surface where readers
write text addressed to the author: work comments in `CommentsOnly`/`Both`
modes, and forum posts under the lighter forum policy (§17.2). In
`ThreadOnly` mode there is no work-page text to classify; the typed-vote
taxonomy is itself the positive-signal infrastructure, and the negative vote
types are costed and meta-moderated (§35.2).

## 35.1 Milestone 31 — Work-linked threads and the reaction bar

**Work-linked threads.** Publishing a new chapter auto-creates (or prompts
the author to create) a discussion topic linked to that work/chapter. The
work page shows a "Discuss" button with the reply count; the topic shows a
backlink card to the work. One linked topic per work by default (chapter
sections inside it), optionally one per chapter for high-traffic works.

Schema: `topic_work_links (topic_id, work_id, chapter_id NULL, unique(topic_id))`.
The link is visible wherever the topic is, including federation surfaces.

**Reaction bar.** On the work page (all modes), a typed-vote bar:

```text
[well-written: 42] [insightful: 17] [funny: 8] [disagree: 3]
                    [Discuss this chapter (47 replies) →]
```

- One vote per pseud per work, changeable, retractable.
- Vote types are the instance's configured set (§35.2), restricted to the
  positive types plus `disagree` on this surface.
- Aggregate counts are public; individual votes follow §35.2 transparency
  tiers.
- The author sees aggregated sentiment at a glance — the point of the bar is
  that the author stops wading through "great chapter!" to find substance.

**Batch migration tool.** Converts a work's comment thread into a forum
topic: comments become posts in order, authorship and timestamps preserved,
original comment soft-deleted with a tombstone pointing at the topic.
Reversible only by re-running the reverse conversion before the tombstones
are pruned.

**API.** `GET/PUT /works/{id}/discussion-mode` (author), `GET /works/{id}/reactions`
(public aggregates), `POST /works/{id}/reactions` (vote, change, retract),
`GET /works/{id}/thread` (linked topic), `POST /works/{id}/migrate-comments`
(author, batch tool). Admin config: `[forum] work_discussion_default`.

**Data model.** `topic_work_links`, `work_reactions (work_id, pseud, vote_type,
created_at, updated_at, unique(work_id, pseud))`, `works.discussion_mode`.

**Acceptance.**
- A `ThreadOnly` work page renders no comment form and shows the bar and
  Discuss link; the link resolves to the linked topic.
- Publishing a chapter creates or prompts the linked topic exactly once
  (idempotent per chapter).
- Votes are one-per-pseud, changeable, and the aggregate counts are correct
  after concurrent votes.
- The migration tool round-trips a comment thread into a topic with
  authorship and timestamps intact.
- A `CommentsOnly` work behaves exactly as §17.1 specifies today.

## 35.2 Milestone 32 — Typed votes, budgets, meta-moderation, karma

**Typed vote taxonomy.** A small fixed set per surface, configurable per
category: default `insightful | funny | interesting | well-written |
disagree`. A Critique category may configure `constructive | harsh-but-fair |
needs-sources` instead. Negative types (default: `disagree`) cost the caster
more budget. The taxonomy is data, not code.

**Vote budget.** N votes per rolling 24h window scaled by trust level
(TL1=10, TL3=30, TL5=60; configuration). Unused votes do not roll over.
Forces signal over noise; a budget-exhausted voter is told plainly.

**Meta-moderation.** TL4+ users spend meta-mod points to flag a vote as
`fair` or `unfair`. A user whose votes are consistently flagged unfair has
their **vote weight** decayed (never their ability to vote). Moderate the
moderators: weight, not voice, is the sanction.

**Transparency tiers.** Aggregate counts public; individual votes anonymous
by default; the post author may opt in to see who voted; moderators always
see; meta-mods see meta-mod actions. Consistent with §29.6: a visible vote
is a vote that can be socially pressured.

**Karma.** Forum karma is derived from received typed votes, weighted by the
caster's vote weight. Karma decays 5% per month of the receiver's inactivity
(configuration). Karma is a display signal only — it never gates trust,
moderation, search ranking, or credits (§0.3). Posting-volume leaderboards
remain not implemented (§17.10 stands); karma leaderboards are equally out.

**This replaces** the generic reaction model on forum surfaces; the existing
`reactions` table is retained for works/library surfaces that already use it.

**API.** `POST /forum/posts/{id}/vote`, `DELETE /forum/posts/{id}/vote`,
`GET /forum/posts/{id}/votes` (per transparency tier), `POST /forum/votes/{id}/meta`
(TL4+), `GET /me/vote-budget`, `GET /forum/karma` (own + public profiles).

**Data model.** `forum_vote_types (id, label, category_scope, weight, cost,
is_negative)`, `forum_votes (post_id, pseud, vote_type, weight_at_cast,
created_at, unique(post_id, pseud))`, `forum_meta_votes (vote_id, pseud,
fair, created_at, unique(vote_id, pseud))`, `forum_karma (pseud, karma,
updated_at)`.

**Acceptance.**
- Budget is enforced per rolling window and per trust level; exhaustion is
  reported, not silently dropped.
- Meta-moderation changes a caster's future vote weight, never their ability
  to cast.
- Individual votes are not exposed where the transparency tier forbids it.
- Karma decays on inactivity and never feeds trust, ranking, or credits.
- Configuring a category's taxonomy changes its surfaces without code changes.

## 35.3 Milestone 33 — Thread modes for creative work

Topic modes are data on the topic; each restructures one surface.

- **AMA / Q&A (`ama`).** Questions float to the top; the topic author's
  replies render as highlighted cards; non-author replies to questions nest
  as discussion.
- **Reading group (`reading_group`).** A schedule of sections ("Week 1:
  chapters 1–5") each unlocking on a date; participants see a progress
  tracker. Builds on §17.11 reading clubs, which keep their group machinery.
- **Critique circle (`critique`).** Private group thread mode: members post
  excerpts on a turn queue; turn order enforced; pile-on limited; quality
  gating ties into trust levels. Critique here is opt-in by joining, so the
  positivity gate's constructive-criticism opt-in (§12) is satisfied by
  membership.
- **Wiki pin (`wiki_pin`).** A collaboratively edited post pinned above the
  OP; edits pass a lightweight approval queue held by the topic author or
  group moderators. Turns long lore threads into living documents.
- **Collaborative fiction (`collab_fic`).** Posts stitch into a single
  narrative; a "compile" action renders the thread as one readable story;
  version history preserved. A thread with traction offers one-click
  **promote to work**: creates a proper work with the thread content as
  chapters, links back, and leaves the thread read-only. This closes the
  loop between community discussion and published content.
- **Prompt (`prompt`).** Posted automatically by the prompt engine
  (daily/weekly writing prompts to a dedicated category); replies are flash
  fiction; community typed-votes decide favorites; winners get badges (§28.11
  governs gamification limits).
- **Character voice (`character_voice`).** Posts render as a character from
  the poster's work (avatar and name), tagged with the character id.
  Moderation applies to the user, never the character.

**Data model.** `forum_topics.mode`, `topic_schedules (topic_id, position,
title, unlocks_at, chapter_range)`, `topic_wiki_pins (topic_id, post_id,
revision, approved_by)`, `critique_queue (topic_id, pseud, position,
posted_at)`, `prompt_posts (topic_id, prompt_date, winner_pseud)`.

**Acceptance.**
- Each mode changes exactly its own surface; a plain topic is unaffected.
- Reading-group sections are invisible and unfetchable before their unlock
  time.
- Critique turn order is enforced by the server, not the client.
- Wiki-pin edits are invisible until approved.
- Promote-to-work creates a real work owned by the thread participants under
  the author's stated permission statement (§33.1), and the thread becomes
  read-only with a backlink.
- Character-voice posts resolve to the user for moderation and blocks even
  though they render as the character.

## 35.4 Milestone 34 — Spoilers, warnings, readability

- **Spoiler-aware zones.** Per-topic spoiler scope ("spoilers through chapter
  12"); collapsible spoiler blocks (blur + click-to-reveal) in posts; a
  "catch me up" badge for readers behind the scope. Reader-side spoiler
  hiding honours the reader's own progress (§9.3), not just the topic scope.
- **Structured content warnings.** `content_warnings (post_id, warning_type,
  severity, custom_text)` with types `violence | sexual_content | self_harm |
  spoilers | custom`. Warned posts blur by default with one-click reveal;
  user prefs control auto-hide vs show per type. Shares the instance
  vocabulary machinery with §15.16.
- **Reading time estimates** from word count on topic cards and post headers.
- **Draft autosave.** Post drafts persist every 30s while composing
  (`post_drafts`, §4.6) and offer resume-on-return. Survives browser crashes.
- **Post scheduling.** `post.scheduled_at`; a background job publishes due
  posts. Useful for reading groups and serialized announcements.
- **Collapsible long posts.** Posts over ~800 words (configuration) fold
  behind "Read more"; user-configurable threshold and off switch.

**Acceptance.**
- Spoiler blocks do not leak content to screen readers when collapsed
  (`aria-expanded`, content hidden from the a11y tree).
- Content-warning blur state follows the viewer's prefs, not the poster's.
- Drafts survive a hard reload mid-composition and resume exactly once.
- Scheduled posts publish at (not after) their time, once, idempotently.
- The fold never hides the first screenful of a post.

## 35.5 Milestone 35 — Discovery, navigation, community health, federation

**Discovery.**
- **Thread summaries.** For topics over N replies, a collapsible TL;DR at
  the top, regenerated every M replies. Local LLM or opt-in external API
  under §23.7's consent and cost rules; deterministic abstention when no
  provider. Stored as `topic.summary_text` + `summary_revision`; never
  presented as author text.
- **Semantic search.** Embeddings of topic title + first post in a
  `pgvector` column (PostgreSQL deployments; SQLite deployments keep FTS
  only and say so on `/api/v1/meta`). Powers "similar threads" and
  "search by vibe". Best-effort; failure never loses the FTS path.
- **Thread forking.** A moderator (or TL4+) selects a contiguous post range
  and forks it into a new topic; originals get "moved to [link]" tombstones;
  `post.original_topic_id` preserved for audit.
- **Cross-reference cards.** Internal links to topics, works, and profiles
  auto-expand into preview cards (title, snippet, reply count, last
  activity), parsed on save and cached. Builds on §31's unfurling.
- **Best-of digests.** TL5+ or elected curators flag posts `featured`; a
  `/forum/digest` view shows them with optional editorial commentary;
  optional email newsletter under §17.5's digest rules.
- **Topic clusters.** Related topics grouped by tag overlap + semantic
  similarity, shown as "Related discussions".

**Community health.**
- **Graduated response ladder.** `verbal_warning → post_throttle (1/hour) →
  read_only (24h/72h/1w) → forum_ban → site_ban`. Each step logs to the
  modlog (§19.12) and notifies the user with the specific reason and appeal
  path. Extends §17.10 scoped sanctions with a named ladder.
- **Appeals.** Structured appeal form; appeals route to a different
  moderator than the actor (enforced by the system); outcomes tracked for
  mod accountability (§19.10).
- **Community-elected moderators.** Category-scoped elections
  (`mod_election`), winners get mod privileges scoped to that category,
  appointed through the same quorum and accountability as §17.12 fandom
  moderators.
- **Diversity of voices.** Gini coefficient of post distribution per
  category; a "dominated discussion" flag surfaces to moderators; suggested
  interventions (new-voices-only threads, slow mode).
- **Newcomer onboarding.** TL0/TL1 users get an invitation to a Welcome
  category with guided prompts.
- **Slow mode.** Per-topic or per-category rate limit: one post per user per
  N minutes (`topic.slow_mode_seconds`).

**UX.**
- **Keyboard shortcuts.** `j/k` next/prev post, `r` reply, `e` edit, `q`
  quote, `/` search; documented in a `?` modal.
- **Reading progress bar** for long topics; scroll position persists across
  loads (extends §17.3 read state).
- **Multi-language posts.** `post.language` (auto-detected or user-set);
  filter topics by language; opt-in auto-translation under §23.7.
- **Offline reading.** Service-worker cache of visited topics with a cached
  indicator; read-state syncs on reconnect (extends §13's offline rules).

**Analytics.**
- **Community health dashboard** (admin): daily active posters, new-user
  retention 7d/30d, average thread depth, time-to-first-reply, report
  volume, mod response time. Privacy-preserving per §24.3.
- **Zero-result search tracking**: log queries that return nothing; surface
  the top ones to admins weekly.
- **Activity sparklines** on topic cards (`topic.activity_history`, daily
  reply counts).
- **First-votes notification**: after a post's first N votes, one gentle
  notification to the author — once per post, never per vote.

**Federation.**
- **Per-topic federation scope**: `public | local | unlisted`
  (`topic.federation_scope`); authors choose at creation, moderators may
  override; `local` topics never leave the instance.
- **Remote profile cards** with home instance, local bio, follow button;
  cached with TTL; `Person` and `Service` actor types.
- **Federated moderation**: local ban of a remote user sends `Reject` to the
  home instance; remote reports aggregate in the mod queue with
  instance-of-origin metadata.
- **Instance reputation**: per-instance spam rate, report rate, takedown
  response time; auto-defederation below a threshold; instance health on the
  admin dashboard. Extends §23.10's instance trust relationships.
- **Federated polls**: ActivityPub `Question` objects; remote votes tallied
  locally, best-effort one-vote-per-actor across instances — the limitation
  is documented, not hidden.

**Acceptance.**
- A `local`-scoped topic is absent from every federation outbound payload.
- Forking preserves original authorship, timestamps, and an audit trail.
- The health dashboard aggregates only what §24.3 permits.
- Sparklines and reading-time come from stored data, not client estimation.
- Federated poll tallies never count one actor twice locally.
- Keyboard shortcuts are documented, discoverable, and never trap focus.

## 35.6 What this section deliberately does not do

- No general-purpose karma economy: karma decays, displays, and does nothing
  else (§0.3).
- No AI-required feature: summaries, semantic search, and translation are
  optional providers with deterministic abstention (§23.7).
- No second forum implementation: fandom spaces (§17.12), reading clubs
  (§17.11), and every mode above build on the same topics/posts machinery.
- No vote on trust: nothing in §35.2 may promote, demote, or gate a trust
  level (§19.1 owns that ladder).

---

The resulting project should be judged by these working behaviors—not by the number of screens, lines of code, imported feature names, or claims in a README.


---

# 36. Growth, Sharing & Ecosystem Expansion

> **2026-09-20 addition.** Lorehaven's growth comes from sharing loops that
> turn readers into promoters, from removing every barrier to entry, and
> from community features that sustain themselves. This section makes those
> capabilities first-class.

## 36.0 Scope

Every capability here builds on existing infrastructure: the public API
(§23.1), the adapter abstraction (§11.1), the forum machinery (§17), the
media model (§30), and the recommendation engines (§16). Nothing here
reimplements what exists.

The order is implementation order, not priority order. An instance may
choose a subset; the acceptance list makes each item independently
verifiable.

## 36.1 Shareable Quote Cards

A reader selects a passage of 20–500 characters on any work page and the
instance renders it as a shareable image card. The card carries the quote,
the work title, the author pseud, and a short link or QR code back to the
work. Authors may generate them too; readers generate them more often.

**Rendering.** The instance generates the card server-side with no client-
side canvas, no DOM capture, no headless browser. The author may choose a
style; the instance offers at least three preset themes and a fandom-
themed template where one exists for the work's primary fandom. Animated
variants (subtle motion, paper texture, flickering candle) are offered for
platforms that support video or GIF.

**Watermark.** The card carries the work's short URL and the Lorehaven word-
mark, small and in a corner — present, not obstructed. A reader sharing a
quote card is promoting the work; the watermark serves the promotion, not
the brand.

**Sharing.** The share sheet offers one-click posting to Twitter/X, Tumblr,
Bluesky, Mastodon, and Discord via their share URLs or intents. The image
is also available for download, so a reader may share it anywhere else
without the instance needing to enumerate every platform.

**API.** `POST /api/v1/works/{id}/quote-card` accepts `{ "start": 142,
"end": 287, "theme": "reading-room" }` and returns `{ "image_url": "...",
"short_url": "...", "qr_url": "..." }`. Authenticated or not — a quote
card is promotion, and promotion is not gated.

**Acceptance.**
- A generated card contains the exact selected text, the work title, and
  the author pseud.
- A card generated by a reader who has never logged in is still generated.
- The short URL on the card resolves to the work and deep-links to the
  quoted passage.
- No client-side canvas, DOM capture, or headless browser is used.
- A fandom-themed template, when present, is automatically selected for
  works carrying that fandom.

## 36.2 One-Click Import and Cross-Posting

A reader pastes an AO3, FFN, or Wattpad profile URL and Lorehaven imports
the entire library — works, bookmarks, reading history, series membership,
co-authors, and tags. The import is a background job with a progress bar
and an email notification on completion.

**Import flow.**
1. Reader enters a profile URL or work URL.
2. The instance identifies the source (§11.1 adapter).
3. For a profile URL, the instance discovers the work list and enqueues
   an import job per work.
4. For a work URL, the instance imports that work directly.
5. The reader sees a progress page: works completed, works remaining,
   estimated time.
6. On completion, the reader's library is populated and an email is sent.

**Mirror mode.** The reader may choose "Mirror" during import: Lorehaven
keeps the imported works but does not replace or hide the originals on the
source site. This is the default. The reader is never asked to choose
between Lorehaven and an existing archive.

**Cross-posting.** A work published on Lorehaven may be pushed to AO3,
FFN, or Wattpad if the reader holds credentials for that destination and
has configured the push. Cross-posting syncs metadata (title, summary,
tags, rating, warnings, characters, relationships) and chapters. A push
fails cleanly if the destination rejects a tag mapping; the Lorehaven work
is unchanged and the reader sees which field failed.

**Acceptance.**
- An AO3 profile URL with 50 works imports all 50 within the progress
  window.
- A mirror-mode import never alters the source work.
- A cross-post to AO3 carries the summary, all tags, rating, warnings,
  and at least one chapter.
- A cross-post that fails on the destination reports the failing field
  and leaves the Lorehaven work unchanged.
- A reader who has never visited AO3 may still import by pasting a URL.

## 36.3 FFN and Wattpad Source Adapters

The adapter abstraction (§11.1) is extended to support FanFiction.net and
Wattpad as import sources. The AO3 adapter (§11.7) already exists; these
follow the same contract.

**FFN adapter.** Identifies by `fanfiction.net/s/{id}` or
`fanfiction.net/u/{user}`. Fetches metadata from the work page and chapter
text from the chapter list. FFN's interface is rate-limited and unforgiving;
the adapter respects a crawl delay (configurable, default 3 seconds) and
backes off on HTTP 429. FFN does not expose a structured tag vocabulary;
the adapter maps FFN's freeform tags to the Lorehaven taxonomy (§15.16)
and flags the mapping for curator review when confidence is below a threshold.

**Wattpad adapter.** Identifies by `wattpad.com/story/{id}` or
`wattpad.com/user/{user}`. Wattpad's interface is heavily JavaScript-
driven; the adapter uses the Wattpad API where available and falls back to
page scraping where it is not. Wattpad's commercial pivot has alienated
many writers; the import flow states clearly that Lorehaven does not mirror
Wattpad's paywall and that imported works are freely readable.

**Acceptance.**
- An FFN adapter identifies a work URL, fetches metadata and all chapters,
  and produces a complete import record.
- An FFN adapter that receives HTTP 429 waits the crawl delay and retries.
- A Wattpad adapter produces a complete import record.
- An FFN tag that maps ambiguously is flagged for curator review rather
  than silently miscategorized.

## 36.4 Year in Review / Wrapped

Once per year (and optionally once per month for the first year to build
the habit), the instance generates a personalized stats page for every
active reader or author: words read, words written, fandoms explored,
achievements unlocked, hours spent, favorite author, most-read work,
longest streak, and a comparison to the reader's own prior period.

**Design.** The page is designed to be screenshotted. The layout is a
vertical stack of stat cards, each large enough to read at phone width.
A "Share this" action generates a single composite image suitable for
social media, with the same watermark treatment as quote cards (§36.1).

**Generation.** The review is generated by a background job in the first
week of January and on the reader's anniversary. It is available at
`/me/wrapped/{year}`. A reader who was inactive that year gets a "quiet
year" page, not an error.

**Acceptance.**
- A reader who read at least one work in the prior year has a wrapped
  page with non-zero stats.
- The wrapped page renders cleanly at 375 px width (phone).
- The shareable image includes the reader's pseudonym and the Lorehaven
  word-mark.
- A reader with no activity in the prior year sees a "quiet year" page.

## 36.5 Writing Prompts Engine

Prompts are submitted by the community, voted on by the community, and
surfaced daily and weekly. A prompt may be fandom-specific or general.
Works written in response are linked back to the prompt and visible in a
prompt response feed.

**Submission.** Any authenticated user may submit a prompt. A prompt is a
short text (10–500 characters) and an optional fandom scope. Submission is
rate-limited: three prompts per pseud per rolling day.

**Voting.** Prompts are voted on with the same typed-vote system as forum
posts (§35.2). Voting is open to all authenticated users. The daily prompt
is the highest-voted prompt in the prior 24-hour window that has not been
used in the prior 7 days. The weekly prompt is the highest-voted in the
prior 7-day window not used in the prior 30 days.

**Adoption.** A user may "adopt" a prompt — a public commitment to write
for it. An adopted prompt shows the adopting pseud's name and an optional
accountability timer. Adopting earns credits; completing (publishing a linked
work) earns more and a badge (§9.7.6).

**Response feed.** Every prompt page shows a feed of linked works, ordered
by publication date. A work is linked when the author selects the prompt
at publication time or adds it later via the work editor.

**Acceptance.**
- A prompt submitted by a user appears in the voting feed.
- The daily prompt is the highest-voted unused prompt in the prior 24h.
- Adopting a prompt earns credits; publishing a linked work earns more and a badge.
- A work linked to the prompt appears in the prompt's response feed.
- A user who submits four prompts in 24 hours is rate-limited on the
  fourth.

## 36.6 Community Gift Exchanges

A gift exchange is a structured event where participants sign up with
preferences (fandoms, tropes, DNWs), the instance runs a matching
algorithm, and each participant is assigned a recipient to create a work
for. Exchanges have a signup window, a matching date, a creation window,
and a reveal date.

**Roles.** The exchange has an organizer (who configures the exchange),
moderators (who handle pinch-hits and disputes), and participants.

**Signup.** A participant signs up with:
- Fandoms they will write in.
- Tropes, characters, and relationships they are willing to write.
- Do-not-writes (DNWs) — tropes, characters, or scenarios they will not
  write.
- A minimum word count they commit to.
- An optional "letter" — a free-text description of what they'd love to
  receive, visible only to their assigned creator.

**Matching.** The matching algorithm runs after signup closes and assigns
each participant a recipient. The algorithm respects DNWs as hard
constraints. A participant with a DNW that no other participant can write
around is offered the option to withdraw without penalty. Matching is
deterministic given the participant set — re-running produces the same
assignments, so an organizer can verify before revealing.

**Pinch-hits.** A participant who cannot complete their assignment may
request a pinch-hit. Moderators assign a pinch-hitter — a volunteer who
has not already completed an assignment for that recipient. A pinch-hit
satisfies the recipient's guarantee: every participant receives at least
one work.

**Reveal.** On the reveal date, all works are posted simultaneously. The
creator is revealed to the recipient but remains pseud-anonymous to others
until the creator chooses to reveal. An archive of past exchanges is
public; works remain in the archive after the exchange ends.

**Acceptance.**
- A participant with a DNW is never assigned a recipient whose letter
  requires that DNW.
- Re-running matching produces the same assignments.
- A participant who requests a pinch-hit is assigned a pinch-hitter
  who has not already created for their recipient.
- On the reveal date, all works are posted and visible.
- A past exchange is accessible as an archive.

## 36.7 Beta Reader Marketplace

An author posts a beta request with requirements: fandom knowledge
needed, turnaround time, focus areas (grammar, characterization, canon
consistency, pacing), and word count. Beta readers browse open requests
and claim them. Both parties rate the interaction on completion.

**Request.** A beta request is a short record: the work (or a chapter
range), the focus areas, the turnaround deadline, and any fandom
prerequisites. A request may be open (anyone may claim) or by invitation
only.

**Claiming.** A beta reader claims an open request. The claim is visible
to the author, who may accept or decline. Once accepted, the beta reader
gets access to the work and a shared annotation workspace.

**Annotation.** The annotation workspace supports in-line comments on the
work body, with accept/reject workflow. An annotation is a span of text
plus a comment; the author may accept, reject, or leave each annotation
unresolved. The workspace is private to the author and the beta reader.

**Rating.** On completion (the author marks the beta complete), both
parties rate the interaction: was the feedback delivered on time? Was it
useful? The rating is visible to future authors considering that beta
reader. Ratings are never anonymous — a pseud is always attached to a
rating.

**Recognition.** The beta reader earns credits for completing a beta and the
author earns credits for leaving a rating (§20.3); neither accrues a lifetime
tally (§9.7.1). A beta reader who completes five betas with an average rating
above a threshold earns a "Reliable Beta" badge.

**Acceptance.**
- An author with a work can post a beta request.
- A beta reader can claim an open request.
- The author sees the claim and may accept or decline.
- Annotations on the work body are visible only to the author and the
  assigned beta reader.
- On completion, both parties rate the interaction and the rating is
  pseudonymous.

## 36.8 Recommendation Threads

A reader starts a "Looking for X" thread describing what they want.
Others respond with work links, upvoted by the community. The thread is
a forum topic (§17.2) with a recommendation schema on top.

**Request.** A recommendation request is a free-text description (10–1000
characters) and optional structured filters (fandom, rating, completion
status, word count range, tropes). The request is a topic with a
`recommendation_request` flag.

**Response.** A response is a post that links to a work (internal or
external) and optionally explains why it fits. Responses are voted on
with the same typed-vote system (§35.2). The requestor may mark a response
as "fulfilled" — the request is satisfied and the thread is highlighted
as answered.

**Auto-recommendation.** When a request is posted, the instance runs a
recommendation pass (§16) using the request text as the query and the
structured filters as constraints. The top results are posted as an
auto-generated response, labeled as machine-generated and not attributed
to any pseud. A reader who disagrees may downvote or reply.

**Archive.** Past recommendation threads are searchable. A reader looking
for something may find an existing thread before starting a new one. The
search interface surfaces threads by fandom, trope, and keywords.

**Acceptance.**
- A reader can post a recommendation request.
- A response can link to a work and explain why it fits.
- The requestor can mark a response as fulfilled.
- Auto-generated recommendations are labeled as machine-generated.
- A reader searching for a request finds matching existing threads.

## 36.9 Fan Art Gallery

An artist uploads fan art linked to a specific work, with the author's
permission. The art appears in a gallery on the work page and in a
standalone discovery feed. The artist retains ownership; Lorehaven stores
the image and a reference to the artist's profile.

**Upload.** An artist uploads an image (JPEG, PNG, or WebP, max 10 MB) and
links it to a work. The upload requires the work author's permission: the
artist sends a request, the author approves or declines. Until approved,
the art is visible only to the artist and the author.

**Gallery.** Approved art appears in a gallery on the work page, in a
standalone feed, and in the artist's profile portfolio. The gallery is
paginated and filterable by work, artist, and fandom.

**Cover art.** The author may select any approved fan art as the work's
cover image, with the artist's permission. The cover image carries a
credit line linking back to the artist's profile.

**Discovery.** Fan art is eligible for search, mood search, and the
recommendation engines on the same terms as works (§30.7). A reader who
disables images does not see fan art in feeds.

**Acceptance.**
- An artist can upload art linked to a work.
- The art is visible to no one until the author approves it.
- Approved art appears in the work page gallery and the artist's
  portfolio.
- The author may select approved art as the work's cover, with the
  artist's permission.
- Fan art is searchable.

## 36.10 Interactive Fiction Navigation

A work with format `interactive` (§30.1) has branching chapters. The
reader navigates choices, and the instance tracks which paths a reader
has taken. The author builds the branch structure in the editor.

**Authoring.** The editor offers a branch tool: a chapter may have
multiple outgoing links, each labeled with the choice text and pointing
to the next chapter. The author may create branches, merge branches, and
visualize the structure. A branch is a directed graph, not necessarily a
tree — chapters may have multiple incoming links.

**Reading.** The reader sees the current chapter and a choice menu. Each
choice links to the next chapter. The reader may go back, undo a choice,
or jump to any previously visited chapter. A progress indicator shows the
reader's current position in the graph and how many chapters remain
reachable.

**Tracking.** The instance records which chapters a reader has visited
and which choices they made. A reader may see their path history and
restart from any point. Completion for an interactive work means the
reader has visited every chapter at least once ("all paths" mode) or
reached a designated end chapter.

**All-paths mode.** A reader may switch to "all paths" mode, which
presents chapters in a deterministic order that covers every path. The
reader's choice history is preserved; all-paths mode is a reading aid, not
a new work.

**Acceptance.**
- An author can create a work with format `interactive`.
- The editor offers a branch tool that creates chapters with multiple
  outgoing links.
- A reader sees a choice menu and navigates branches.
- The reader can go back, undo a choice, and revisit chapters.
- Completion is recorded when all chapters are visited or an end chapter
  is reached.
- All-paths mode presents chapters in a deterministic covering order.

## 36.11 Mood Journal and Mood-Based Discovery

A reader may attach a private mood tag to any work they have read: "made
me cry," "cozy," "devastating," "hopeful," "adrenaline-fueled," or a
custom label. The journal is private; no other reader sees a reader's
mood tags. The journal is the reader's own record of how fiction affected
them.

**Mood tags.** The instance offers a preset vocabulary of mood tags and
allows custom labels. A mood tag is a single label per reader per work;
changing the tag replaces it, it does not accumulate. A reader may also
attach a private note (up to 500 characters) to a mood tag.

**Search.** A reader can search their own journal: "show me everything I
tagged as 'made me cry'." The search is private and scoped to the reader's
own pseud.

**Mood-based recommendation.** A reader may ask the instance for
recommendations based on a mood: "I want to feel [cozy]." The instance
runs a recommendation pass (§16) using the mood as the primary signal.
Recommendations are drawn from works the reader has not read and are
labeled with the mood that motivated them. The reader may dismiss a
recommendation; dismissed works are excluded from future mood-based recs
for a cooling-off period.

**Acceptance.**
- A reader can attach a mood tag to a work they have read.
- The journal is private to the reader's pseud.
- A reader can search their own journal by mood.
- A mood-based recommendation returns works the reader has not read.
- Dismissed recommendations are excluded for the cooling-off period.

## 36.12 Author Analytics Dashboard

An author sees a private dashboard of aggregate statistics for each of
their works: views over time, reader drop-off by chapter, referral
sources (anonymized), geographic distribution (country-level, anonymized),
device breakdown, and reading completion rate.

**Privacy.** The dashboard is aggregate only. No individual reader is
identifiable. Geographic data is country-level, not city or region. Device
data is category (desktop, mobile, e-reader), not individual user-agent
strings. A work with fewer than 10 views suppresses all breakdowns except
the total view count, to prevent identification of individual readers.

**Referral.** Referral sources are categorized: Lorehaven search, Lorehaven
recommendation, external link (Discord, Tumblr, Twitter, direct), and
quote card. The author sees counts per category, not individual referring
URLs.

**Drop-off.** The drop-off chart shows the percentage of readers who
started each chapter. A chapter with a sharp drop-off is highlighted.
This is informational; the author decides what to do with it.

**Acceptance.**
- An author sees view counts for each of their works.
- A work with fewer than 10 views suppresses all breakdowns except total
  views.
- Referral sources are categorized, not individually identified.
- The drop-off chart shows per-chapter retention.
- No individual reader is identifiable in any dashboard view.

## 36.13 SEO Optimization

Every public page — work, chapter, author, collection, fandom, forum
topic — emits structured data (JSON-LD) using schema.org vocabulary:
`CreativeWork`, `Person`, `Review`, and `DiscussionForumPosting` as
appropriate. Sitemaps are generated with priority weighting: work pages
higher than tag pages, author pages higher than instance pages.

**Open Graph.** Every public page emits Open Graph and Twitter Card meta
tags. A work page uses the work's cover image (§31) or an auto-generated
card with the work title and tags. A quote card (§36.1) is a variant of
this.

**Canonical URLs.** Every page has a canonical URL. A work reachable by
multiple paths (by ID, by slug, by old slug) has one canonical URL; the
others redirect or rel-canonical to it. Hreflang tags are emitted for
translated works (§22).

**Server-side rendering.** Public pages are server-rendered and readable
without JavaScript. A search engine's crawler sees the same content as a
reader with JavaScript disabled. Interactive features (reactions, voting)
require JavaScript; the content does not.

**Acceptance.**
- A work page emits JSON-LD `CreativeWork` structured data.
- A work page's Open Graph image is the cover or a generated card.
- A page reachable by multiple paths has one canonical URL.
- A public page's core content is readable without JavaScript.
- A sitemap is generated and includes all public works and authors.

## 36.14 User-Facing Webhooks and CLI

A developer or power user may configure webhooks for events on their
pseud or works: new work published, new chapter, kudos received, new
follower, new forum post in a subscribed topic, achievement unlocked. A
webhook is a URL, a set of subscribed events, and a secret for HMAC
verification. Events are delivered as JSON payloads with idempotency keys.

**CLI client.** A command-line client (`lorehaven-cli`) exposes the public
API (§23.1) as subcommands: `search`, `read`, `publish`, `import`, `stats`,
`sprint`. The client is a single binary, reads credentials from the system
keychain or a config file, and outputs JSON by default with optional human
readable formatting. Tab completion is provided for shells.

**Bot compatibility.** Discord bots, Telegram bots, Mastodon bots, and
Bluesky feed generators are thin REST clients of the public API (§23.1,
§23.2). No special bot endpoint exists; the public API is the bot
interface. The quote-card endpoint (§36.1) is one of the endpoints bots
call.

**Acceptance.**
- A user can configure a webhook for "new follower" events.
- A webhook delivery includes an HMAC signature and an idempotency key.
- A webhook that fails is retried with exponential backoff.
- The CLI client can search, read, and publish.
- A Discord bot posting a new work is a thin client of the public API.

## 36.15 Instance Discovery and Account Migration

A reader may discover other Lorehaven instances and migrate their account
between instances. The instance directory is a public listing of Lorehaven
instances that opt in to discovery, with stats (works, active users,
uptime, moderation style, federation scope).

**Directory.** Each instance in the directory lists: the instance name,
a short description, the primary fandoms (if any), the moderation style
(general, fandom-specific, NSFW, SFW), the federation scope (open, local,
unlisted), and live stats (works, active users, uptime). An instance opts
in by publishing a discovery record; the record is signed with the
instance's ActivityPub key.

**Discovery.** A reader may search the directory by fandom, language, or
moderation style. A "Find your community" wizard asks the reader about
their interests and recommends instances. The reader may view an instance's
directory entry and, with one click, begin account creation there.

**Migration.** A reader may migrate their account from one instance to
another. Migration exports the reader's works, bookmarks, reading
history, follows, achievements, and pseud identity, and imports them
on the destination instance. The destination instance creates the account,
imports the data, and redirects followers. The source instance marks the
account as migrated and stops serving it. Migration is a background job
with progress reporting.

**ActivityPub.** A migrated account is announced via ActivityPub. Followers
on other instances are notified of the new location. The old actor ID
resolves to a tombstone with a `movedTo` reference (ActivityPub
`Move` activity).

**Acceptance.**
- An instance can publish a discovery record.
- A reader can search the directory by fandom.
- A reader can migrate their account to another instance.
- After migration, the source account is a tombstone with a `movedTo`
  reference.
- Followers are notified of the migration.

## 36.16 Acceptance

- A quote card generated by an anonymous reader contains the selected
  text, work title, and author pseud.
- An AO3 profile URL with 50 works imports all 50; a mirror-mode import
  never alters the source.
- A reader who read at least one work in the prior year has a non-empty
  wrapped page.
- The daily prompt is the highest-voted unused prompt in the prior 24h.
- A gift exchange matching is deterministic; a participant with a DNW is
  never assigned a recipient requiring that DNW.
- A beta reader can claim an open request; annotations are private to the
  author and the assigned reader.
- A recommendation request surfaces auto-generated, machine-labeled
  suggestions.
- Fan art is invisible until the author approves it.
- An interactive work's reader can navigate branches and revisit chapters.
- A mood journal is private to the reader's pseud.
- A work with fewer than 10 views suppresses all breakdowns except total
  views.
- A work page emits JSON-LD structured data; public pages are readable
  without JavaScript.
- A webhook delivery includes an HMAC signature and an idempotency key.
- A migrated account's source is a tombstone with a `movedTo` reference.

## 36.17 What this section deliberately does not do

- **No paywalled growth.** Import from AO3, FFN, and Wattpad is a reader's
  right, not a paid feature. Cross-posting is free.
- **No ranking by popularity.** Recommendation threads surface responses by
  vote quality (§35.2), not by raw count. Gift exchanges match by
  preference, not by follower count.
- **No surveillance analytics.** Author analytics are aggregate and
  anonymized; no individual reader is identifiable.
- **No AI-generated content.** Auto-recommendations and auto-summaries
  (§35.5) are labeled and abstain when confidence is low. A mood-based
  recommendation draws from existing works, not generated text.
- **No lock-in.** Account migration means a reader who chose a small
  instance can move to a larger one without losing anything. The
  directory means a reader can choose again.

---

# 37. Multi-Platform Companion Bot

> **2026-09-20 addition.** Lorehaven's companion bot is a thin client of
> the public API (§23.1) that lets readers search, browse, download, and
> interact with works from Discord, Telegram, Matrix, IRC, the Fediverse
> (Mastodon, Bluesky), and the terminal. The bot never touches the
> database; it speaks the same REST API as every other client.

## 37.0 Architecture

The bot is a **separate workspace** (`lorehaven-bot`) with a platform-neutral
core driving thin adapters:

```
lorehaven-bot/
├── crates/
│   ├── bot-core/          # typed API client, PlatformMessage IR, dispatch, intent, store, cache
│   ├── lorehaven-bot-discord/     # Poise + Serenity adapter
│   ├── lorehaven-bot-telegram/    # Teloxide adapter
│   ├── lorehaven-bot-matrix/      # matrix-sdk adapter
│   ├── lorehaven-bot-irc/         # irc crate adapter
│   ├── lorehaven-bot-fediverse/   # Mastodon + Bluesky (raw XRPC)
│   └── lorehaven-bot-cli/         # REPL + TUI (ratatui) + download tool
```

**Platform-neutral core.** `bot-core` exports:
- `LorehavenClient` — typed HTTP client for every public API endpoint
- `PlatformMessage` — intermediate representation with `RichItem` + `ActionRow`
- `do_*` functions — one implementation of each command (search, recs, download, ...)
- `intent` — free-form mention classification (optional LLM, heuristic fallback)
- `store` — token store (`/link` flow) keyed by platform user id
- `cache` — Redis pagination + response cache
- `ratelimit` — cross-platform rate limiter

**Adapters.** Each adapter renders `PlatformMessage` natively and wires
platform events to the shared `do_*` functions. No adapter contains
business logic.

**Why a separate workspace.** The bot brings its own dependencies (Poise,
Teloxide, matrix-sdk, irc, ratatui) that Lorehaven's server does not need.
Deploying it independently means a bot restart does not restart the server.

## 37.1 Shared Infrastructure

**API client.** `LorehavenClient` wraps `reqwest::Client` and exposes one
method per public endpoint (§23.1): `search`, `recommendations`, `work`,
`work_download`, `work_bookmark`, `work_kudos`, `forum_categories`,
`forum_topics`, `forum_topic`, `forum_post_create`, `fandoms`, `fandom`,
`tags`, `random`, `trending`, `similar`, `also_bookmarked`, `blind_date`,
`comments`, `reading_status`, `user`, `user_works`, `user_bookmarks`,
`notifications`, `me`, `link_token`, `link`, `unlink`. Every method returns
a typed model; errors are `BotError`.

**Token store.** OAuth-style link flow: a reader runs `/link`, the bot
stores a pending code, the reader authorizes on the Lorehaven site, the bot
exchanges the code for a token. Tokens are stored in Redis keyed by
`bot:{platform}:{user_id}`. The store also holds per-user preferences
(filter chips, recs tuners, track state).

**Pagination cache.** Search and list responses are cached in Redis with a
short TTL. A pagination session pointer (`bot:page:{user_id}`) tracks the
current page so `next`/`prev` work without re-fetching.

**Rate limiting.** Cross-platform rate limiter using Redis buckets. A user
who spams commands across Discord and Telegram shares one bucket. Limits
are configurable per-platform.

**Intent classification.** When a user @mentions the bot with free text
(not a slash command), `intent::classify` parses it. With
`LOHAVEN_BOT_LLM_ENABLED=0` (default), a heuristic parser handles URLs,
questions, and keywords. With the flag on, a local LLM (Ollama) classifies
against a fixed `Intent` enum — prompt injection can only pick from the
closed action set.

## 37.2 Commands

Every adapter exposes the same command surface. The canonical command list:

| Command | Description | Auth |
|---------|-------------|------|
| `/search <query>` | Search works with optional filters | no |
| `/ask <question>` | Natural-language archive question | no |
| `/recs` | Personalized recommendations | yes |
| `/fresh` | Recently active works | no |
| `/gems` | Hidden-gem recommendations | no |
| `/roll` | Random work from recs | yes |
| `/download <url>` | Download links (EPUB/PDF/MOBI) | no |
| `/metadata <url>` | Metadata card for a URL | no |
| `/bookmark <url>` | Bookmark a work | yes |
| `/kudos <url>` | Leave kudos | yes |
| `/work <id>` | Work detail card | no |
| `/fandoms` | List all fandoms | no |
| `/fandom <slug>` | Fandom detail | no |
| `/forum cats` | Forum categories | no |
| `/forum topics <slug>` | Topics in a category | no |
| `/forum show <id>` | Thread detail | no |
| `/forum reply <id> <text>` | Reply to a topic | yes |
| `/forum follow <id>` | Subscribe to a topic | yes |
| `/forum mark-read <id>` | Mark topic read | yes |
| `/forum search <query>` | Forum full-text search | no |
| `/blind-date` | Random work, hidden metadata | no |
| `/trending` | Trending works | no |
| `/similar <id>` | Similar works | no |
| `/also-bookmarked <id>` | Readers also bookmarked | no |
| `/comments <url>` | Work comments | no |
| `/random` | Random work | no |
| `/help [topic]` | Help text | no |
| `/link` | Link Lorehaven account | no |
| `/unlink` | Unlink account | yes |
| `/me` | Linked account info | yes |

## 37.3 Discord Adapter

Built on Poise + Serenity. Slash commands with autocomplete for fandoms and
tags. Embeds for search results, metadata, and recommendations. Buttons
for pagination (`next`/`prev`) and actions (`download`, `bookmark`,
`kudos`). Free-form @mentions trigger intent classification.

**Guild configuration.** Server admins set default fandom filters, NSFW
channel restrictions, and command permissions via `/guild config`.

**OAuth.** `/link` generates a one-time code; the user authorizes on the
Lorehaven site; the bot stores the token.

## 37.4 Telegram Adapter

Built on Teloxide. Slash commands with the same surface as Discord.
Inline mode: type `@lorehavenbot <query>` in any chat for instant results.
Callback buttons for pagination and actions. Channel integration: admins
point a channel at a fandom or author feed for auto-posts.

**Instant View.** Works shared via the bot use Telegram's Instant View
renderer when available.

## 37.5 Matrix Adapter

Built on matrix-sdk. Logs into a homeserver, joins configured rooms, listens
for room messages. Renders `PlatformMessage` as HTML `m.room.message`.
Supports both slash commands and free-form @mentions.

## 37.6 IRC Adapter

Built on the `irc` crate. Connects to an IRC server, joins channels,
responds to `!command` prefix and free-form text. Renders as plain text
with numbered actions (IRC has no buttons).

## 37.7 Fediverse Adapter

**Mastodon.** Polls `/api/v1/notifications` for mentions. Replies to
mentions with rendered `PlatformMessage`. Posts new works to a configured
Mastodon account (one per work, with tags and quote card image). Supports
Misskey, Akkoma, Pleroma, GoToSocial via the same Mastodon-compatible API.

**Bluesky.** Polls `app.bsky.notification.listNotifications` via raw XRPC
(no atrium dependency). Replies are posted with
`com.atproto.repo.createRecord` on `app.bsky.feed.post`, threading into
the original post. Files are not supported from a bot app-password;
`PlatformMessage::File` degrades to the download link.

**Piefed/Lemmy.** Poll-based community monitor using the Lemmy API. Posts
new works matching configured filters to a community.

## 37.8 CLI/TUI Adapter

**REPL.** `lorehaven-bot repl` runs a read-evaluate-print loop. Line grammar:
- `quit`/`exit` — end session
- `!cmd args` — command dispatch
- `page N`/`next`/`prev`/`first`/`last` — paginate last list
- bare fanfic URL — metadata card
- `1`..`9` — pick numbered action from last reply
- any other text — intent classification

**TUI.** `lorehaven-bot tui` runs a three-pane terminal UI (ratatui):
left pane with Search/Forum tabs, right pane with metadata or threads,
bottom status bar + input line. Keybindings: `↑/↓` move cursor, `Enter`
open, `Tab` switch pane, `h/l` switch tab, `/` focus input, `q` quit,
`r` refresh, `d` download, `b` bookmark, `f` follow, `R` reply.

**Download tool.** `lorehaven-bot download <url>` fetches a work in the
specified format and writes it to disk. Pipe-friendly: `lorehaven-bot
search --json | jq '.results[0].url' | lorehaven-bot download --format epub`.

## 37.9 Webhooks

The bot can receive webhooks from Lorehaven (§36.14) and forward them to
configured platform channels. A new-work event in a fandom can be posted
to a Discord channel, a Telegram group, a Matrix room, or an IRC channel.

## 37.10 Configuration

Environment variables (all prefixed `LOHAVEN_BOT_`):

| Variable | Default | Description |
|----------|---------|-------------|
| `LOHAVEN_BOT_API_URL` | `https://lorehaven.polarisocial.xyz` | Lorehaven instance URL |
| `LOHAVEN_BOT_REDIS_URL` | `redis://127.0.0.1:6379` | Redis connection |
| `LOHAVEN_BOT_DISCORD_TOKEN` | — | Discord bot token |
| `LOHAVEN_BOT_TELEGRAM_TOKEN` | — | Telegram bot token |
| `LOHAVEN_BOT_MATRIX_HOMESERVER` | — | Matrix homeserver URL |
| `LOHAVEN_BOT_MATRIX_USER` | — | Matrix user ID |
| `LOHAVEN_BOT_MATRIX_PASSWORD` | — | Matrix password |
| `LOHAVEN_BOT_IRC_SERVER` | — | IRC server host |
| `LOHAVEN_BOT_IRC_NICK` | `lorehaven-bot` | IRC nick |
| `LOHAVEN_BOT_MASTODON_INSTANCE` | — | Mastodon instance |
| `LOHAVEN_BOT_MASTODON_TOKEN` | — | Mastodon access token |
| `LOHAVEN_BOT_BSKY_HANDLE` | — | Bluesky handle |
| `LOHAVEN_BOT_BSKY_PASSWORD` | — | Bluesky app password |
| `LOHAVEN_BOT_LLM_ENABLED` | `0` | Enable Ollama intent classification |
| `LOHAVEN_BOT_LLM_URL` | `http://localhost:11434` | Ollama URL |
| `LOHAVEN_BOT_LLM_MODEL` | `llama3.2` | Ollama model |
| `LOHAVEN_BOT_RATE_LIMIT_PER_MINUTE` | `10` | Commands per user per minute |
| `LOHAVEN_BOT_CACHE_TTL` | `300` | Response cache TTL (seconds) |

## 37.11 Acceptance

- A user can search Lorehaven from Discord, Telegram, Matrix, IRC, and the terminal.
- A user can link their Lorehaven account via `/link` and use authed commands.
- A user who @mentions the bot with a fanfic URL gets a metadata card.
- A user who @mentions the bot with a natural-language question gets an answer.
- Pagination (`next`/`prev`) works across all adapters.
- A Mastodon mention of the bot triggers a reply with search results.
- A Bluesky mention of the bot triggers a reply with search results.
- The TUI runs in a terminal and renders search results, metadata, and forum threads.
- The CLI download tool fetches a work in the specified format.
- A webhook from Lorehaven is forwarded to a configured Discord channel.
- A user who sends 11 commands in a minute is rate-limited on the 11th.
- The bot shares one rate limit bucket across Discord and Telegram for the same user.

## 37.12 What this section deliberately does not do

- **No server coupling.** The bot is a separate workspace with its own
  dependencies. It talks to Lorehaven's public API, not its database.
- **No AI-generated content.** Intent classification is optional and
  heuristic by default. The LLM can only pick from a closed action set.
- **No paywalled features.** Every command available in the bot is also
  available in the web interface. The bot is a convenience, not a premium
  tier.
- **No platform lock-in.** A reader who uses the bot on Discord can switch
  to Telegram without losing their linked account or preferences.


---

# 38. Instance Configuration — The Self-Hosted Contract

> **2026-09-20 addition.** Lorehaven is self-hosted. The operator — not the
> codebase — decides how their instance behaves. This section codifies that
> principle: every tunable value is admin-configurable via `lorehaven.toml`,
> with sensible defaults that let a fresh instance run without touching a
> single config line.

## 38.1 The principle

1. **No hardcoded magic numbers.** If a value affects behavior (timeouts,
   limits, thresholds, retention windows, budgets, taxonomies), it lives in
   `Config`, not a `const` in application code.
2. **Defaults are safe, not restrictive.** A fresh `lorehaven.toml` with
   every section omitted must produce a working instance. Defaults favor
   development-friendliness; production hardening is opt-in.
3. **Every config key has a documented default.** The operator can see what
   the default is without reading source code.
4. **Validation at startup, not at use.** A malformed config key refuses to
   start with a clear error message. No silent fallbacks to arbitrary values.
5. **Environment overrides file.** `LOREHAVEN_*` environment variables take
   precedence over the file, so containerized deployments can inject secrets
   and tune values without rewriting the file.

## 38.2 Currently configurable (verified against `Config`)

| Section | Key | Default | Purpose |
|---------|-----|---------|---------|
| `[site]` | `name` | `"Lorehaven"` | Instance display name |
| `[site]` | `base_url` | auto from bind+port | Canonical URL |
| `[site]` | `contact_email` | `None` | Admin contact |
| `[site]` | `topics` | `[]` | Public discovery topics |
| `[server]` | `bind` | `"127.0.0.1"` | Listen address |
| `[server]` | `port` | `8080` | Listen port |
| `[server]` | `max_body_bytes` | `2097152` (2 MiB) | Max request body |
| `[server]` | `request_timeout_secs` | `30` | Request timeout |
| `[database]` | `url` | `sqlite://./data/lorehaven.sqlite` | DB connection string |
| `[database]` | `max_connections` | `5` (dev) / `10` (prod) | Connection pool size |
| `[database]` | `acquire_timeout_secs` | `10` | Pool acquire timeout |
| `[storage]` | `root` | `"./data"` (dev) / `"/var/lib/lorehaven"` (prod) | Blob storage root |
| `[security]` | `cookie_secure` | `false` (dev) / `true` (prod) | Secure cookie flag |
| `[security]` | `session_ttl_days` | `30` | Session lifetime |
| `[security]` | `csrf_required` | `true` | CSRF protection |
| `[security]` | `trust_proxy` | `false` | Trust `X-Forwarded-*` |
| `[logging]` | `filter` | `"info,lorehaven_app=debug"` | Log filter |
| `[logging]` | `format` | `"pretty"` (dev) / `"json"` (prod) | Log format |
| `[accounts]` | `registration_open` | `true` | Allow new registrations |
| `[age]` | `threshold` | `14` | Age of consent threshold |
| `[age]` | `guardian_workflow_enabled` | `false` | Guardian authorization |
| `[rate_limits]` | `auth.burst`, `auth.per_minute` | `5`, `20` | Auth rate limit |
| `[rate_limits]` | `write.burst`, `write.per_minute` | `10`, `60` | Write rate limit |
| `[rate_limits]` | `search.burst`, `search.per_minute` | `20`, `120` | Search rate limit |
| `[rate_limits]` | `export.burst`, `export.per_minute` | `3`, `10` | Export rate limit |
| `[rate_limits]` | `default.burst`, `default.per_minute` | `30`, `180` | Default rate limit |
| `[imports]` | `solver_url` | `None` | CAPTCHA solver URL |
| `[imports]` | `archive_fallback` | `false` | Fallback to archive.org |
| `[imports]` | `honour_robots` | `true` | Respect robots.txt |
| `[theme]` | `mode` | `"thematic"` | Theme mode: `generic`, `thematic`, `adaptive` (§0.4.6) |
| `[theme]` | `allow_user_opt_out` | `true` | Whether the §16.5 dial can reach zero for theme influence |
| `[theme]` | `theme_dial_floor_bp` | `1000` | Dial lower bound (basis points) when opt-out is locked |
| `[theme]` | `adaptive_max_drift_bp` | `0` | Maximum adaptive drift in basis points (0 = no drift) |
| `[theme]` | `influence_sources` | `[{ kind = "operator_topics" }]` | Ordered influence sources (§0.4.6) |
| `[theme]` | `boost_tags` | `[]` | Tag substrings that boost discovery rank |
| `[theme]` | `suppress_tags` | `[]` | Tag substrings that suppress discovery rank |
| `[theme]` | `tag_gravity_bp` | `{}` | Per-tag gravity in basis points (overrides boost/suppress) |
| `[tts]` | `engine` | `"piper"` | TTS engine (`silent` for a pipeline check without a synthesizer) |
| `[tts]` | `piper_path` | `None` | Piper binary path |
| `[tts]` | `piper_voice_model` | `None` | Piper voice model |
| `[tts]` | `default_voice` | `None` | Default voice |
| `[tts]` | `monthly_spend_cap_cents` | `None` | Monthly TTS spend cap |
| `[directory]` | `page_size` | `50` | Entries per page |
| `[directory]` | `require_approval` | `true` | Submissions need operator approval before becoming visible |
| `[directory]` | `extra_categories` | `[]` | Operator-added categories beyond the seeded eight |
| `[directory]` | `vote_weighting` | `"trust_and_taste"` | `flat`, `trust`, or `trust_and_taste` |
| `[directory]` | `trust_vote_weights` | `"0.5,0.75,1.0,1.25,1.5,1.75,2.0"` | Multiplier per trust level TL0–TL6 |
| `[directory]` | `taste_vote_floor` | `0.75` | Vote multiplier at taste affinity 0 |
| `[directory]` | `taste_vote_ceiling` | `1.25` | Vote multiplier at taste affinity 1 |
| `[forum]` | `vote_budget` | `[(1,10),(3,30),(5,60)]` | Vote budget per TL |
| `[forum]` | `karma_decay_percent` | `5` | Monthly karma decay % |
| `[forum]` | `meta_mod_points` | `1` | Points per meta-mod verdict |
| `[forum]` | `meta_mod_min_verdicts` | `3` | Min verdicts to elect moderator |
| `[forum]` | `min_vote_weight_bp` | `100` | Min vote weight in basis points |
| `[forum]` | `work_discussion_default` | `"thread_only"` | Default discussion mode |
| `[browse]` | `default_sort` | `"for-you"` | Ordering for a signed-in reader with a profile (§43.4) |
| `[browse]` | `anonymous_sort` | `"top"` | Neutral ordering for anonymous traffic (§43.4) |
| `[browse]` | `surface_defaults` | unset | Per-surface overrides of `default_sort` |
| `[weighting]` | `mode` | `"trust_taste_contribution"` | `flat \| trust \| trust_taste \| trust_taste_contribution` (§16.16) |
| `[weighting]` | `taste_floor` | `0.75` | Taste multiplier floor; `taste_floor × taste_ceiling ≤ 1.0` enforced |
| `[weighting]` | `taste_ceiling` | `1.25` | Taste multiplier ceiling |
| `[weighting]` | `contribution_floor` | `1.0` | Contribution multiplier floor |
| `[weighting]` | `contribution_ceiling` | `2.0` | Contribution multiplier ceiling |
| `[weighting]` | `contribution_window_days` | `180` | How far back contribution is counted |
| `[weighting]` | `demand_diversity_percent` | `20` | Fraction of surfaced demand with no boost; must be `> 0` |
| `[discovery]` | `taste_sources` | `[{ kind = "admin" }]` | Taste sources and their members (§16.15) |
| `[discovery]` | `taste_source_min_members` | `5` | Cohort-size floor enforced at startup |

## 38.3 Hardcoded values that MUST become configurable

These are currently `const` values in application code. They must be moved to
`Config` with defaults and TOML keys.

| Current location | Constant | Proposed key | Default |
|------------------|----------|--------------|---------|
| `crates/app/src/exports.rs` | `RETENTION_DAYS = 7` | `[exports] retention_days` | `0` (forever) |
| `crates/app/src/exports.rs` | `GRANT_TTL_SECONDS = 3600` | `[exports] grant_ttl_secs` | `3600` |
| `crates/app/src/revisions.rs` | `REVISION_TTL_SECONDS = 604800` | `[revisions] ttl_secs` | `604800` |
| `crates/app/src/worker.rs` | `TERMINAL_JOB_RETENTION = 30d` | `[jobs] terminal_retention_days` | `30` |
| `crates/app/src/bulk_export.rs` | `DEFAULT_MAX_ITEMS = 50` | `[bulk_export] max_items` | `50` |
| `crates/app/src/bulk_export.rs` | `DEFAULT_MAX_BYTES = 1GiB` | `[bulk_export] max_bytes` | `1073741824` |
| `crates/domain/src/library.rs` | `UPDATE_CHECK_RETENTION_DAYS = 90` | `[library] update_check_retention_days` | `90` |
| `crates/app/src/library_updates.rs` | `CHECK_BATCH = 50` | `[library] check_batch` | `50` |
| `crates/app/src/administration.rs` | `webhook_timeout_secs = 10` | `[administration] webhook_timeout_secs` | `10` |
| `crates/app/src/administration.rs` | `webhook_max_attempts = 5` | `[administration] webhook_max_attempts` | `5` |
| `crates/app/src/administration.rs` | `webhook_base_delay_ms = 500` | `[administration] webhook_base_delay_ms` | `500` |

## 38.4 Migration path

1. Add new fields to the relevant `*Config` struct with `serde::Deserialize`.
2. Add defaults to `*Config::default()`.
3. Read values in `Config::load()` from the TOML file.
4. Replace `const` references with `config.xxx` calls.
5. Update `docs/spec.md` §38.2 and §38.3.
6. Update `docs/config-reference.md` (create if missing).

## 38.5 Acceptance

- Every hardcoded value in §38.3 has a corresponding `[section] key` in
  `lorehaven.toml`.
- `lorehaven.toml` with all defaults omitted produces a working instance.
- Setting `retention_days = 0` disables export cleanup entirely.
- A malformed config value (e.g., `retention_days = -1`) refuses to start
  with a clear error.
- All existing tests pass with default config values.

## 38.6 What this section deliberately does not do

- **No runtime reload.** Config changes require a restart. (Future: SIGHUP
  reload, but not now.)
- **No per-user overrides.** Instance config is global. User preferences are
  in `privacy_settings` and `reader_settings`.
- **No feature flags.** This is about tuning values, not toggling features.
  Feature gating is done at compile time or via Cargo features.

# 39. Resource Directory — a community-curated map of the fandom ecosystem

> **2026-09-21 addition.** Lorehaven is one node in a fandom ecosystem that
> spans archives, Discord servers, author platforms, writing tools and
> communities. Readers arriving at a fresh instance have no map of that
> ecosystem; this section gives them one, ranked by the people who use it.
> The design borrows the shape of a well-known "list-of-sites" directory:
> many categories, one ranked list per category, ranked by users — but every
> ranking input is a named community vote, never traffic, never money, and
> never the administrator's private taste (§0.3).

## 39.1 Scope — lists and their entries

The surface is a set of **curated lists**. A list is a ranked collection of
**entries**, and an entry is one of two kinds:

- **External resources** — things that do not live on this instance: fanfiction
  archives, Discord servers, author platforms and homepages, writing tools,
  communities (subreddits, Tumblr tags, forums), podcasts and newsletters, and
  other Lorehaven instances (which also appear in the §36.15 instance
  directory; a list entry links to that record rather than duplicating its
  stats).
- **Internal references** — works, authors (pseuds), tags and fandoms **on this
  instance**. An internal entry stores the referenced id, not a copy: a work
  entry renders from the live work row, so a list never goes stale and never
  widens what the reference alone may show (an ineligible work is refused at
  submission, and an entry whose work later becomes private is hidden from
  everyone but its curator until it is public again).

Every list has a **kind** — `external`, `works`, `authors`, `mixed` — and the
kind gates what an entry may reference: a `works` list accepts only work ids,
an `authors` list only pseud ids, `mixed` and `external` accept any external
resource (internal references only in `mixed`). Categories from §39.2 apply
to external entries inside any list.

**Instance lists.** Any list may be marked by the operator as an **instance
list** — the curated face of this instance, shown on the landing page and in
the site header ("Start here", "This month's picks", "The archives we read").
Instance lists are the operator's editorial surface: the operator picks which
lists represent the instance and in what order; the ranking inside each list
is still the community's votes (§39.4). An instance list is read-only to
everyone but its curator and the operator.

## 39.2 The surface

`/directory` with:

- **Category tabs**, one ranked list per category, ordered by score
  descending, ties broken by submission date (older first).
- **Entry cards**: title (linked), one-line description, tags, score, vote
  controls, submitter handle, submission date. An entry the viewer has already
  voted on shows its state.
- **Search** across title, description and tags, per category or global.
- **Tag filter**: clicking a tag narrows the list to entries carrying it.
- **Pagination** with a page size set by `[directory].page_size` (default 50).

Categories are seeded (`fanfiction_archive`, `discord_server`,
`author_platform`, `writing_tool`, `community`, `podcast_newsletter`,
`lorehaven_instance`, `other`) and the operator may add more through
`[directory].extra_categories` — the self-hosted contract (§38) applies: the
category list is instance configuration, not code.

## 39.3 Submission and review

Any signed-in account may submit an entry: category, title, URL, description
(≤ 500 characters), optional tags. The URL must be absolute http(s) and must
not resolve to a private or loopback address (the same SSRF rules the
§14.9 webhook verifier follows).

Review is operator-configurable:

- `[directory].require_approval = true` (default): a submission is **pending**
  and invisible to everyone except its submitter and the operator until the
  operator approves it. Approval is one click from the operator's queue and
  is recorded with approver and timestamp.
- `require_approval = false`: submissions are visible immediately. The
  operator may still remove an entry, and removal is recorded.

An entry may be edited by its submitter while pending; after approval, only
the operator may edit or remove. Removal never cascades to votes — a removed
entry's rows stay for audit and a resubmission of the same URL starts at
zero.

## 39.4 Voting and ranking

One vote per account per entry, `+1` or `-1`, toggleable (voting the same
value again removes the vote; voting the other value flips it). The score is
the sum of live weighted votes, denormalised onto the entry row and
recomputed inside the same transaction as the vote so the list never shows a
stale score.

**Vote weight.** A vote carries a weight — the directory is the instance's
curated front door, and whose recommendation counts more is operator policy:

```text
vote_weight = trust_multiplier(voter_trust_level)
            × taste_multiplier(voter_admin_taste_affinity)
```

- **Trust multiplier** scales with the §19.1 ladder. Defaults:
  TL0 0.5, TL1 0.75, TL2 1.0, TL3 1.25, TL4 1.5, TL5 1.75, TL6 2.0 —
  a reviewed trusted regular's recommendation counts for more than a
  day-old account's, and the operator can retune every rung through
  `[directory].trust_vote_weights` (seven comma-separated multipliers).
- **Taste multiplier** scales with the voter's affinity to the
  administrator's taste profile — the same `aggregate_affinities` signal the
  §9.7.4 demand multiplier uses, applied to a voter instead of a work.
  Affinity 0 maps to `[directory].taste_vote_floor` (default 0.75) and
  affinity 1 maps to `[directory].taste_vote_ceiling` (default 1.25).

Weighting is operator-configurable: `[directory].vote_weighting` is one of
`flat` (every vote weighs 1 — the weightless directory), `trust` (trust
multiplier only), `trust_and_taste` (both; the default). `flat` is always
available as the off switch.

**Silence is the contract (§0.3).** Weights are applied, never shown: no
voter list, no per-voter attribution, no weight breakdown, no user-facing
label that taste influenced anything. A reader sees a score; nothing about
the score reveals the administrator's taste profile or any individual's
affinity to it. The same rule ratings follow (§33.2) applies to the
individual vote; the same rule the demand multiplier follows (§9.7.4)
applies to the taste signal.

A vote on a directory entry is public in the aggregate (the score) and
private in the individual. Weight recomputation is triggered by trust-level
changes and nightly taste-refresh jobs, folded into the denormalised score
in the same transaction as the change.

## 39.5 Acceptance

- An anonymous reader sees only approved entries, ranked by score, and can
  search and filter by category and tag.
- A signed-in reader can submit an entry, and with `require_approval = true`
  sees their own pending entry with a visible "pending review" state.
- Voting twice with the same value removes the vote; voting the other value
  flips it; the score the list shows reflects the change immediately.
- With `vote_weighting = "trust_and_taste"`, a TL4 vote moves the score more
  than a TL0 vote on the same entry; with `vote_weighting = "flat"` they move
  it equally.
- No surface, API response or error message reveals a voter's weight, the
  taste component of any score, or the administrator's affinity to any
  voter.
- A URL that is not absolute http(s), or that points at a loopback or private
  address, is refused with a named reason.
- The operator's queue lists pending entries with submitter and date;
  approval makes the entry visible to everyone in the same request.
- A category added through `[directory].extra_categories` appears on the
  surface after a restart with no code change.

## 39.6 What this section deliberately does not do

- **No paid placement, no affiliate links, no sponsored entries.** Money
  never touches the directory (§0.3).
- **No weight disclosure.** Vote weights — trust or taste — are never
  surfaced, exported or explained per voter (§0.3).
- **No traffic or uptime probing.** The instance does not fetch listed URLs
  on a schedule; a dead link is reported by users, not detected by bots.
- **No federation of the directory itself.** Entries are local to this
  instance. Sharing curated lists between instances is a future concern and
  would ride §36.15's discovery records, not this table.
- **No per-entry comment threads.** The forum (§17) is where discussion
  belongs; an entry links out, it does not host.

---

# 40. Remix — fork with provenance, and permission statements made enforceable

> **2026-09-21 addition.** Spec §33.1 defined permission statements and
> derivative lineage as a draft. This section schedules their implementation
> and adds the one thing §33.1 described only as data: the *affordance*. A
> derivative edge nobody can create is a schema, not a feature.

## 40.1 The fork affordance

A **Fork this work** action on every eligible work page. One click creates a
new draft owned by the forker, linked to its parent through the §33.1
derivative lineage edge (`kind = remix` by default), inheriting the parent's
tags, fandoms, characters and relationships, and copying no body text — a
fork starts empty, because the forker's words are the point.

Guards, in order:

1. **Permission statement.** The parent's `remix` statement is checked: `no`
   refuses with the statement named in the refusal; `ask` routes to a
   request the author answers (never auto-approved); `unstated` and `yes`
   proceed. The refusal names the statement, per §33.1.
2. **Exclusion registry.** A parent on the exclusion registry is refused by
   name.
3. **Depth limit.** `[works].max_fork_depth` (default 3): a work whose
   lineage chain is already that deep refuses with the chain shown. Fork
   chains that go forever are how low-effort copies drown a platform.
4. **Visibility.** A fork of a private work is private; a fork never widens
   the parent's audience.

The parent's page lists its children under a **Remixes** heading, each with
its lineage kind and a link. The child's page names its parent the same way.
The edge survives orphaning (§32.3) and deleting a parent never deletes a
child.

## 40.2 Permission statements, implemented

The §33.1 statement model becomes real on every work: `podfic`,
`translation`, `remix`, `continuation`, `redistribution`, `ai_training` —
each `yes | ask | no | unstated`, defaulting to `unstated`, editable only by
the owner, surviving orphaning and account deletion. The work editor grows a
**Sharing & permissions** panel with one control per statement and a plain
sentence under each ("Readers may translate this work without asking" /
"Readers must ask first" / "Readers may not translate this work").

Enforcement doors, each naming the statement in its refusal:

- Fork (§40.1) checks `remix`.
- The translation pipeline (M17) checks `translation` before a request is
  created.
- Narration/TTS (M26) checks `podfic`.
- The import adapters (M6) carry a source-published statement through with
  provenance and never upgrade `unstated` to `yes`.

## 40.3 Acceptance

- The fork button on a work whose statement is `remix: no` refuses and names
  the statement; `ask` creates a request; `yes` and `unstated` create a
  draft.
- A fork inherits tags and lineage, copies no body text, and appears in its
  parent's Remixes list.
- A chain at `max_fork_depth` refuses with the chain shown.
- Every statement is editable by the owner only, and the editor shows the
  effective sentence for each.
- A statement change is recorded with its before and after values.

## 40.4 What this section deliberately does not do

- **No automatic enforcement of external platforms' rules.** The statement is
  the author's declaration; chasing violations off-instance is the author's
  choice, not the instance's job.
- **No licence generation.** The statement is not a legal licence (§32 owns
  rights); it is a machine-readable social contract.
- **No retroactivity.** Statements gate new derivatives; existing ones stand.

---

# 41. Longevity and ambient social signals

> **2026-09-21 addition.** Two Gravity-derived signals, adopted because they
> serve the priority stack's first item — maximise high-quality fiction —
> without optimising for engagement (§0.3's vital-sign rule).

## 41.1 Content half-life

A work's **half-life** is how long it keeps being read after publication.
Freshness (§16.14) biases recency; completion rates (§9.8) measure the first
read. Neither answers the question a reader actually asks: *is this still
worth reading now?* A work still being finished and discussed two years on
is evergreen quality — the thing the priority stack names first.

Computation, nightly, per work published more than `[discovery].half_life_min_age_days`
(default 30) days ago: from `reading_events`, the ratio of readers who
started the work in the last 30 days to those who started it in its first 30,
expressed in basis points (`half_life_bp`, `INTEGER`, house binding rules).
The score is written onto the work row by a scheduled job and is **never
shown to readers as a number** — it is a ranking input, consumed by the
discovery engines as a multiplier on `Candidate.score` (§16.3's blend), the
same silent-reordering contract operator affinity follows.

Configuration: `[discovery].enable_half_life` (default `false` until the job
proves itself), `half_life_min_age_days`, `half_life_window_days` (default 30).

## 41.2 Interaction tiers — ambient warmth

Most readers never comment (§17's positivity model shapes comments; it has
nothing for the silent majority). Interaction tiers give a shy reader a
graded way to matter: every interaction with an author's works — reading a
chapter, finishing a work, reacting, commenting — accumulates a private
**warmth** value between the reader's account and the author's. Warmth
crosses thresholds (`lurk < react < comment < create`, the thresholds in
basis points under `[community].warmth_thresholds`) and the resulting tier
is shown to the author as an aggregate only: "this month, 34 lurkers, 12
reactors, 4 commenters, 2 creators engaged with your works." Never a list of
names, never a per-reader value, never shown to the reader themselves — the
reader sees nothing, because a warmth meter on your own reading is a
streak-shaped dark pattern (§0.2 attention rules).

Recording hooks: `record_reading_progress`, quick reactions, comments. Each
hook writes one row (upsert by pair, accumulate the delta, recompute tier)
inside the same transaction as the interaction it accompanies.

## 41.3 Acceptance

- With `enable_half_life = true`, the nightly job writes `half_life_bp` on
  eligible works and a changed score reorders discovery output with no field
  changes (the §16.3 silent rule).
- A work with no recent readers has a lower half-life score than one still
  being finished; the job is idempotent — running twice changes nothing.
- An author's aggregate panel shows tier counts; a test asserts no endpoint
  exposes a per-reader warmth value or a reader list.
- Warmth accumulates across interactions within one transaction; a failed
  interaction writes no warmth.

## 41.4 What this section deliberately does not do

- **No public half-life badge.** The score is a ranking input, not a label;
  publishing it would make recency-of-attention a target.
- **No reader-facing warmth.** The reader never sees their own tier or
  progress toward one — the anti-streak rule is absolute.
- **No warmth in ranking.** Warmth describes a reader-author relationship;
  it never feeds discovery, search or any public ordering.


# 42. Export CTAs — instance-configurable, curator-marked

Exported ebook files carry a call-to-action (CTA) pointing readers back to
the instance: "read more at …", "leave a comment", "support the author".
CTAs are growth surface, and growth surface is exactly what must not be
forced on authors who already do their own outreach.

## 42.1 Placement

The CTA is appended after the last paragraph of a chapter's XHTML:

- **`per_chapter`** (default): every chapter ends with the CTA.
- **`per_work`**: only the final chapter carries the CTA.
- **`off`**: no CTA anywhere.

```toml
[exports]
cta_placement = "per_chapter"   # per_chapter | per_work | off
cta_html = "Read more at <a href=\"https://example.org\">Example</a>."
```

`cta_html` is an operator-authored XHTML fragment. It is sanitized with the
same allow-list as chapter bodies ([`crate::document`]) before it is
embedded — an operator's configure-file typo must not produce an invalid
EPUB, and a compromised config file must not smuggle script into readers'
devices.

## 42.2 The author-CTA exemption

A work whose author already includes their own CTA (in an author's note,
a bio block, a "support me" paragraph) must not receive the instance CTA:
doubling up is noise, and speaking over the author is the one thing this
platform does not do.

Whether a work carries its own CTA is **marked by curators** — accounts at
trust level ≥ 3 — through a quorum vote:

- `cta_marks(work_id, curator, has_own_cta, marked_at)` records each mark.
- A work is exempt when marks agreeing on `has_own_cta = true` reach
  **quorum** (default 2 agreeing marks, `[exports] cta_quorum = 2`).
- Marks are revocable; retracting a mark recomputes the exemption.
- The exemption is evaluated at export time, never cached in the work row:
  a curator's retraction takes effect on the next export.

## 42.3 Rules

- The exemption applies to the whole work, not per chapter.
- The CTA never renders inside the attribution block (§13.3) — attribution
  is legal notice, not growth surface.
- `validate()` reports `cta_chapters: Vec<u32>` so tests can assert exactly
  which chapters carry the CTA.
- If `cta_html` is unset, the default is a plain link to the instance's
  base URL.

## 42.4 Acceptance

- Default config exports a work with `per_chapter` CTAs on every chapter.
- `per_work` places the CTA on the final chapter only; `off` places none.
- A work with quorum-marked `has_own_cta` exports with no CTA in any
  placement mode.
- A single curator's mark without quorum does not suppress the CTA.
- Retracting marks below quorum restores the CTA.
- Malformed `cta_html` (e.g. `<script>`) is sanitized or refused, never
  embedded raw.
- The CTA does not appear in the attribution section of any export.

---

# 43. Recommendation-first browsing — one ordering contract for every surface

> **2026-09-21 addition.** §16 defines the engines, §16.5 promises the reader's dial applies to
> "all surfaces", and §16's acceptance says exact and chronological sorts stay exact — but no
> section ever named the surfaces or defined how a browse page chooses an order. In practice each
> route author chose, and this section says otherwise: a reader who has told the instance what they
> like gets that taken into account *wherever they browse*, and every other ordering remains one tap
> away. This section is the contract; §16 is the mechanism.

## 43.1 Surfaces

Every one of these takes the §43.2 vocabulary, and each has a configured default:

`/discover`, `/fandoms/:id`, `/tags/:tag`, `/moods/:mood`, `/people`, `/authors/:id`,
`/collections/:id`, `/series/:id`, `/reading-paths/:id` (§18.9), the discovery rails inside
`/library`, directory lists (§39), the challenge, request and wishlist boards (§18.2, §18.5), and
the e-mail digest.

A surface added later inherits this contract rather than inventing an order; §26's route ownership
table names the owning module, and a new surface without a `sort` parameter is a defect.

## 43.2 The vocabulary

One enum, everywhere, documented and translated once:

| `sort` | Meaning | Exactness |
|---|---|---|
| `for-you` | the §16 engine blend and the reader's recipe | a permutation (see 43.3) |
| `new` | publication or update event order, from the §4.3 event log rather than a row timestamp | exact |
| `updated` | last publication event | exact |
| `top` | §15.10's quality signals: completion rate, positive-feedback ratio, recency and update velocity, bibliography quality | exact, never taste-steered |
| `trending` | time-windowed with §16.14's decay | exact |
| `best-match` | literal query relevance; search only | **never taste-steered (§15.10)** |
| `az` | alphabetical; people, tags, collections | exact |

An unknown value is a validation error naming the accepted set, never a silent fallback.

## 43.3 `for-you` is a permutation, never a filter

The reachable set for a given filter is **identical under every `sort` value**. `for-you` changes
the order of results and the page they appear on; it never removes one, never truncates the last
page, and never changes a count. Pagination must be able to walk to the final item of the
underlying set under `for-you` exactly as it can under `new`.

This is the section's load-bearing rule. A taste-skewed default that quietly reduces reachability is
a shadowban (§19.7) applied by ranking instead of by moderation — invisible by construction, because
§0.3 forbids explaining the order, and unaccountable for the same reason. A test asserts set
equality: for one fixture, the union of pages under `for-you` equals the union under `new`, in both
directions, including for a work `for-you` ranks last.

## 43.4 Defaults

- **Signed in with a profile:** `for-you`, unless the operator configures otherwise per surface.
- **Signed in with no profile:** `for-you` must be indistinguishable from the baseline order. §16's
  acceptance already requires the fallback; this makes it observable.
- **Anonymous:** the surface's neutral order (`top`, `new` or `trending`). `for-you` is refused for
  anonymous traffic while the instance's declared topics are empty or all `public = false`, because
  the *ordering itself* would publish a private theme on the front page (§0.4.3, §16.16.2). With at
  least one public topic, a taste-skewed default for anonymous readers is permitted and the
  mechanism may be documented.
- **Stickiness:** the choice is per-pseud and remembered across sessions, mirroring §16.8's
  per-pseud dashboard layouts. It is a *sort choice*, not a dial — §16.5's weight remains the only
  influence control, and there is no per-surface strength setting.

## 43.5 Composition rules

A `for-you` page is assembled in the order the implementation already uses for `/discovery`
(`crates/app/src/routes/discovery.rs`): shared candidate fetch → blend → silent reordering
(half-life §41.1, operator affinity §16.3) → diversity reservations → paginate. Consequences:

- **Rank after the shared fetch.** Per-reader ordering is never part of a shared cache key; the
  candidate pool stays cacheable and only the ordering is per-reader (§10.4's cache boundaries).
- **§16.4's diversity budget applies per surface**, not only on `/discover`. A reservation that
  holds on one page and not the next just moves the collapse it prevents.
- **Every candidate still passes §16.1's shared eligibility rules.** Ordering is the last step, not
  a bypass.
- **The `reason` field travels with `for-you` items** (§16.1) and obeys §16.16.2: a reason may name
  the reader's own terms, and instance terms only for a topic declared `public = true`.

## 43.6 Measurement

The operator's discovery page reports, per surface and per period: what `for-you` surfaced, what
each explicit sort surfaced, the overlap between them, and the share of surfaced items that had
never been surfaced before. This is §16.13's "the slot's effect is measured and reported" extended
to every browse page, and it is the only way an operator can see `for-you` collapse into the same
forty works on every page. Measurements are aggregate; no per-reader ordering history is exposed to
anyone, including the operator (§3.7).

## 43.7 Acceptance

- Every §43.1 surface accepts the §43.2 vocabulary and answers an unknown value with a named error.
- Under two different `sort` values, the reachable set for one filter is identical, counts included.
- `for-you` with no profile matches the baseline order exactly.
- An anonymous reader on an instance with no public topic is never served a taste-skewed default,
  and no reason field names a theme term on any anonymous page.
- An exact or literal query is not reordered by any profile, at any weight (§15.10).
- The sort choice survives a new session and does not leak across pseuds.
- Per-surface diversity reservations hold under the §25.4 load profile.
- A `for-you` page lands within the timing budget of the same surface's `new` page; ranking happens
  once per page over a bounded candidate pool, not once per item.

## 43.8 What this section deliberately does not do

- **No taste-steering of exact search.** §15.10's rule is untouched; `for-you` is an ordering among
  eligible results, and a literal query is not a recommendation request.
- **No per-surface influence dial.** One dial (§16.5) governs instance influence; this section
  governs display order only.
- **No new privacy setting.** Defaults follow §0.4.3's existing topic-visibility rule. This section
  introduces no separate "recommendations on browse pages" consent, because the reader's existing
  dial and sort choice already cover it.
- **No reordering of a path, series or curated list.** §18.9 is explicit that a path never reorders
  its stops: `for-you` orders the *list of paths*, never the contents of one.
- **No change to `top`.** The quality ordering stays as §15.10 defines it; `for-you` sits beside it
  rather than replacing it.

