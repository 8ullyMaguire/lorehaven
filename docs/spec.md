# Lorehaven: Complete Implementation Specification

This is a dependency-ordered specification for building the entire platform—not a prototype. It defines architecture, data structures, workflows, implementation milestones, and verification requirements.

This document specifies intended behavior. It does not claim that any feature, adapter, integration, or performance target has already been implemented or verified.

---

# 0. Site Premise

## 0.1 In one sentence

Lorehaven is a self-hosted, self-governing fanfiction platform that lets users shape their own experience through extensions, grows the body of available fiction through frictionless importing and writing, keeps authors motivated through a positivity-first feedback culture, and uses trust-level and quorum-based community governance so the administrator can focus on curating taste rather than policing behavior.

## 0.2 Priority stack

When two goals conflict, the higher-numbered priority yields to the lower-numbered one.

1. **Fully customizable website** through installable themes, recommendation engines, widgets, writing tools, challenge variants, and reader enhancements.
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
- The administrator's taste profile must never be visible, inferable, or hinted at through any user-facing label, multiplier name, or credit breakdown.
- Honest verification claims. Feature presence in this document is not evidence of implementation.

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
0013-no-gamification-decision.md
0014-declarative-recipes-vs-scripting.md
0015-shadowban-policy.md
0016-presence-and-typing-indicators.md
```

## 1.5 Features do not override foundational protections

No feature—including a paid or admin-configured one—may:

- Reveal hidden pseud linkage.
- Bypass content eligibility checks.
- Purchase trust, ranking, or moderation authority.
- Introduce XP, levels, or reputation scores.
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
| Optional billing | Stripe adapter, with billing-disabled operation |
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
| Library | bookmarks, notes, shelves, shelf_entries, reading_progress, reading_events, reading_aggregates, saved_searches, author_watches, reading_goals |
| Reader feedback | ratings, reviews, review_revisions, work_metric_aggregates, quick_reactions, appreciation_notes, cheer_events |
| Positivity | comment_classifications, feedback_holds, moderation_queue_entries, author_visible_feedback, classifier_training_signals |
| Jobs | jobs, job_attempts, job_events, outbox_events |
| Search | search_documents, search_index_state, search_demand_aggregates, fuzzy_correction_suggestions |
| Community | comments, reactions, follows, groups, memberships, boards, topics, posts, polls, poll_votes, post_drafts, scheduled_posts |
| Forum depth | topic_read_states, topic_tags, topic_tag_assignments, watch_preferences, mention_events |
| Messaging | conversations, conversation_members, messages, chat_rooms, presence_preferences |
| Collections | collections, collection_roles, collection_submissions, collection_entries |
| Writing events | challenges, prompts, signups, assignments, claims, fulfillments, mentorships, sprints, wishlist_items, wishlist_votes, request_candidates |
| Governance | trust_policies, trust_history, expertise, role_assignments, reports, cases, proposals, votes, sanctions, appeals, audit_events, process_feedback, quorum_records, shadowban_actions |
| Economy | wallets, ledger_transactions, ledger_entries, credit_holds, subscriptions, payment_events, bounties, entitlements |
| Extensions | packages, package_versions, manifests, installations, grants, reviews, approvals, execution_usage, revocations, extension_purchases, extension_ratings, webhook_subscriptions |
| Discovery | user_preferences, taste_profiles, taste_profile_versions, permitted_signals, exposure_events, aggregate_affinities, similarity_suggestions, similarity_votes, recommendation_recipes, recipe_versions, diversity_budgets, editorial_picks |
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

Restricted Tiptap schema: paragraphs, headings, emphasis, strong, lists, blockquotes, links, scene breaks.

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

Chapter navigation, whole-work mode, table of contents, typography settings, light/dark/sepia themes, width/line-height controls, distraction-free mode, spoiler reveal, progress, private notes, search within current work, reading-time estimates, end-of-work actions.

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

Do not count opens as proof of reading. Label estimates as such.

## 9.7 Reading goals, streaks, and gamification

### 9.7.1 Design principles

Gamification rewards the behaviors the platform wants more of: completing fiction, leaving genuine feedback, importing new sources, reading diversely, and fulfilling community demand.

Rules:

- **Never punish inactivity.** Missing a day costs nothing. Streaks reset without guilt messaging.
- **Never create anxiety.** No "your streak will break" notifications. No loss-framed language.
- **Reward quality, not volume.** Completion rates and positive feedback multiply author earnings. Raw word count and post count do not.
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

**Deliberately absent:** most words written, most posts, most kudos given, longest streak, any global XP ranking.

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

**Acceptance**

- Daily login credits award once per calendar day.
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
- Honor `Disallow` as a refusal, not a warning. A path the source forbids is not fetched, and an import that needs it reports the restriction rather than a parse failure. Matching is the de-facto standard's: `*` and trailing `$` supported, most-specific rule wins, `Allow` breaks a tie, and a group naming our product token applies ahead of the `*` group. The token is our `User-Agent`'s leading word (`Lorehaven`).
- Use **one request per second** when no delay is published. "No information" must not be read as "no limit", and absence is not permission to go faster. One second is also the floor for a host whose published delay is shorter or unreadable, so a malformed directive can never become a faster pace.
- Treat a `404` or `410` for `robots.txt` as a site with no restrictions. Treat any other failure to read it as rules unknown: proceed at the default pace, and record the condition against the source's health rather than refusing a reader's import over a file that is temporarily broken.
- Read it once per host per import run, and cache it for a bounded interval when a process outlives one run.

This is enforced inside the shared fetcher, for the same reason the address checks are: an adapter that could opt out of pacing would make the rule advisory. An import is resumable, so a slow import is a cost the reader can wait out; an import that hammers a volunteer-run archive is a cost somebody else pays.

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

## 15.5 Fuzzy matching and typo tolerance

Misspelled tags, author names, and fandom names produce "Did you mean?" suggestions rather than silent auto-correction.

Implementation:

- SQLite: trigram-based similarity or FTS5 with distance ranking.
- PostgreSQL: `pg_trgm` extension for similarity queries.
- Suggestion threshold configurable per query type.
- Never auto-execute a corrected query; the user must confirm.

Fuzzy matching applies to tag names, author handles, fandom names, and work titles in search input. It does not apply to body text (which uses exact phrase matching).

## 15.6 Metadata completeness

Negative filters support:

- Match declared metadata.
- Require sufficiently complete metadata.
- Include uncertain results separately.

Missing ship metadata is not proof that a story contains no ship.

## 15.7 Filters

Characters and prominence, relationships and prominence, relationship kinds and exclusions, character attributes and roles, fandom and crossovers, completion, word/chapter ranges, rating and warnings, language, dates, author, collection, series, tropes, settings, moods, content notes (worldbuilding-heavy, dialogue-driven, etc.), length histogram, public works/private library scope, read/unread, user mutes.

Bound query depth, clause count, result windows, execution time.

## 15.8 Mood and tone taxonomy

Curated initial mood tags: comfort, angst-with-happy-ending, angst, cozy, adventure, slow-burn, fast-paced, episodic, dark, hopeful, humorous, bittersweet, catharsis.

Authors assign moods to their works. Readers filter by mood. Community proposals extend the taxonomy through quorum.

Moods are distinct from plot tags: they describe reading experience, not plot elements.

## 15.9 Full-text body search

Index permitted body text for published eligible local works, the requesting pseud's private imported copies, and explicitly authorized public preservation content.

Phrase search, prose and dialogue search, chapter-level matches, highlighted snippets, jump-to-match anchors, search within one work, explicit metadata-only/body-only/combined modes.

Body search does not require a globally shared body cache. Permission checks apply before counts, facets, snippets, and result serialization. Inaccessible content must not leak through counts or autocomplete.

Sanitize highlighting output. Strip executable markup before indexing. Index revision state so stale snippets can be invalidated.

## 15.10 Ranking

Only primary tags add tag-ranking boosts. Secondary tags remain filterable. Exact search is not taste-steered. Document scoring differences between database backends while keeping filtering and authorization consistent.

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

## 16.2 Administrator taste profile

Explicitly selected taste-source profile. Exclude moderation sessions, troubleshooting, import tests, accidental opens, activity marked private-from-learning.

Signals: explicit likes/dislikes, selected bookmarks, private ratings, optional completion events, admin seed prompts, wishlist items.

Long-term and recent profiles:

```text
profile = 0.65 × long_term + 0.35 × recent
```

Configurable decay and versioning.

## 16.3 Influence layer

Reference scoring:

```text
score = 0.65 × user_relevance
      + 0.15 × administrator_affinity
      + 0.10 × bridge_relevance
      + 0.10 × quality_signals
```

Initial values are tunable, not universal guarantees.

Additions: relevance floor, author concentration limits, repetition limits, negative-feedback cooldowns, diversity reranking, completion preference, positivity ratio consideration.

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

Turning it off removes administrator influence from candidate generation, ranking, reranking, recipes, dashboard widgets, prompts, challenges, notifications, and cached recommendations.

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

**Scripting is not part of recipes.** Users who want trigger-based automation build extensions in the WASM sandbox (Milestone 21). This is a deliberate security choice: declarative configuration cannot exfiltrate data or execute arbitrary code.

## 16.8 Widget-composed dashboard

First-party widget registry and default layout factory.

Initial widgets: continue reading, recent library updates, import progress, saved views, drafts, followed authors, selected recommendation engine, community subscriptions, optional writing opportunity, watched authors, cheer received (for authors), positive feedback received (for authors), reading goal progress.

Add/remove, reorder, resize within accessible constraints, per-pseud layouts, mobile adaptation, reset, safe mode.

Dashboard is not the only route to essential features. Third-party widgets use extension permissions and bounded data APIs.

Multiple dashboard views per pseud allow different layouts for different purposes (reading, writing, moderating).

## 16.9 Automatic and admin-seeded writing opportunities

Generate optional trope combinations, weekend prompts, response-fic opportunities, challenges, "write next" suggestions.

**Admin seed prompts:** administrator plants specific prompt ideas into the public prompt pool, tagged as "instance prompt." Writers can respond. Distinct from algorithmic generation.

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

Controls: taste source, signal inclusion, learning pause, influence pause, recency, weights, profile history, reset and rollback, aggregate evaluation, recommendation cache invalidation, diversity budget tuning, temporary boosts, editorial pick slots.

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
- Diversity budgets are honored under load.
- Request candidates only scan the requesting user's own content.

---

# 17. Milestone 12: Comments, Forums, Groups, Messaging, and Presence

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

Represent variants through a shared configurable workflow.

**Finished-work reading challenges:** "Read 5 completed fics under 10k words this month." Encourages completed-work reading.

Reading challenge completion is private and cosmetic. No XP, credits, or trust rewards.

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

Wishlist voting surfaces demand. Vote counts are visible to all users and to potential fulfillers. Votes do not obligate authors.

## 18.6 Editorial curator picks

Trusted curators (and the administrator) may feature works with short rationales in the dedicated discovery slot (Milestone 16).

## 18.7 "Best of" community-voted lists

Periodic community votes for completed works by category (fandom, mood, length, etc.). Separate from algorithmic trending. Voting requires trust level and completes with quorum review.

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

## 19.2 Effects

Trust may increase rate limits, batch sizes, proposal eligibility, curator eligibility, extension resource ceilings, gift/bounty limits, moderation queue eligibility, wishlist claim priority, invite-code issuance quota.

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

DMCA workflow respects legal requirements while preventing abuse of takedown mechanisms for harassment.

## 19.12 Public modlog

Publish redacted decision summaries, not private evidence.

Do not expose hidden pseud linkage, private messages, child-related evidence, reporter identity, source credentials, sensitive search or reading history, positivity classifier scores, individual shadowban targets, DMCA claimant identities.

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

Do not reward raw reading surveillance, posting volume, sanctions issued, positive star ratings, or maintaining streaks.

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

## 20.6 Subscriptions

Optional configurable reference plans:

- **Reader:** €3/month. Higher credit regeneration, larger download quotas, AI translation quota, ad-free supporter status.
- **Author:** €6/month. All Reader benefits plus larger publishing quotas, priority queue slots, AI-assisted comment classification for own works, cross-posting quota.
- **Curator:** €12/month. All Author benefits plus larger extension resource ceilings, priority marketplace review, larger batch import quotas.
- **Patron:** €25/month. All Curator benefits plus custom recognition, direct support channel.

No unlimited compute, no purchased trust, no search-ranking advantage, no moderation authority, no positivity filter bypass.

## 20.7 Marketplace revenue

Paid extensions and themes generate revenue split with developers. Configurable reference split of 85/15 (developer/platform). Operator handles tax and invoicing setup.

Marketplace ratings and reviews do not affect account trust. Popular extensions do not receive preferential trust or moderation authority.

## 20.8 Webhook handling

Signature verification, event-ID storage, idempotency, out-of-order handling, reconciliation.

## Acceptance

- Concurrent spending cannot overdraw.
- Retried webhooks do not duplicate grants.
- Failed jobs release holds.
- Standard jobs execute under paid load.
- Billing-disabled operation retains the core archive.
- Credits and monetary marketplace revenue remain distinct ledgers.
- Subscription tiers do not grant ranking, trust, or moderation authority.
- Wishlist bounties escrow correctly.

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

Support: mention alerts, replies (positive only by default), source-update notices, credential expiry, batch completion, digests, delivery failures, positive feedback received, cheer received, wishlist fulfillment.

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

Open Graph, Twitter/X cards, and embed previews for every public work URL with title, author, summary, fandom, cover image.

Cover images use content-addressed storage, permission checks, and size limits.

## Acceptance

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

## 25.2 Security tests

Cover stored XSS, CSRF, SSRF and DNS rebinding, credential forwarding across redirects, file traversal, malicious archives and decompression bombs, object-level authorization, pseud isolation, search snippet and count leakage, shared-cache existence leakage, session/token revocation, credit races, webhook replay, plugin exhaustion, CSS resource exfiltration, private cache leakage, bot channel-context errors, email relay abuse, feed-token logging, retention and deletion failures, positivity filter bypass attempts, translation permission bypass, extension purchase entitlement bypass, shadowban visibility leaks, DMCA workflow abuse.

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

---

# 26. Route and API Ownership

| Surface | Frontend routes | API owner |
|---|---|---|
| Discovery | `/`, `/discover`, `/blind-date`, `/dashboard`, `/surprise` | discovery |
| Fandom pages | `/fandoms/:id` | discovery |
| Recipes | `/recipes/*` | discovery/extensions |
| Search | `/search`, `/search/advanced`, `/find-fic` | search |
| People | `/people`, `/u/:handle` | identity/content |
| Reader | `/works/*`, `/series/*` | content/library |
| Writing | `/write/*` | content |
| Imports/library | `/library/*` | imports/library |
| Watches | `/library/watches` | imports/library |
| Source status | `/sources`, `/sources/:id` | imports |
| Cross-posting | `/write/:id/cross-post` | imports |
| Community | `/forum/*`, `/groups/*`, `/messages/*` | community |
| Events | `/challenges/*`, `/requests/*`, `/wishlists/*` | community |
| Translation | `/translations/*` | translation |
| Governance | `/trust/*`, `/moderation/*`, `/curation/*`, `/appeals/*` | governance |
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

## 28.10 Deliberately not adopted

- **XP and level progression.** No 100-level system, no XP bars, no rank-gated features. Gamification uses flat credit rewards, quality multipliers, and badges — not cumulative progression ladders.
- **Volume-based leaderboards.** No "most words written," "most posts," or "most kudos given" categories. Leaderboards reward completion, quality, and contribution.
- **Reputation scores.** No visible score derived from activity volume that determines trust or governance authority. Trust requires reviewed conduct.
- **Coercive streak mechanics.** No guilt-framed loss notifications, no streak-length credit multipliers, no "your streak will break" push notifications. Streaks are cosmetic with optional freezes.
- **Purchased trust or governance authority.** Credits buy compute priority, not moderation power or search ranking.
- **Visible admin taste influence.** The demand multiplier affecting author credits is never labeled, broken down, or hinted at in any user-facing surface. Authors see quality bonuses based on reader behavior only.
- **A promised source count.** Counts follow verified adapter support.
- **A permanent archive of every fetched body.** Shared infrastructure may deduplicate; public preservation requires explicit workflow.
- **Credentials as an SSRF exception.** Internal-network integrations require separate configuration.
- **Popularity-based moderation verdicts.** Process feedback is advisory.
- **Invasive or exclusionary bot detection.** No default fingerprinting, blanket headless-browser bans, compulsory JavaScript for reading.
- **Coercive reading analytics.** No mandatory streaks, loss warnings, public reading leaderboards, or rewards tied to reading behavior.
- **Automatic publication of AI outputs.** All translations, suggestions, and generated content require review.
- **Purchased visibility or authority.** No paid ranking, trust, or moderation.
- **Silent public visibility of hidden comments.** If a comment is hidden from the author, it is hidden from the public work page.
- **Scripted recipes.** Recipes are declarative configuration; scripting lives in the WASM extension sandbox.
- **Multi-tenancy.** One instance per deployment. Multiple communities run multiple instances.
- **Community suggestions of other users' social links.** Only the author adds their own social links.
- **NodeBB import.** Not applicable to a new platform.
- **Author social proposals as a public queue.** Suggestions may be sent privately to the author, not published.

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

---

The resulting project should be judged by these working behaviors—not by the number of screens, lines of code, imported feature names, or claims in a README.
