# Lorehaven: Revised Implementation Specification

This is a dependency-ordered specification for building the entire platform—not a prototype. It defines architecture, data structures, workflows, implementation milestones, and verification requirements.

It incorporates compatible FicNexus features while retaining Lorehaven’s privacy, safety, governance, and operational principles.

The three foundational priorities are:

1. **Reliable importing and private-library management.**
2. **Safe, low-friction writing and publishing.**
3. **Excellent reading, downloading, and offline access.**

Community, recommendations, governance, credits, and extensions build on those foundations.

The administrator’s tastes remain private. They influence discovery automatically through an opt-out recommendation layer; the administrator does not have to curate lists or publicly explain personal preferences.

This document specifies intended behavior. It does not claim that any feature, adapter, integration, or performance target has already been implemented or verified.

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

Importer support must use the same distinction. Never count an adapter skeleton as a supported source.

Documentation in an older project is evidence of intended behavior, not proof that the behavior works in Lorehaven.

## 1.3 Build vertical slices

Complete small end-to-end workflows before expanding them.

For example:

> Create draft → save chapter → publish → read → edit → read updated version.

And:

> Paste source URL → preview → import privately → read → download → read offline.

Do not build fifty backend endpoints before connecting the first frontend page.

## 1.4 Use architectural decision records

Store decisions in:

```text
docs/adr/0001-stack.md
docs/adr/0002-content-model.md
docs/adr/0003-pseud-isolation.md
docs/adr/0004-import-storage-and-deduplication.md
docs/adr/0005-source-credentials.md
docs/adr/0006-cross-source-identity.md
docs/adr/0007-analytics-and-retention.md
...
```

Each record contains:

- Problem.
- Decision.
- Alternatives.
- Consequences.
- Conditions that would justify revisiting it.

## 1.5 Features do not override foundational policies

Every added feature must preserve:

- Private-library isolation.
- Pseud compartmentalization.
- Content eligibility enforcement.
- Meaningful recommendation opt-out.
- Free core reading, writing, and publishing.
- Bounded resource use.
- Accessible non-gamified operation.
- Honest verification claims.

No feature may introduce a hidden exception to these rules.

## 1.6 Requirements are traceable

Maintain `docs/requirements.csv` with:

```text
requirement_id
description
milestone
backend_owner
frontend_surface
privacy_class
acceptance_test
verification_status
documentation_path
```

Features described as optional integrations must still have working disabled, unavailable, misconfigured, and failed states.

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
| SQLite search | FTS5 |
| PostgreSQL search | Native full-text search |
| Browser testing | Playwright |
| Frontend unit tests | Vitest |
| WASM execution | `wasmi` initially |
| Reverse proxy | Caddy |
| Service management | systemd |
| Default storage | Local filesystem |
| Optional billing | Stripe adapter, with billing-disabled operation |
| Optional semantic search | Provider interface; local or external implementation |

Pin compatible versions and commit lockfiles.

### Why Rust

Rust satisfies the native-compilation, memory-efficiency, expressive-type-system, and ARM64 requirements.

### Why Svelte and TypeScript

This combination keeps interactive frontend code reasonably concise and works well with browser APIs. Production does not require a Node.js server.

### Why a modular monolith

It simplifies:

- Local development.
- Transactions.
- Deployment.
- Backups.
- Debugging.
- Resource control.

A separate worker process is an optional operating mode of the same executable, not a separate service architecture.

## 2.2 Runtime components

```text
Browser / Installed PWA / Authorized API Client
                       |
                     HTTPS
                       |
                     Caddy
                       |
              Lorehaven executable
                ├── HTTP/API
                ├── Embedded frontend
                ├── Public page rendering
                ├── Job scheduler
                ├── Import workers
                ├── Export workers
                ├── Search indexing
                ├── Notification delivery
                └── Optional extension workers
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
- AI providers.
- ActivityPub servers.
- Chat platforms.
- Redis.

Core reading, writing, publishing, and private importing must remain functional without optional cloud services.

Redis is an optional transport and coordination optimization. Durable application state must not depend on Redis pub/sub delivery.

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
│   ├── governance/
│   ├── economy/
│   ├── extensions/
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
│   ├── importers/
│   ├── documents/
│   ├── search/
│   └── malicious-inputs/
├── scripts/
├── packaging/
│   ├── systemd/
│   └── caddy/
└── docs/
    ├── adr/
    ├── tutorial/
    ├── help/
    ├── operator/
    ├── api/
    ├── verification.md
    └── requirements.csv
```

Do not create a crate for every small feature. These boundaries are the maximum initial decomposition; closely related modules can share a crate.

## 2.4 Distribution

Use AGPL-3.0-or-later for first-party application code, with compatible dependency licensing and documented third-party notices.

Include:

- License and copyright notices.
- A source-code link in the application.
- Operator guidance for distributing modified deployments.
- A software bill of materials or equivalent dependency inventory.

Imported fiction remains subject to its own rights and permissions. The application’s license does not license hosted or imported works.

---

# 3. Shared Engineering Conventions

## 3.1 IDs and timestamps

Use UUIDs for primary identifiers.

- PostgreSQL: native UUID columns.
- SQLite: consistently encoded UUID text or 16-byte blobs.
- Public API: UUID strings.
- Timestamps: UTC internally, RFC 3339 in APIs.
- Display times in the user’s locale.

Do not expose sequential account IDs or account ownership through public URLs.

## 3.2 Money, credits, and counts

- Money: integer minor currency units.
- Credits: integer units.
- Word counts: nonnegative integers.
- Never use floating-point values for financial balances.

Statistical estimates and recommendation scores may use floating-point values, with documented interpretation.

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

Important error codes include:

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
```

Use `404` rather than revealing the existence of inaccessible private objects where appropriate.

Translate user-facing messages without changing machine-readable codes.

## 3.4 Concurrency

Every editable resource gets a monotonically increasing `version`.

Updates include:

```json
{
  "expected_version": 12,
  "changes": {}
}
```

Return `409 REVISION_CONFLICT` if the version differs.

Never silently overwrite another device’s or coauthor’s work.

## 3.5 Authentication

Use opaque server-managed sessions in secure cookies.

Cookie settings:

- `HttpOnly`.
- `Secure` in production.
- Appropriate `SameSite`.
- Narrow path/domain.
- Explicit expiry and revocation.

Use CSRF protection for state-changing cookie-authenticated requests.

API integrations use separately issued, scoped tokens whose hashes are stored server-side.

## 3.6 Authorization

Every resource operation follows:

```text
authenticate or establish anonymous actor
→ resolve active pseud where applicable
→ load resource
→ evaluate policy
→ perform operation
→ record required audit event
```

Put authorization rules in policy functions, not scattered frontend checks.

```rust
fn can_edit_work(
    actor: &Actor,
    work: &Work,
    contributors: &[Contributor],
) -> Decision;
```

Frontend visibility is convenience, not security.

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

Unlisted content is not private or encrypted.

Cache keys, search indexes, analytics, notifications, and background jobs must preserve these classifications.

## 3.8 Rate limiting and abuse controls

Use layered limits rather than one global IP bucket:

- Account or API token.
- Anonymous first-party client identifier, where appropriate.
- IP/network ceiling.
- Endpoint/resource class.
- Source domain.
- Concurrent job count.
- Expensive operation budget.

An anonymous client identifier is untrusted and resettable. It must not permit bypassing aggregate network limits.

Avoid punishing an entire campus or household for one client where possible. Do not claim that `(ip, client_id)` alone solves abuse.

Trust forwarded IP headers only from configured reverse proxies.

---

# 4. Database Plan

Use database-specific migrations and repository implementations where SQL differs. Do not assume SQLite and PostgreSQL are interchangeable.

Run integration tests against both from the beginning.

## 4.1 Shared table conventions

Unless inappropriate, mutable tables contain:

```text
id
created_at
updated_at
version
```

Index:

- Foreign keys used in lookups.
- Owner plus creation/update time.
- Status plus next-processing time for jobs.
- Unique normalized handles.
- Canonical source identifiers.
- Composite relationship/filter keys.
- Search document scope and revision.

Use JSON for flexible documents and extension manifests, not as a substitute for searchable relationship data.

For every migration, document deletion and retention behavior.

## 4.2 Identity tables

| Table | Important fields |
|---|---|
| `accounts` | status, email, email_verified_at, age_state |
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
| `mutes` | pseud_id, target_type, target_id |
| `api_tokens` | account_id, acting_pseud_id, token_hash, scopes, expiry |
| `integration_authorizations` | account_id, client_id, granted_scopes, expiry, revoked_at |

Only specifically authorized staff may retrieve private pseud ownership.

## 4.3 Content and cross-source identity tables

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
| `story_identities` | canonical bibliographic metadata, visibility, status |
| `story_identity_members` | identity_id, work_id/external_record_id, edition_relation |
| `identity_merge_proposals` | proposed members, evidence, status |
| `identity_merge_history` | previous identities, resulting identity, decision_reference |

The structured editor document is the source of truth. Sanitized HTML and plain text are derived representations.

A `story_identity` groups confirmed editions or cross-postings. It does not own their content or grant access to them.

Do not automatically equate:

- A translation and its original.
- A rewrite and its earlier edition.
- A sequel and its predecessor.
- Two works sharing a title.
- Matching author labels on different sites.

These may require typed relations rather than identity merges.

## 4.4 Private import and storage tables

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

Do not reuse public `works` ownership semantics for external copies.

Public source metadata may be shared only when it is genuinely public. Credentialed or restricted-source metadata must be kept in an appropriately private record.

Physical deduplication must never imply shared authorization.

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
```

Important constraints:

- Alias uniqueness is scoped by type and namespace.
- Relationship participant sets have a stable canonical signature.
- Character attributes belong to an identified work-character assertion.
- Relationship prominence belongs to the work’s assertion.
- Spoiler status belongs to the assertion or displayed metadata item.
- Merges preserve redirects and history.
- Suggestions do not become author assertions without the appropriate approval.

## 4.6 Other entity groups

| Module | Tables |
|---|---|
| Library | bookmarks, notes, shelves, shelf_entries, reading_progress, reading_events, reading_aggregates, saved_searches |
| Reader feedback | ratings, reviews, review_revisions, work_metric_aggregates |
| Jobs | jobs, job_attempts, job_events, outbox_events |
| Search | search_documents, search_index_state, search_demand_aggregates |
| Community | comments, reactions, follows, groups, memberships, boards, topics, posts, polls, poll_votes |
| Forum depth | topic_read_states, topic_tags, topic_tag_assignments, watch_preferences, mention_events |
| Messaging | conversations, conversation_members, messages, chat_rooms |
| Collections | collections, collection_roles, collection_submissions, collection_entries |
| Writing events | challenges, prompts, signups, assignments, claims, fulfillments, mentorships, sprints |
| Governance | trust_policies, trust_history, expertise, role_assignments, reports, cases, proposals, votes, sanctions, appeals, audit_events, process_feedback |
| Economy | wallets, ledger_transactions, ledger_entries, credit_holds, subscriptions, payment_events, bounties |
| Extensions | packages, package_versions, manifests, installations, grants, reviews, approvals, execution_usage, revocations, entitlements |
| Discovery | user_preferences, taste_profiles, taste_profile_versions, permitted_signals, exposure_events, aggregate_affinities, similarity_suggestions, similarity_votes, recommendation_recipes, recipe_versions |
| Interface | dashboard_layouts, widget_instances, user_locale_preferences |
| Integrations | notifications, notification_preferences, push_subscriptions, feed_tokens, federation_actors, federation_deliveries, delivery_addresses, bot_links, translation_jobs, translation_reviews |
| Operations | aggregate_site_metrics, security_events, retention_runs |

Do not create every table in Milestone 0. Introduce each schema through its owning vertical slice.

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

Commands:

```text
lorehaven serve
lorehaven worker
lorehaven migrate
lorehaven seed --development
lorehaven doctor
```

Configuration precedence:

```text
command-line argument
→ environment variable
→ configuration file
→ documented default
```

Never log secrets, full authenticated URLs, private feed tokens, or source credential material.

## Acceptance

- A clean checkout builds.
- SQLite and PostgreSQL startup both work.
- A frontend page loads from the Rust executable.
- `/health/live` checks process liveness.
- `/health/ready` checks essential dependencies.
- Production startup rejects unsafe development configuration.
- OpenAPI output is generated from implemented endpoints, not aspirational examples.

## Tutorial

Explain the browser/backend boundary, configuration, migrations, embedded assets, and API contract.

---

# 6. Milestone 1: Design System, Navigation, Localisation, and Help

## 6.1 Reusable components

Implement:

- Buttons and links.
- Inputs and labels.
- Selects and comboboxes.
- Dialogs and drawers.
- Tabs.
- Pagination.
- Toasts.
- Error summaries.
- Loading skeletons.
- Work cards.
- Metadata chips.
- Identity switcher.
- Empty-state panels.
- Job-progress panel.
- Source-health badge.
- Visibility selector.
- Permission-request panel.

Use design tokens:

```text
color.background
color.surface
color.text
color.muted
color.primary
color.danger
space.*
radius.*
font.interface
font.reader
```

Visual direction:

- Warm neutral surfaces.
- Deep plum accent.
- Restrained teal secondary accent.
- Generous spacing.
- Light, dark, and system modes.

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

## 6.3 Command palette

Provide `Ctrl+K` / `Cmd+K` and a visible menu entry.

Initial commands:

- Search works.
- Paste URL to import.
- Open library.
- Resume reading.
- Create draft.
- Switch pseud.
- Open saved view.
- Open help.
- Change appearance.

Commands that mutate data still require their ordinary confirmation and authorization.

Do not intercept editor or assistive-technology shortcuts indiscriminately.

## 6.4 Interface localisation

Localise interface chrome, not just work metadata.

Requirements:

- Message catalogs from the beginning.
- Locale-aware pluralization.
- Date, number, and currency formatting.
- No sentence construction by concatenating translated fragments.
- Language selector independent of work-language preferences.
- Document `lang` and direction attributes.
- Layout support for longer translations and right-to-left text.
- Locale fallback without blank labels.

Initial release catalogs:

- English.
- Spanish.

Additional locales are supported through the same contribution workflow. Do not advertise a locale as complete until its catalog and critical journeys are reviewed.

## 6.5 Contextual help

Provide:

- Contextual `?` links.
- In-app help panel or modal.
- Direct links to the relevant documentation section.
- “Try it” links that prefill a form without submitting it.
- Searchable help.
- Help available without JavaScript where practical.

Help and tutorial material share stable topic identifiers. Do not copy divergent explanations into several places.

Never put credentials, private draft text, or sensitive search queries into help deep-link URLs.

## 6.6 Appearance modes

Provide two first-party layout presets:

- **Modern:** card-oriented discovery and contemporary navigation.
- **Archive:** compact metadata, list-oriented browsing, restrained decoration.

These are presets over shared components, not separate applications.

Reader typography remains independently configurable. Neither preset may hide safety controls, attribution, or essential metadata.

## Acceptance

- Keyboard operation and visible focus.
- Dialog focus trapping and restoration.
- Usability at 320 CSS pixels and 200% zoom.
- Reduced-motion support.
- Accessible error announcements.
- Critical journeys work in English and Spanish.
- Pseudolocalisation catches clipping.
- Appearance changes do not remove functionality.
- Help links remain valid in CI.

---

# 7. Milestone 2: Accounts, Pseuds, Privacy, and Age Policy

## 7.1 Implement

- Registration and login.
- Password reset.
- Email verification.
- Session listing and revocation.
- TOTP and recovery codes.
- Pseud creation and switching.
- Privacy settings.
- Block and mute primitives.
- Age-policy state.
- Scoped API tokens.
- Per-pseud learning and reading-history controls.

Use an established password-hashing implementation with reviewed parameters.

## 7.2 Pseud behavior

Each pseud has separate:

- Public profile.
- Works.
- Follows.
- Messages.
- Recommendation settings.
- Ratings and reviews.
- Public bookmarks.
- Reading history.
- Dashboard layout.
- Source credential grants.
- Notification preferences.

The account shares:

- Credentials.
- Security state.
- Private wallet.
- Trust eligibility.

Do not publicly reveal shared ownership.

A scoped integration token should bind to an explicitly selected pseud unless it genuinely needs account-level security functions.

## 7.3 Age implementation

Use a state machine:

```text
unknown
→ declared_minor
→ declared_adult
→ authorization_required
→ authorized_under_policy
→ restricted
```

Do not treat a self-declared adult as independently verified.

For a Spain-based operator:

- Anonymous reading of suitable public fiction remains available.
- Registered child participation must follow the configured legal basis and authorization policy.
- Spain generally uses 14 as the relevant threshold for a child’s own data-processing consent; this does not settle every age-related obligation.
- If an under-threshold authorization workflow has not been legally and operationally established, do not enable unrestricted child registration through a checkbox.

Avoid collecting full birth dates unless necessary.

## 7.4 Shared content eligibility

Implement:

```text
can_access_content(actor, content_rating, visibility, policy)
```

Use it for:

- Reader.
- Search and snippets.
- Downloads and email delivery.
- Feeds.
- Notifications.
- Recommendations.
- Direct API access.
- Extensions.
- Chat bots.
- Public statistics where content categories could expose restricted material.

## 7.5 Core API

All paths below use `/api/v1`.

```text
POST   /auth/register
POST   /auth/login
POST   /auth/logout
POST   /auth/password-reset
POST   /auth/password-reset/complete
GET    /auth/sessions
DELETE /auth/sessions/:id

GET    /pseuds
POST   /pseuds
PATCH  /pseuds/:id
POST   /pseuds/:id/activate

GET    /settings/privacy
PATCH  /settings/privacy
GET    /settings/content
PATCH  /settings/content

GET    /api-tokens
POST   /api-tokens
DELETE /api-tokens/:id
```

## Acceptance

- No cross-account pseud editing.
- No hidden linkage in public responses.
- Revocation takes effect.
- Frontend bypass cannot retrieve restricted content.
- Sensitive data is absent from logs.
- Minor-protective messaging defaults are persisted.
- Private history and source credentials remain compartmentalized after pseud switching.

---

# 8. Milestone 3: Drafts, Chapters, Publishing, and Revisions

## 8.1 First vertical slice

1. Create draft.
2. Enter title.
3. Add chapter.
4. Save.
5. Preview.
6. Publish.
7. Read publicly.
8. Edit and republish.

## 8.2 Work states

```text
Lifecycle:
draft | scheduled | published | withdrawn | deleted

Visibility:
public | unlisted | restricted

Completion:
in_progress | complete | hiatus | abandoned
```

Explain that unlisted content can be accessed by anyone with the URL who satisfies applicable restrictions.

## 8.3 Editor

Restricted Tiptap schema:

- Paragraphs.
- Headings.
- Emphasis.
- Strong text.
- Lists.
- Blockquotes.
- Links.
- Scene breaks.

No arbitrary HTML, scripts, iframes, or custom embedded objects initially.

Autosave:

```text
debounce
→ local recovery save
→ versioned server update
→ visible saved/saving/offline/conflict state
```

Preserve local text when the server rejects a save.

Imported DOCX, HTML, EPUB, and Markdown content must be converted into the same restricted schema before becoming an editable draft.

## 8.4 Publishing transaction

In one transaction:

- Validate required metadata.
- Verify contributor permissions.
- Update publication state.
- Record publication event.
- Insert outbox events for notifications and indexing.

Do not send email inside the transaction.

Scheduled publication uses the same idempotent publication service.

## 8.5 API

```text
POST   /works
GET    /works/:id
PATCH  /works/:id
POST   /works/:id/publish
POST   /works/:id/withdraw

POST   /works/:id/chapters
PATCH  /chapters/:id
POST   /works/:id/reorder-chapters
GET    /chapters/:id/revisions
POST   /chapters/:id/restore-revision

POST   /works/:id/contributors/invitations
PATCH  /works/:id/contributors/:pseudId
```

## Acceptance

- Concurrent edits return conflicts.
- Revision restoration creates a new revision.
- Public readers never receive unpublished revisions.
- Repeated publication with one idempotency key does not duplicate notifications.
- Pseud switching does not change ownership.
- Invitations identify the exposed pseud.
- Withdrawn or newly restricted content disappears from public indexes and caches.

---

# 9. Milestone 4: Reader, Ratings, History, and Work Pages

## 9.1 Routes

```text
/works/:id
/works/:id/chapters/:chapterId
/works/:id/download
/works/:id/comments
/works/:id/reviews
/series/:id
```

Private imported content uses authenticated library routes rather than becoming public through these routes.

## 9.2 Reader features

- Chapter navigation.
- Whole-work mode.
- Table of contents.
- Typography settings.
- Light, dark, and sepia reader themes.
- Width and line-height controls.
- Distraction-free mode.
- Spoiler reveal.
- Progress.
- Private notes.
- Search within the current work.
- Reading-time estimates.
- End-of-work actions.

Long works must not require rendering every paragraph at once.

## 9.3 Progress

Store:

```text
work_or_library_item_id
chapter_id
content_revision
paragraph_anchor
position_fraction
updated_at
device_id
```

Use stable paragraph anchors where possible, with approximate fallback after content changes.

When devices disagree, present a choice rather than always taking the furthest position.

## 9.4 Ratings and reviews

Support:

- Optional 1–5-star personal rating.
- Optional written review.
- Spoiler marking.
- Edit and delete.
- Independent rating/review visibility.
- Reporting and blocking.
- Inclusion in personal discovery only under the relevant learning setting.

Defaults:

- Ratings are private.
- Written reviews remain private until explicitly published.
- Public review identity is the active pseud.
- Private ratings never contribute to public averages.

Public aggregate ratings:

- Include only explicitly public ratings.
- Display the count and aggregation method.
- Use a minimum publication threshold.
- Are not a default ranking signal.
- Are not applied to authors as a reputation score.

Work owners may disable display of public rating aggregates on their work pages. This does not erase readers’ private ratings.

Reviews are distinguished from conversational comments.

## 9.5 Reading history and personal analytics

Implement:

```text
/library/history
/library/reading-stats
```

History contains:

- Recently opened works.
- Resumable works.
- Explicitly finished works.
- Relevant timestamps.
- Removal controls.

Settings separately control:

- Progress synchronization.
- Detailed history retention.
- Personal aggregate statistics.
- Recommendation learning.

Disabling learning must not disable resume.

Personal analytics may include:

- Works marked finished.
- Chapters read.
- Estimated words read.
- Approximate reading time.
- Optional personal reading-day streak.

Do not count a chapter open as proof that every word was read. Label inferred measures as estimates and distinguish them from explicit completion.

Streaks:

- Are private and opt-in.
- Grant no trust, credits, or access.
- Produce no loss warnings or compulsory reminders.

Provide pause, clear, and configurable retention controls.

## 9.6 Reading-time estimates

Use word count and configurable reading speed as the baseline.

A dialogue-density adjustment may be added only after benchmarking and documentation. Display an approximate value or range rather than false precision.

Support language-sensitive tokenization where available and document fallback behavior.

## 9.7 Views and hit counts

Implement clearly defined aggregate view counts.

Rules:

- Deduplicate repeated opens within a documented window.
- Filter known automated traffic where feasible.
- Do not claim perfect human uniqueness.
- Do not publish visitor identities.
- Avoid long-lived fingerprinting.
- Exclude private imports from public counts.
- Make public work-level count display configurable by the author.

Metric collection and public display are separate decisions.

## 9.8 End-of-work page

Show:

- Appreciation.
- Bookmark.
- Rating or review.
- Comment.
- Mark finished.
- Next in series.
- Another work by the author.
- Configurable next reads.
- Optional writing opportunity.

## Acceptance

- Progress survives refresh.
- Content updates do not crash resume.
- Sanitized content cannot execute scripts.
- Public eligible work pages have meaningful sharing metadata and useful non-JavaScript content.
- Clearing history removes retained detailed events.
- Private ratings never appear in public APIs or public averages.
- Repeated refreshes do not inflate views without bound.
- Reading estimates are not presented as measured attention.

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

Fields:

```text
kind
owner_account_id
owner_pseud_id
resource_class
priority_class
payload
idempotency_key
attempt_count
run_after
lease_owner
lease_expires_at
progress
error_code
```

Batch jobs contain durable child-job references and resumable enumeration cursors.

## 10.2 Worker behavior

- Transactional job claims.
- Leases and renewal.
- Backoff for transient errors.
- Bounded attempts.
- Abandoned-job recovery.
- Cancellation between safe checkpoints.
- Idempotent handlers.

Delivery is at least once.

## 10.3 Storage

Use generated or content-addressed storage keys, never user-supplied paths.

Implement:

- Temporary upload directory.
- Atomic finalization.
- Checksums.
- Logical and physical storage accounting.
- Quotas.
- Reference-aware orphan cleanup.
- Disk-pressure warnings.
- Safe path resolution.
- Extraction limits.

Define whether quotas charge logical ownership or physical bytes. Users must not infer another user’s holdings from quota discounts.

## 10.4 Revision cache and physical deduplication

Adopt shared storage infrastructure, not a universal publicly searchable archive of everything imported.

Separate:

1. **Authorized user snapshots:** durable copies referenced by private library items.
2. **Temporary fetch cache:** reusable bytes under a documented scope and expiry.
3. **Approved public preservation corpus:** content explicitly authorized for public archival use.

Cache keys include:

```text
source identity
source revision or validated fingerprint
adapter extraction version
security scope
```

Security scopes distinguish public cache-eligible fetches from credentialed and private imports.

Requirements:

- Credentialed fetches are private-scoped by default.
- A checksum is not an authorization credential.
- Blob lookup by arbitrary hash is not a public API.
- Deduplication does not expose whether another user has a work.
- Cache reuse does not bypass current source-access policy.
- A shared byte store does not automatically create a shared search corpus.
- Retained cache entries expire independently of user-owned snapshots.
- Deletion removes references and eventually collects unreferenced blobs.
- Public preservation is an explicit workflow, never a side effect of importing.

Automatic storage growth is bounded by quotas and retention, not “keep every scraped body forever.”

## 10.5 Secret encryption

Provide authenticated encryption for recoverable integration secrets.

Use a reviewed AEAD implementation, such as AES-256-GCM, with:

- Unique nonces.
- Authenticated context binding.
- Versioned ciphertext.
- Key identifiers.
- Key rotation.
- Keys stored outside the database.
- Backup and recovery documentation.

Encryption at rest does not protect against a fully compromised running server. State that limitation.

Jobs store secret references, not plaintext credentials.

## Acceptance

- Worker death does not lose jobs.
- Parallel workers do not publish or charge twice.
- Low disk produces actionable errors.
- Traversal and archive-bomb attempts fail safely.
- Errors are redacted.
- Cross-user cache reuse cannot bypass authorization.
- Credential ciphertext cannot be decrypted without the configured key material.
- Cache eviction does not delete a referenced private snapshot.

---

# 11. Milestone 6: Imports, Source Credentials, Batches, and Preservation

## 11.1 Adapter abstraction

```rust
trait SourceAdapter {
    fn identify(&self, url: &Url) -> Option<SourceKey>;

    async fn fetch_metadata(
        &self,
        request: ImportRequest,
    ) -> Result<SourceMetadata, ImportError>;

    async fn fetch_chapter(
        &self,
        chapter: SourceChapter,
    ) -> Result<ImportedChapter, ImportError>;
}
```

Add optional capability interfaces for:

- Bibliography enumeration.
- Update checking.
- Source authentication.
- Conditional requests.
- Source-specific revision identifiers.

Capability absence must be visible. Do not simulate bibliography support by guessing URLs.

Adapters use the shared safe fetcher.

## 11.2 Destinations

Require an explicit choice:

- Private library.
- Own draft.
- Republication with permission.
- Approved preservation batch, for authorized operators.

Default URL imports to private library.

A source login demonstrates access, not permission to republish.

## 11.3 Entry points

Provide:

- Paste-a-URL box on the home page and library page.
- `/library/imports/new`.
- File upload.
- Command-palette action.
- Drag-to-bookmarks-bar bookmarklet.
- PWA share target where supported.

The bookmarklet:

- Transfers only the selected page URL.
- Does not scrape cookies or page content.
- Opens Lorehaven’s confirmation screen.
- Never imports or publishes through a state-changing GET.
- Warns about source URLs containing private tokens.

Prefer transient client-side transfer and immediate URL cleanup for sensitive source URLs. Redact importer entry URLs from access logs.

## 11.4 Workflow

```text
URL/file
→ source detection
→ source status and authentication check
→ metadata preview
→ destination selection
→ confirmation
→ queued import
→ chapter fetching
→ sanitation
→ duplicate/update review
→ completed library item
```

## 11.5 Safe fetching

The shared fetcher must:

- Reject unsupported schemes.
- Reject loopback, link-local, private-network, and metadata-service targets.
- Validate every redirect and connection destination.
- Address DNS rebinding.
- Limit bytes, time, redirects, decompression, and concurrency.
- Strip or proxy unsafe embedded resources.
- Avoid forwarding credentials to unrelated origins.
- Respect source rate limits and relevant retry instructions.
- Redact credential material.

A source credential does **not** justify an SSRF exception.

Any trusted administrative integration that needs an internal destination uses a separate explicitly configured capability, not a user-controlled importer escape hatch.

Do not implement CAPTCHA, paywall, or access-control circumvention.

## 11.6 Per-source credential vault

Support source authentication only for adapters with a documented authentication method.

Prefer source-issued tokens or scoped credentials where available. Password or session-cookie storage requires explicit consent and adapter-specific documentation.

Vault behavior:

- Pseud-scoped by default.
- Encrypted at rest.
- Plaintext never returned by an endpoint.
- Decrypted only for the required operation, for the shortest practical lifetime.
- Default expiry of 30 days or the source credential’s earlier expiry.
- Shorter configurable expiry.
- Immediate revocation.
- Re-consent for renewal.
- Origin-bound credential use.
- Audit events without secret contents.
- No automatic copying across pseuds.
- No credentials in job payloads, exports, logs, traces, or analytics.

Expired credentials pause affected jobs with an actionable status. Do not repeatedly retry authentication failures.

API:

```text
GET    /source-credentials
POST   /source-credentials
DELETE /source-credentials/:id
POST   /source-credentials/:id/test
```

Responses expose metadata only: source, label, expiry, status, and last successful use.

Deleting a credential does not automatically delete already imported copies. Explain both actions separately.

## 11.7 Initial formats and adapters

Implement local file import first:

1. Plain text.
2. HTML.
3. EPUB.
4. DOCX.
5. Markdown.

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
10. Perform live verification when permitted and available.
11. Record evidence and status.

Do not promise a source count.

## 11.8 Runtime source health

Track source health separately from verification status.

Health states:

```text
unknown | healthy | degraded | unavailable | paused
```

Distinguish failures:

- Source outage.
- Rate limiting.
- Authentication failure.
- Parser incompatibility.
- Policy restriction.
- Network failure.
- User-specific access denial.

Implement:

- Rolling outcome windows.
- Bounded retry.
- Per-domain concurrency.
- Circuit breakers.
- Operator pause/resume.
- Recovery probes.
- User-visible incident messages.
- Adapter-version attribution.

Do not label a whole source unavailable because one user’s credentials expired.

Public source status omits private import volumes, credential use, and user identities.

## 11.9 Author and bibliography batch import

Accept supported author, user, or bibliography URLs.

Workflow:

```text
identify bibliography
→ enumerate resumably
→ preview discovered works
→ select/filter
→ estimate size and limits
→ confirm destination
→ enqueue child imports
→ show per-work progress
```

Support:

- Select all or a subset.
- Exclude already imported works.
- Update existing items.
- Cancellation.
- Resume.
- Per-entry retry.
- Pagination and enumeration limits.
- Rate-limit-aware scheduling.

Bibliography importing does not bypass permission requirements or default to republication.

## 11.10 Cross-source identity

Distinguish:

- Duplicate import from the same source.
- Confirmed cross-posting.
- Different edition.
- Translation.
- Adaptation.
- Similar but unrelated work.

Users may privately group their own copies without changing the public catalog.

Public identity merges require evidence and review.

A unified story page may list public source editions and the viewer’s own private copies. It must never expose another user’s private holdings.

Merges:

- Preserve source records and provenance.
- Do not combine private notes or ownership.
- Do not grant access to another edition’s body.
- Are reversible.
- Preserve redirects and decision history.

## 11.11 Preservation and archive migration batches

Support Open Doors-style preservation workflows without implying affiliation.

Require:

- Authorized operator role.
- Import manifest.
- Source archive identity.
- Documented permission or other reviewed legal basis.
- Original author attribution.
- Source URLs and timestamps.
- Destination collection.
- Claiming and correction procedure.
- Dry-run report.
- Idempotent execution.
- Duplicate review.
- Batch rollback or withdrawal strategy.

Never assign imported authors to local accounts solely by matching names or email strings.

Preservation batches may initially remain private or review-only. Public release is a separate approval step.

## 11.12 Updates

Never overwrite imported copies destructively.

Create a new snapshot and preserve:

- Notes.
- Shelves.
- Bookmarks.
- Ratings.
- Progress where mappable.

Show review notices for removed, reordered, or substantially changed chapters.

## Acceptance

- Repeat imports avoid accidental duplicates.
- Failed chapter fetching resumes.
- External content never becomes public automatically.
- Malicious archives cannot escape extraction storage.
- Unsupported sources are excluded from support counts.
- Credentials cannot leak through redirects.
- Expiry pauses rather than loops jobs.
- Batch progress survives restart.
- Identity merging does not alter content permissions.
- Preservation dry runs do not publish anything.

---

# 12. Milestone 7: Exports, Device Delivery, and Offline Reading

## 12.1 Export order

Implement:

1. Plain text.
2. Sanitized standalone HTML.
3. Markdown.
4. EPUB.
5. PDF converter integration.
6. AZW3 converter integration.
7. MOBI converter integration.

Markdown export documents which rich-text features are simplified.

For converters:

- Discover availability at startup.
- Build fixed arguments.
- Avoid unsafe shell interpolation.
- Enforce time, memory, and output limits.
- Disable unavailable formats with installation guidance.
- Record converter version in verification evidence.

Format support and device-delivery support are separate claims.

## 12.2 Export behavior

Exports include, as applicable:

- Title and author attribution.
- Chapter ordering.
- Table of contents.
- Language.
- Source provenance.
- Edition or snapshot information.
- Applicable license or permission statement.
- User-selected typography where supported.

Private notes are excluded by default and require explicit inclusion.

Generated download URLs are authenticated or short-lived and scoped. Shared export caches preserve source and owner security boundaries.

## 12.3 EPUB validation

Validate:

- ZIP structure.
- MIME declaration.
- Package metadata.
- Navigation.
- Chapter order.
- Unicode.
- Attribution.
- Source provenance.

## 12.4 Send-to-Kindle and device email

Implement an optional export-delivery adapter using SMTP.

Initial workflow:

```text
add device address
→ verify address or complete documented ownership check
→ configure approved sender where required
→ choose work and format
→ review privacy notice
→ queue export and delivery
→ show delivery status
```

Requirements:

- Device addresses are private.
- Destination changes require appropriate confirmation.
- Rate and size limits.
- No arbitrary public mail relay.
- No untrusted user-controlled mail headers.
- Attachment and export limits.
- Bounce/failure handling where available.
- Revocation and address deletion.
- Explicit disclosure that the delivery provider receives the content.

Use a format supported by the destination’s current documented delivery workflow. Do not assume AZW3 or MOBI export implies acceptance by Kindle email delivery.

“SMTP accepted” is not “arrived on device.”

## 12.5 PWA

Implement:

- Web manifest.
- Icons.
- Service worker.
- IndexedDB schema.
- Download manager.
- Offline reader route.
- Update notification.
- Supported-platform share target.

Caching:

| Data | Strategy |
|---|---|
| Versioned assets | Cache-first |
| Public dynamic data | Network/revalidation |
| Explicitly downloaded works | IndexedDB |
| Private authenticated responses | No blanket shared caching |
| Reading progress | Local first, queued synchronization |

## 12.6 Offline privacy

On logout, offer removal of local private downloads, with a privacy-protective default on shared devices.

Explain:

- Browser storage may be evicted.
- Downloaded copies cannot always be remotely revoked.
- Device access can expose offline content.
- Service-worker caching is not encryption.
- Account deletion on the server does not guarantee deletion from disconnected devices.

## Acceptance

- Downloads open without a network.
- Missing chapters show actionable offline states.
- Interrupted downloads recover.
- Quota errors preserve existing downloads.
- IndexedDB upgrades are tested.
- Private data does not cross accounts or pseuds.
- Foreground synchronization works without Background Sync.
- Device delivery cannot be used as an open relay.
- Unavailable converters are honestly disabled.

---

# 13. Milestone 8: Library, Saved Views, Bookmarks, and Updates

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
/library/views
/library/views/:id
/library/source-credentials
```

## 13.1 Core library features

- Shelves.
- Reading statuses.
- Private tags.
- Bookmark notes.
- Batch actions.
- Source filters.
- Update checking.
- Duplicate review.
- Cross-source edition grouping.
- Storage usage.
- Import provenance.
- Source health.
- Credential-expiry notices.

Default bookmarks to private.

Deleting a shelf does not delete its works.

## 13.2 Saved searches and named views

A saved view contains:

```text
name
versioned_query_ast
sort
scope
display_preferences
pinned
visibility
```

Support:

- Save current search.
- Rename.
- Duplicate.
- Pin to navigation or dashboard.
- Edit through visual filters or query syntax.
- Delete.
- Optional public sharing of safe public-work views.

Private-library views are private. Public view serialization must not expose private shelf IDs, notes, reading state, or hidden pseud information.

Relative filters such as “updated in the last seven days” retain relative meaning.

After a search-schema upgrade, migrate the AST or show an explicit repair state.

## 13.3 Batch outcomes

```json
{
  "succeeded": ["..."],
  "failed": [
    {
      "id": "...",
      "code": "ACCESS_DENIED"
    }
  ]
}
```

## Acceptance

- Public bookmark lists exclude private entries.
- Removing an item distinguishes deleting a reference from deleting a private copy.
- Source update checking respects rate limits.
- Pinned views survive refresh and pseud switching correctly.
- Shared views cannot expose private filters.
- History and statistics deletion controls work independently of library ownership.

---

# 14. Milestone 9: Structured Taxonomy, Body Search, and Query Language

## 14.1 Character assertions

```text
Character A:
  prominence = protagonist
  roles = [mentor]
  attributes = [vampire, BAMF]
```

## 14.2 Relationship assertions

```text
Participants = [A, B]
Kind = romantic
Prominence = central
Dynamics = [enemies_to_lovers]
```

Support more than two participants.

## 14.3 Typed query AST

```text
And
Or
Not
Text
Phrase
WorkField
ExistsCharacter
ExistsRelationship
TagMatch
MetadataCompleteness
```

Example:

```json
{
  "and": [
    {
      "exists_character": {
        "character_id": "A",
        "prominence": ["protagonist"],
        "attributes_all": ["BAMF"]
      }
    },
    {
      "not": {
        "exists_relationship": {
          "participant_any": ["A"],
          "kind_any": ["romantic", "sexual"]
        }
      }
    }
  ]
}
```

Compile bound predicates into SQL `EXISTS` clauses. Never allow one character to satisfy another character’s attributes accidentally.

## 14.4 User-facing query language

Support:

- Quoted phrases.
- `AND`, `OR`, `NOT`.
- Parentheses.
- Leading `-` exclusions.
- Fielded search.
- Documented escaping.
- Autocomplete and error locations.

Initial fields:

```text
title:
author:
fandom:
character:
relationship:
tag:
summary:
body:
language:
status:
```

Examples:

```text
fandom:"Example Fandom" AND title:"winter"
```

```text
body:"we were never alone" -tag:"major character death"
```

```text
(author:"Writer A" OR author:"Writer B") AND status:complete
```

Define operator precedence and implicit conjunction explicitly.

The parser produces the same typed AST as the visual filter builder. Do not create separate semantics for text and form searches.

Advanced bound character predicates may use visual controls initially. The interface must show when a query contains predicates that cannot be represented by a simpler text shorthand.

Malformed syntax produces a helpful error, not a silently different query.

Never pass user query text directly as SQL or raw database-specific FTS syntax.

## 14.5 Metadata completeness

Negative filters support:

- Match declared metadata.
- Require sufficiently complete metadata.
- Include uncertain results separately.

Missing ship metadata is not proof that a story contains no ship.

## 14.6 Filters

Implement:

- Characters and prominence.
- Relationships and prominence.
- Relationship kinds and exclusions.
- Character attributes and roles.
- Fandom and crossovers.
- Completion.
- Word/chapter ranges.
- Rating and warnings.
- Language.
- Dates.
- Author.
- Collection.
- Series.
- Tropes.
- Settings.
- Vibes.
- Public works/private library scope.
- Read/unread.
- User mutes.

Bound query depth, clause count, result windows, and execution time.

## 14.7 Full-text body search

Index permitted body text for:

- Published eligible local works.
- The requesting pseud’s private imported copies.
- Explicitly authorized public preservation content.

Support:

- Phrase search.
- Prose and dialogue search.
- Chapter-level matches.
- Highlighted snippets.
- Jump-to-match anchors.
- Search within one work.
- Explicit metadata-only, body-only, or combined modes.

Body search does not require a globally shared body cache.

Permission checks apply before counts, facets, snippets, and result serialization. Inaccessible content must not leak through “three hidden matches” or autocomplete.

Sanitize highlighting output. Strip executable markup before indexing.

Index revision state so stale snippets can be invalidated after withdrawal, restriction, update, or deletion.

## 14.8 Ranking

Only primary tags add tag-ranking boosts. Secondary tags remain filterable.

Exact search is not taste-steered.

Database backends may differ in relevance scoring. Document those differences while keeping filtering and authorization semantics consistent.

## 14.9 Canonicalization and metadata correction

Tag workflow:

```text
proposal
→ review
→ quorum
→ apply
→ reindex
→ preserve redirect/history
```

Cross-source identity proposals use an equivalent evidence-based workflow.

Metadata correction supports:

- Title.
- Author attribution.
- Summary.
- Completion status.
- Language.
- Source mapping.
- Taxonomy assertions.

Rules:

- Authors control their authored work metadata, subject to clearly defined moderation policies.
- Curators cannot silently rewrite author text.
- Private imported metadata may be corrected locally without changing a shared record.
- Source updates do not silently discard local overrides.
- Every shared correction records provenance and history.

Auto-tag suggestions use deterministic rules first and optional AI later. They enter a review queue and are never presented as confirmed author declarations.

## 14.10 People directory

Implement `/people` with:

- A–Z browsing.
- Handle/display-name search.
- Fandom filtering.
- Author/reader role filters where voluntarily declared.
- Pagination.
- Discoverability controls.

Include only pseuds that permit directory listing. Do not infer fandom interests from private reading or imports.

## 14.11 Zero-result demand insights

Provide an optional privacy-preserving curator view of unmet search demand.

Requirements:

- Clear collection setting and policy.
- Exclude private-library and sensitive queries by default.
- Avoid storing raw queries when aggregate taxonomy buckets suffice.
- Redact likely personal data.
- Suppress low-count cohorts.
- Bound retention.
- No public list of individual searches.
- No automatic import, publication, or challenge creation.

Any external AI clustering of query text requires separate explicit consent and configuration.

Label clusters as observed search demand, not proof that content does not exist.

## Acceptance fixture

Include contrasting works:

- A protagonist/B vampire.
- A vampire protagonist.
- A supporting vampire.
- A/B central.
- A/B background.
- A with unknown relationship metadata.
- A with confirmed no romantic/sexual relationship.
- A private imported body containing a unique phrase.
- A withdrawn work containing the same phrase.

Test both databases for:

- Correct binding.
- Negative-filter semantics.
- Query parser round trips.
- Phrase search.
- Snippet sanitation.
- Permission-safe counts.
- Index invalidation.
- Query complexity limits.

---

# 15. Milestone 10: Discovery, Private Taste Influence, Recipes, and Dashboards

## 15.1 Baseline engines

Implement:

- Recent.
- Trending by time window.
- Content similarity.
- Also bookmarked.
- Similar by bookmarks.
- Blind Date.
- User-preference matching.
- Community-suggested similarity.

Each engine implements:

```text
generate_candidates(context, limit)
→ candidate IDs and baseline scores
```

All candidates pass shared eligibility rules.

Private bookmark and rating data are not silently pooled into collaborative signals. Use explicitly permitted signals, aggregation thresholds, and documented retention.

## 15.2 Administrator taste profile

Use an explicitly selected taste-source profile.

Exclude:

- Moderation sessions.
- Troubleshooting.
- Import tests.
- Accidental opens.
- Activity marked private-from-learning.

Signals:

- Explicit likes/dislikes.
- Selected bookmarks.
- Private ratings.
- Optional completion events.

Maintain long-term and recent profiles:

```text
profile = 0.65 × long_term + 0.35 × recent
```

Use configurable decay and versioning.

## 15.3 Influence layer

Reference scoring:

```text
score =
  0.80 × user_relevance
  + 0.12 × administrator_affinity
  + 0.08 × bridge_relevance
```

Initial values are tunable, not universal guarantees.

Add:

- Relevance floor.
- Author concentration limits.
- Repetition limits.
- Negative-feedback cooldowns.
- Diversity reranking.
- Completion preference.

## 15.4 Meaningful opt-out

Setting:

> Include this instance’s evolving discovery preferences alongside your own interests.

Turning it off removes administrator influence from:

- Candidate generation.
- Ranking.
- Reranking.
- Recipes.
- Dashboard widgets.
- Prompts.
- Challenges.
- Notifications.
- Cached recommendations.

Individual recommendations need not carry administrator-specific labels, but explanations must not be fabricated.

Private taste profiles are not returned to users, recipes, or plugins. External engines receive baseline context and have the host apply any permitted private influence afterward.

Public behavior may permit broad inference; do not promise mathematical secrecy. Avoid fine-grained explanation or score endpoints that reveal individual private signals.

## 15.5 Community similarity suggestions

Readers may suggest:

> If you liked this work, you may like that work.

Support:

- Optional rationale.
- Spoiler marking.
- Up/down usefulness votes.
- Reporting.
- Withdrawal.
- Duplicate consolidation.
- Eligibility and block enforcement.

These suggestions are separate from machine-generated similarity.

Votes:

- Count unique accounts internally.
- Do not reveal hidden pseud linkage.
- Affect this recommendation signal only.
- Grant no trust or moderation authority.

## 15.6 Recommendation recipe builder

A recipe is declarative configuration, not arbitrary code.

It may combine:

- Baseline engines.
- Weights.
- Filters.
- Diversity settings.
- Exclusions.
- Completion preference.
- Bounded boosts.
- Reranking options.

Support:

- Preview.
- Save privately.
- Publish.
- Install.
- Duplicate.
- Remix/fork.
- Version history.
- Reset.

Recipes cannot:

- Override eligibility.
- Re-enable opted-out administrator influence.
- Access hidden taste vectors.
- Search someone else’s library.
- Buy ranking.
- Grant execution permissions.

Public recipes must remove private object references. Installation validates schema and resource cost.

When extensions arrive, the recipe gallery integrates with the marketplace without requiring recipes to execute WASM.

## 15.7 Widget-composed dashboard

Provide a first-party widget registry and default layout factory.

Initial widgets:

- Continue reading.
- Recent library updates.
- Import progress.
- Saved views.
- Drafts.
- Followed authors.
- Selected recommendation engine.
- Community subscriptions.
- Optional writing opportunity.

Support:

- Add/remove.
- Reorder.
- Resize within accessible constraints.
- Per-pseud layouts.
- Mobile adaptation.
- Reset.
- Safe mode.

Do not make the dashboard the only route to essential features.

Third-party widgets later use extension permissions and bounded data APIs.

## 15.8 Automatic writing opportunities

Generate optional:

- Trope combinations.
- Weekend prompts.
- Response-fic opportunities.
- Challenges.
- “Write next” suggestions.

Use deterministic pools first. AI is optional.

Combine writer interests with instance affinity only when enabled.

Never invent human sponsors, commissions, or community demand.

## 15.9 Administration

```text
/admin/discovery
```

Controls:

- Taste source.
- Signal inclusion.
- Learning pause.
- Influence pause.
- Recency.
- Weights.
- Profile history.
- Reset and rollback.
- Aggregate evaluation.
- Recommendation cache invalidation.

## Acceptance

- Opt-out changes candidates and scores across all surfaces.
- Exact and chronological sorts remain exact.
- Missing profiles fall back to baseline.
- Blocked or ineligible content never enters displayed results.
- Minors receive policy-appropriate discovery rather than default behavioral steering.
- Recipes cannot bypass host policies.
- Dashboard reset works even with a broken widget.
- Taste alignment never affects governance or trust.

---

# 16. Milestone 11: Comments, Forums, Groups, and Messaging

## 16.1 Comments and reviews

Implement:

- Work/chapter comments.
- Replies.
- Appreciation.
- Spoiler formatting.
- Appropriate edit history.
- Author locking.
- Reporting.
- Block/mute enforcement.
- Review moderation under the same safety framework.

Limit nesting depth and flatten deeper replies clearly.

## 16.2 Forums

Implement:

- Categories.
- Boards.
- Topics.
- Posts.
- Polls.
- Reactions.
- Pins.
- Locks.
- Subscriptions.
- Pagination.
- Topic tags.
- Forum full-text search.
- Mentions.
- Read state and unread badges.

## 16.3 Read state

Store a per-pseud topic high-water mark using stable post ordering.

Behavior:

- Opening a topic marks only the content actually presented under the documented read policy.
- Paginated entry does not mark unseen later pages read.
- Writes are bounded, monotonic where appropriate, and idempotent.
- “Mark topic/category read” is explicit.
- Deleted or hidden posts do not break cursors.
- Counts exclude inaccessible categories.

## 16.4 Forum search and tags

Search includes:

- Topic titles.
- Post bodies.
- Category scope.
- Author.
- Topic tags.
- Dates.
- Safe highlighted snippets.

Permissions apply before counts and snippets.

Topic tags are distinct from fiction taxonomy but may share canonicalization infrastructure.

Support scoped tag aliases, merge history, and tag pages.

## 16.5 Mentions and notification preferences

Support `@handle` mentions with disambiguation.

Mention creation checks:

- Visibility.
- Blocks.
- Unsolicited-contact restrictions.
- Rate limits.
- Minor-protective policy.

Per-category/topic watch levels:

```text
muted | normal | tracking | watching
```

Notification settings cover:

- In-app.
- Email.
- Push.
- Immediate versus digest.
- Quiet periods.
- Followed topics.
- Mentions and replies.

Email digests are optional and unsubscribeable without requiring a login where safely possible.

## 16.6 Groups

Roles:

```text
owner | manager | member
```

Visibility:

```text
public | approval_required | private
```

## 16.7 Messaging

Implement:

- Conversation invitations.
- Direct messages.
- Group conversations.
- Chat rooms.
- Leave/mute/block/report.
- Unsolicited-message restrictions.
- Minor-protective defaults.

Persisted messages are authoritative. Live delivery is an optimization.

Do not claim end-to-end encryption.

## 16.8 Real-time delivery and catch-up

Use a common event envelope with durable cursors.

Support:

- WebSocket delivery.
- SSE read-only delivery where suitable.
- Polling fallback.
- Catch-up through an opaque `after` cursor.
- Duplicate suppression.
- Reauthorization on reconnect and catch-up.
- Gap recovery when a cursor expires.

Single-process operation uses in-process fan-out plus durable storage.

Optional Redis pub/sub may distribute events across processes. It is not the event history. When unavailable, degrade to supported local delivery or database-backed catch-up.

## 16.9 Scoped sanctions

Support category- or group-scoped:

- Posting timeout.
- Reply restriction.
- Topic-creation restriction.
- Access ban where policy permits.

Every sanction records:

- Scope.
- Reason.
- Start.
- Expiry.
- Issuer.
- Review reference.
- Appeal path.

Forum reputation points and posting-volume leaderboards are not implemented. Reliability, expertise, and staff roles remain separate.

## Acceptance

- Removed members lose access.
- Private topics cannot be fetched by ID.
- Read badges are accurate under pagination.
- Mentions do not bypass blocks.
- Digests omit newly inaccessible content.
- Reconnection does not duplicate messages.
- Redis failure does not lose persisted posts.
- Expired sanctions cease to apply.
- Reporting exposes only appropriately scoped evidence.

---

# 17. Milestone 12: Collections, Challenges, Requests, and Writing Events

## 17.1 Collections

```text
submitted → approved/rejected
approved → withdrawn/removed
```

Implement:

- Curator roles.
- Invitations.
- Submission approval.
- Sections and ordering.
- Work-owner withdrawal.
- Preservation-batch provenance links.

Collections do not grant permission to republish or expose private imports.

## 17.2 Challenges

Support:

- Signups.
- Prompt pools.
- Assignments.
- Claims.
- Deadlines.
- Reveal dates.
- Anonymous-until-reveal submissions.
- Fulfillment.
- Withdrawal.

Represent variants through a shared configurable workflow.

## 17.3 Mentorship

- Applications.
- Availability.
- Interest matching.
- Invitations.
- Session/task completion.
- Reporting.
- Private participation settings.

## 17.4 Sprints

- Start/end time.
- Optional shared room.
- Private/public counters.
- Manual word-count updates.
- No publication requirement.
- No compulsory streaks.

## 17.5 Requests and bounties

Implement request handling now. Attach credit escrow after the ledger milestone.

Distinguish:

- Search help.
- Recommendation request.
- Writing prompt.
- Commission or bounty, where enabled.
- Preservation request.

Do not convert zero-result search analytics into public requests without user action.

## Acceptance

- Challenge identities remain hidden until reveal.
- Collection ownership does not bypass permissions.
- Missed deadlines have defined outcomes.
- Withdrawals preserve required audit and attribution history.
- Generated events do not fabricate human sponsorship.

---

# 18. Milestone 13: Trust, Reports, Quorum, Appeals, and Process Feedback

## 18.1 Trust model

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

Higher levels require reviewed conduct, not merely point totals.

Core publishing and reading remain available at TL0.

## 18.2 Effects

Trust may increase:

- Rate limits.
- Batch sizes.
- Proposal eligibility.
- Curator eligibility.
- Extension quotas.
- Gift/bounty limits.

It does not automatically grant private-message access or administrator powers.

Credits, ratings, views, posting volume, reading streaks, and marketplace purchases do not buy trust.

## 18.3 Reports

```text
submitted
→ triaged
→ investigating
→ proposal
→ decision
→ appealed
→ closed/reversed
```

Trust may prioritize review; it does not establish guilt.

Use account identity privately to prevent multiple pseuds from counting as independent reporters or voters.

## 18.4 Quorum defaults

- Routine tag change: two independent approvals.
- High-impact tag or public identity merge: three.
- Routine sanction: proposer plus independent reviewer.
- Permanent ban: three eligible reviewers where staffing permits.
- Appeal: reviewers independent of the original decision.
- Emergency containment: one authorized moderator, followed by review.

Preservation releases and significant metadata corrections use explicitly assigned review policies.

## 18.5 Emergency actions

- Temporarily hide content.
- Freeze replies.
- Restrict messaging.
- Suspend posting.
- Disable an extension.
- Pause an unsafe importer.

Require reason, review deadline, escalation, and audit entry.

## 18.6 Bootstrap mode

If only one administrator is available, label single-person decisions honestly. Do not call them quorum.

## 18.7 Public modlog

Publish redacted decision summaries, not private evidence.

Do not expose:

- Hidden pseud linkage.
- Private messages.
- Child-related evidence.
- Reporter identity.
- Source credentials.
- Sensitive search or reading history.

## 18.8 Community feedback on moderation process

Adopt process feedback, not popularity-based verdicts.

Allow eligible participants to assess redacted decision summaries for:

- Clarity.
- Consistency with published policy.
- Procedural fairness.
- Adequacy of explanation.

Requirements:

- Optional participation.
- Aggregation thresholds.
- Anti-brigading controls.
- No disclosure of private case evidence.
- No automatic reversal or punishment.
- No public moderator popularity leaderboard.
- Independent review of persistent process concerns.

Feedback is not a substitute for appeals and must be labeled as participant opinion, not a factual finding about the case.

## Acceptance

- No self-approval.
- Two pseuds from one account count once.
- Appeals exclude original decision-makers.
- Temporary actions trigger deadlines.
- Public summaries are redacted.
- Process feedback cannot directly impose sanctions.
- Financial activity has no effect on eligibility.

---

# 19. Milestone 14: Credits, Fair Queues, Bounties, and Billing

## 19.1 Ledger

Use balanced entries, not balance mutation without history.

```text
transaction_id
type
idempotency_key
reference
entries[]
created_at
```

Separate:

- Earned credits.
- Subscription grants.
- Purchased credits, if enabled.
- Held credits.

Balances are transactional or derived from verified cached totals.

## 19.2 Job charging

```text
quote
→ reserve
→ submit
→ complete
→ capture actual charge
```

Failure:

```text
release hold or apply documented partial charge
```

Quotes cover batch imports, device delivery, conversion, and AI where applicable.

## 19.3 Initial economy

| Action | Credits |
|---|---:|
| Daily regeneration | 10, capped regenerated balance |
| First publication | 20, once and abuse-reviewed |
| Accepted challenge completion | 10, monthly cap |
| Reviewed mentorship | 10, monthly cap |
| Recognized constructive review | 2, strict caps |
| Reviewed governance contribution | 5 per approved batch, not per sanction |

Do not reward raw reading surveillance, posting volume, sanctions issued, or positive star ratings.

| Priority job | Credits |
|---|---:|
| Import priority | 2 base |
| Additional 20-chapter batch | 1 |
| EPUB/HTML/text/Markdown priority | 1 |
| PDF/AZW3/MOBI priority | 3 |
| Background report | 5 |
| AI | Explicit estimate |

Standard jobs remain free within fair-use limits.

## 19.4 Scheduling

Use weighted queues with aging.

Reserve capacity for standard jobs. Paid demand must not starve free users.

Maintain separate resource classes so a large converter cannot block all small imports.

## 19.5 Bounties

```text
funded → claimed → submitted → accepted → paid
```

Alternatives:

```text
expired | disputed | refunded | canceled
```

Define deadlines, evidence, disputes, and acceptance authority before enabling transfers.

## 19.6 Billing

Optional configurable reference plans:

- Supporter: €3/month.
- Creator: €8/month.
- Patron: €15/month.

No unlimited compute, purchased trust, or search-ranking advantage.

Webhook handling:

- Signature verification.
- Event-ID storage.
- Idempotency.
- Out-of-order handling.
- Reconciliation.

## Acceptance

- Concurrent spending cannot overdraw.
- Retried webhooks do not duplicate grants.
- Failed jobs release holds.
- Standard jobs execute under paid load.
- Billing-disabled operation retains the core archive.
- Credits and monetary marketplace revenue remain distinct ledgers.

---

# 20. Milestone 15: Marketplace, Extension Isolation, and Gallery Mechanics

## 20.1 Manifest

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
```

## 20.2 Categories

- Reader widgets.
- Dashboard widgets.
- Themes.
- Recommendation engines.
- Declarative recipes.
- Search helpers.
- Writing tools.
- Challenge variants.
- Integrations.

## 20.3 Capabilities

Examples:

- Public metadata.
- Selected preferences.
- User-selected draft.
- Approved network domains.
- Specific widget slots.

No implicit access to:

- All drafts.
- Messages.
- Hidden pseud linkage.
- Administrator taste profiles.
- Arbitrary network destinations.
- Source credentials.

Source-authentication secrets stay inside the host’s approved integration pathway.

## 20.4 WASM execution

Use `wasmi` initially.

Enforce:

- Fuel.
- Memory ceilings.
- Output limits.
- Host-call limits.
- Bounded concurrency.
- Process isolation where practical.
- Worker termination for hard wall-clock limits.
- Host-I/O timeouts.

Trust changes resource ceilings, not permissions.

Reference memory tiers:

```text
TL0 16 MiB
TL1 24 MiB
TL2 32 MiB
TL3 48 MiB
TL4 64 MiB
TL5 96 MiB
TL6 128 MiB
```

Benchmark fuel rather than assuming a direct milliseconds conversion.

## 20.5 Review workflow

```text
submitted
→ automated checks
→ permission review
→ security review
→ independent approval
→ published
```

New permissions require renewed user consent.

Provide emergency revocation and rollback.

## 20.6 Gallery mechanics

Support:

- Idempotent installation.
- One active installation per package and pseud scope unless explicitly designed otherwise.
- Version pinning and update policy.
- Uninstall.
- Optional star rating and written review.
- Review reporting.
- Install counts with documented semantics.
- Remix/fork lineage.
- License compatibility checks.
- Attribution preservation.

Forking does not bypass review or copy user grants.

Do not inflate install counts through reinstalls or expose individual installation histories publicly.

Marketplace ratings do not affect account trust.

## 20.7 Themes and layout safety

Use tokens, constrained layout slots, and scoped styling.

Provide:

- Preview.
- Reset.
- Safe mode.
- Accessibility checks.
- Protected security and recovery controls.
- No arbitrary JavaScript disguised as styling.
- No uncontrolled remote-resource loading through CSS.

Modern and archive presets are first-party themes over the same functional surfaces.

## 20.8 Paid marketplace

Separate from credits:

- Purchases.
- Entitlements.
- Refunds.
- Developer revenue accounting.
- Optional processor-backed payouts.
- Configurable reference split of 85/15.
- Operator tax/invoicing setup.

Paid status never bypasses security review.

## Acceptance

- Infinite loops terminate.
- Memory excess fails safely.
- Host APIs enforce permission denial.
- Revoked packages stop running.
- Uninstall removes grants.
- Reset remains accessible after a broken theme.
- Forks preserve attribution and request their own grants.
- Custom engines and recipes honor opt-out.

---

# 21. Milestone 16: Public API, Bots, Feeds, Push, Federation, and AI

## 21.1 Public developer API

The application API is a documented supported interface, not merely an internal frontend detail.

Deliver:

- OpenAPI specification.
- Interactive documentation.
- Scoped tokens.
- Explicit acting pseud.
- Cursor pagination.
- Rate-limit documentation and response headers.
- Idempotency conventions.
- Versioning and deprecation policy.
- Example clients.
- Error-code reference.
- Security reporting guidance.

Publish only supported endpoints. Administrative and experimental APIs are marked separately.

Third-party tools receive the same authorization, content, privacy, and rate-limit checks as the frontend.

## 21.2 Chat bots as thin REST clients

Provide a bot-client framework and reference adapters for:

- Discord.
- Telegram.
- Matrix.

Each adapter gets separate fixture and live-verification status.

Supported initial actions:

- Search eligible public works.
- Fetch public metadata.
- Start an authorized private import.
- Check job status.
- Request a permitted export.
- Save a bookmark.
- Return a link to continue on Lorehaven.

Linking:

```text
bot issues short-lived linking challenge
→ user opens Lorehaven
→ signs in on Lorehaven only
→ selects pseud and scopes
→ confirms
→ bot receives revocable limited authorization
```

Bots never receive the user’s Lorehaven password.

Security:

- Tokens stored securely by the bot deployment.
- No private library results posted to public channels.
- Private actions require private delivery or a Lorehaven link.
- Destination/channel context checked before every response.
- Revocation and unlinking.
- Explicit disclosure that the chat provider receives submitted messages.

A bot is a separate client, not a privileged path around application policy.

## 21.3 Notifications

```text
domain event
→ notification eligibility
→ in-app record
→ optional email/push job
```

Apply privacy and rating restrictions before generating text and again where necessary before delayed delivery.

Support mention alerts, replies, source-update notices, credential expiry, batch completion, digests, and delivery failures.

## 21.4 Push

- Request permission after relevant user action.
- Store subscriptions per device.
- Remove invalid subscriptions.
- Generic lock-screen text by default.
- Best-effort delivery.
- No sensitive excerpts by default.

## 21.5 RSS, Atom, and OPDS

Provide:

- Public feeds.
- Scoped revocable private feeds.
- OPDS catalogs with eligible download links.
- Documented token rotation and revocation.

Never log private feed tokens.

Private feed URLs are bearer secrets and require explicit disclosure.

## 21.6 ActivityPub

Initial scope:

- Public opted-in pseud actors.
- Follow/unfollow.
- Work announcements.
- Updates.
- Deletion notices.

Implement:

- Signature verification.
- Retry.
- Replay and abuse controls.
- SSRF defenses.
- Remote actor cache.
- Instance blocks.
- Opt-out and deletion documentation.

Do not federate private imports, reading history, hidden account linkage, or private ratings.

Explain that remote copies may persist after a deletion notice.

## 21.7 AI provider interface

Tasks:

- Summarization.
- Translation.
- Grammar assistance.
- Prompt assistance.
- Embeddings.
- Optional metadata suggestions.

Requirements:

- Disabled without configuration.
- Explicit private-text consent.
- Quoted costs.
- Cancellation.
- No automatic publication.
- Generated output distinguished from author text.
- Same permissions and exclusions as ordinary search.
- Provider-specific retention and data-use disclosure.

## 21.8 Translation review

Translation states:

```text
requested
→ generated/imported
→ private draft
→ submitted for review
→ changes_requested/approved/rejected
→ published
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

A curator’s approval is a quality/workflow decision, not a grant of copyright permission.

Private reader translations need not enter a public review queue unless the user submits them.

## Acceptance

- API examples execute against test instances.
- Token revocation affects bots promptly.
- Public channels never receive private results through fallback behavior.
- Feed restrictions match reader restrictions.
- Delayed notifications recheck access.
- AI-disabled operation remains complete.
- Translation approval cannot silently publish an unauthorized derivative work.

---

# 22. Milestone 17: Administration, Statistics, Abuse Defense, Privacy, and Operations

## 22.1 Administration routes

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
/admin/translations
/admin/billing
/admin/extensions
/admin/federation
/admin/analytics
/admin/abuse
/admin/audit
/admin/health
/admin/backups
```

Show:

- Memory and disk pressure.
- Job backlog.
- Failed imports.
- Source incidents.
- Converter availability.
- Backup age.
- Moderation backlog.
- Database health.
- Integration status.
- Search index lag.
- Credential-expiry counts without secret details.
- Retention and deletion backlog.

Ordinary dashboards do not expose individual reading histories.

## 22.2 Public statistics

Provide `/stats` with eligible aggregate metrics such as:

- Public works.
- Public chapters and words.
- New public works by period.
- Public appreciation counts.
- Explicitly public bookmark counts.
- Aggregate eligible views.
- Active public contributors, under a documented definition.
- Source support status and public incidents.

Do not count private imports as public archive holdings.

Public “works read” metrics, if enabled, require separately permitted aggregate input, cohort suppression, and clear definitions. They are not derived silently from private history.

Each metric documents:

- Definition.
- Refresh interval.
- Coverage.
- Exclusions.
- Approximation.
- Retention.

Suppress small or sensitive cohorts.

## 22.3 Privacy-preserving usage analytics

Use first-party aggregate analytics without third-party tracking by default.

Separate:

- Product metrics.
- Security logs.
- Personal reading statistics.
- Recommendation learning.

Potential metrics:

- Approximate daily/weekly/monthly visitors.
- Active versus view-only sessions.
- Aggregate action timelines.
- Import success rate.
- Search success rate.
- Export completion.
- Offline-download failures.

Rules:

- Do not call IP addresses or persistent identifiers “non-PII.”
- Avoid cross-site identifiers and browser fingerprinting.
- Use short-lived or rotating identifiers where needed.
- Establish the applicable legal basis.
- Apply consent where required.
- Document estimation limits.
- Never retain raw search queries or reading paths merely to produce broad counts.

Anonymous visitor estimates must not be presented as exact people counts.

## 22.4 Abuse dashboard

Restrict detailed security telemetry to authorized security staff.

Show:

- Request bursts.
- Failed-authentication bursts.
- Expensive-operation rates.
- Export-to-request ratios.
- Source-domain pressure.
- Repeated limit violations.
- Temporary mitigations and expiry.

IP addresses and related identifiers are personal/security data:

- Short retention.
- Access audit.
- Masked ordinary display.
- Justified reveal.
- No public leaderboard of addresses.

## 22.5 Layered abuse defense

Required baseline:

- Resource-specific rate limits.
- Account/client/network layers.
- Concurrency limits.
- Upload and extraction limits.
- Session and token controls.
- Source throttling.
- Anomaly alerts.

Optional escalating controls:

- Accessible honeypots.
- Form-timing signals.
- Targeted proof-of-work with strict device budgets.
- Targeted browser challenges.
- Manual review or alternative verification.

These signals must not alone establish abuse.

Do not:

- Block solely because JavaScript is disabled.
- Treat assistive technology as automation.
- Blanket-block headless browsers.
- Require expensive proof-of-work for ordinary reading.
- Penalize shared NAT users through a single narrow global IP bucket.
- Use invasive fingerprinting by default.

Every challenge needs an accessible fallback and a way to recover from false positives.

## 22.6 Data export

Export:

- Account settings.
- Pseuds.
- Authored works.
- Private library metadata.
- Permitted private content copies.
- Bookmarks and notes.
- Ratings and reviews.
- History and personal statistics.
- Saved views and recipes.
- Relevant messages.
- Credit history.
- Consent and authorization records.

Exclude plaintext source credentials, active session secrets, API tokens, and private feed tokens.

Use a job and expiring authenticated download.

## 22.7 Deletion

Distinguish:

- Account.
- Pseud.
- Work.
- Eligible work orphaning.
- Private library copy.
- Source credential.
- History.
- Rating/review.
- Integration authorization.
- Local offline data.

Document retention exceptions and backup expiry.

Deletion propagates to:

- Search indexes.
- Recommendation signals.
- Caches.
- Pending notifications.
- Derived personal statistics.
- Blob references.
- Integration grants.

Shared physical blobs remain only while valid references or documented cache retention justify them.

## 22.8 Backups

SQLite:

- Use the backup API or a consistent snapshot.
- Do not copy only the main live database while ignoring WAL state.

PostgreSQL:

- Supported backup tooling.
- File-storage consistency strategy.

Include encrypted-secret recovery planning. Operators must understand whether restoring a database without its encryption keys makes source credentials unrecoverable.

Restore tests read restored works and validate private-library authorization.

## 22.9 Upgrades

```text
check compatibility
→ maintenance/coordination
→ backup
→ install verified executable
→ migrate
→ health check
→ resume
```

Do not assume replacing the binary reverses migrations.

Include:

- Database schema compatibility.
- Search reindex requirements.
- IndexedDB migration compatibility.
- Extension host API compatibility.
- Recipe/query AST migration.
- Encryption-key version support.

---

# 23. Milestone 18: Hardening and Release

## 23.1 Functional browser journeys

Automate:

1. Register → create pseud → publish → read.
2. Import → update → bookmark → export.
3. Add source credential → import → expire → renew.
4. Bibliography preview → partial batch → restart → resume.
5. Preservation dry run → review → approved publication.
6. Download → disconnect → read offline.
7. Rate privately → publish review → verify visibility.
8. Record history → pause → clear.
9. Search bound character attribute → exclude ship.
10. Search private body phrase → verify no cross-user leakage.
11. Save query → pin view → migrate query version.
12. Change engine → install recipe → opt out.
13. Forum read state → mention → digest → reconnect.
14. Report → quorum → appeal.
15. Earn credits → reserve → capture/refund.
16. Install extension → deny permission → fork → uninstall.
17. Link bot → import privately → revoke.
18. Export account → delete account.
19. Backup → restore to a clean instance.
20. Complete critical journeys in both initial locales.

## 23.2 Security tests

Cover:

- Stored XSS.
- CSRF.
- SSRF and DNS rebinding.
- Credential forwarding across redirects.
- File traversal.
- Malicious archives and decompression bombs.
- Object-level authorization.
- Pseud isolation.
- Search snippet and count leakage.
- Shared-cache existence leakage.
- Session/token revocation.
- Credit races.
- Webhook replay.
- Plugin exhaustion.
- CSS resource exfiltration.
- Private cache leakage.
- Bot channel-context errors.
- Email relay abuse.
- Feed-token logging.
- Retention and deletion failures.

## 23.3 Accessibility

Automate what can be automated, then manually test:

- Keyboard-only navigation.
- Screen-reader reading and forms.
- Dialogs.
- Editor.
- Reader settings.
- Command palette.
- Dashboard reordering alternatives.
- Query-builder errors.
- Zoom and mobile layouts.
- Reduced motion.
- Contrast.
- Challenge fallback.
- Both appearance presets.

## 23.4 Performance

Seed:

- 10,000 works.
- 1,000 accounts.
- Realistic chapters and metadata.
- Large works.
- Dense tags.
- Private imports.
- Body-search documents.
- Forum and bookmark activity.
- Batch-import history.

Benchmark:

- Idle memory.
- Read latency.
- Metadata and body search.
- Progress writes.
- Concurrent browsing.
- One heavy conversion.
- Import backlog.
- Bibliography enumeration.
- Search indexing.
- Extension execution.
- Real-time catch-up.
- Disk growth and cache eviction.

Define “100 concurrent users” through a reproducible request mix and think time.

Record actual results. If a 1 GB profile fails, reduce budgets or mark the workload unsupported rather than claiming success.

## 23.5 Platform and integration evidence

Address:

- x86-64.
- ARM64.
- SQLite.
- PostgreSQL.
- Supported browsers.
- PWA limitations by browser.
- Converter versions.
- SMTP delivery.
- Each website adapter.
- Each bot adapter.
- Federation peers used in tests.

Fixture success is not live verification.

---

# 24. Route and API Ownership

| Surface | Frontend routes | API owner |
|---|---|---|
| Discovery | `/`, `/discover`, `/blind-date`, `/dashboard` | discovery |
| Recipes | `/recipes/*` | discovery/extensions |
| Search | `/search`, `/search/advanced`, `/find-fic` | search |
| People | `/people`, `/u/:handle` | identity/content |
| Reader | `/works/*`, `/series/*` | content/library |
| Writing | `/write/*` | content |
| Imports/library | `/library/*` | imports/library |
| Source status | `/sources`, `/sources/:id` | imports |
| Community | `/forum/*`, `/groups/*`, `/messages/*` | community |
| Events | `/challenges/*`, `/requests/*` | community |
| Governance | `/trust/*`, `/moderation/*`, `/curation/*` | governance |
| Credits/billing | `/credits/*`, `/billing` | economy |
| Marketplace | `/marketplace/*` | extensions |
| Help | `/help/*` | app/documentation |
| Developer API | `/developers/*`, `/api/v1/*` | relevant module |
| Statistics | `/stats` | restricted aggregate service |
| Settings | `/settings/*` | relevant module |
| Administration | `/admin/*` | restricted administration |
| Bot clients | external clients, linking under `/settings/integrations` | integrations |

Generate an endpoint inventory from implemented routes and compare it with requirements in CI.

For every endpoint document:

- Authentication.
- Acting pseud.
- Authorization.
- Request schema.
- Response schema.
- Error codes.
- Rate limits.
- Idempotency.
- Privacy classification.
- Cache policy.
- Retention implications.
- Public API stability status.

Bot commands, feeds, and extension host calls must map to documented policies rather than duplicate their own authorization logic.

---

# 25. Tutorial Delivery Plan

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
v0.02-design-system-i18n-help
v0.03-identity
v0.04-publishing
v0.05-reader-feedback-history
v0.06-jobs-storage-secrets
v0.07-importing-batches-preservation
v0.08-exports-offline
v0.09-library-saved-views
v0.10-search-taxonomy
v0.11-discovery-recipes-dashboard
v0.12-community
v0.13-events
v0.14-governance
v0.15-economy
v0.16-marketplace
v0.17-integrations
v0.18-operations
v1.0-release
```

No required functionality is left as “an exercise for the reader.”

The tutorial, contextual help, API documentation, and operator documentation must describe the same implemented behavior.

---

# 26. Final Completion Checklist and Deliberate Exclusions

## 26.1 Core product

- [ ] Authors can create, revise, publish, and complete works.
- [ ] Readers can read suitable public fiction without unnecessary registration.
- [ ] Private imports remain private.
- [ ] Source credentials are encrypted, scoped, revocable, and expiring.
- [ ] Supported adapters have documented evidence.
- [ ] Runtime source health is separate from support status.
- [ ] Bibliography batches resume safely.
- [ ] Preservation batches retain permission records and attribution.
- [ ] Cross-source identity does not merge permissions.
- [ ] Exports are valid.
- [ ] Device delivery is permission-aware and honestly reported.
- [ ] Downloaded reading works offline.
- [ ] Draft conflicts never silently destroy text.

## 26.2 Reading and library

- [ ] Ratings default to private.
- [ ] Public reviews require explicit publication.
- [ ] Private ratings do not enter public aggregates.
- [ ] History, progress, and learning have separate controls.
- [ ] Reading metrics are accurately labeled.
- [ ] Views do not expose visitors.
- [ ] Saved views can be named, pinned, and migrated.
- [ ] Shared storage does not reveal private holdings.
- [ ] Deletion and cache retention behave as documented.

## 26.3 Search and discovery

- [ ] Main/supporting distinctions work.
- [ ] Character attributes bind correctly.
- [ ] Negative ship filtering does not remove characters.
- [ ] Unknown metadata is explicit.
- [ ] Query syntax and visual filters share one AST.
- [ ] Body search is permission-safe.
- [ ] Counts, facets, and snippets do not leak private content.
- [ ] Exact search remains exact.
- [ ] Administrator tastes remain private at the data/API level.
- [ ] Influence updates automatically.
- [ ] Opt-out works across recommendations, recipes, widgets, prompts, and notifications.
- [ ] Community similarity is distinct from machine-generated similarity.
- [ ] Search-demand insights protect sensitive queries.

## 26.4 Community and governance

- [ ] Pseuds remain compartmentalized.
- [ ] Forum unread state works.
- [ ] Mentions and digests honor blocks and visibility.
- [ ] Real-time delivery has durable catch-up.
- [ ] Scoped sanctions expire.
- [ ] Reports, quorum, appeals, and emergency review work.
- [ ] Trust cannot be purchased.
- [ ] Process feedback does not become mob adjudication.
- [ ] Public logs are redacted.
- [ ] Child-related controls operate server-side.

## 26.5 Economy and extensibility

- [ ] Credits cannot double-spend.
- [ ] Free queues cannot starve.
- [ ] Billing can be disabled.
- [ ] Plugins have bounded execution at every trust level.
- [ ] Permissions are enforced by the host.
- [ ] Installation is idempotent.
- [ ] Forks preserve attribution and undergo review.
- [ ] Revocation, rollback, uninstall, and safe mode work.
- [ ] No paid product buys ranking or governance authority.

## 26.6 Interface and integrations

- [ ] Modern and archive presets preserve functionality.
- [ ] Command palette is accessible.
- [ ] Contextual help links remain valid.
- [ ] English and Spanish critical journeys are reviewed.
- [ ] Public API documentation matches implemented endpoints.
- [ ] Bots store revocable authorization rather than user passwords.
- [ ] Public channels cannot receive private results accidentally.
- [ ] Feeds and federation preserve privacy boundaries.
- [ ] AI is optional and private-text use is explicit.
- [ ] Translation review does not replace publication permission.

## 26.7 Operations

- [ ] SQLite and PostgreSQL integration tests pass.
- [ ] Backups restore usable content and correct permissions.
- [ ] Upgrades have migration safeguards.
- [ ] ARM64 and x86-64 builds are addressed.
- [ ] Resource claims are measured.
- [ ] Public statistics exclude private holdings.
- [ ] Analytics definitions and retention are documented.
- [ ] Abuse controls have accessible fallbacks.
- [ ] Security telemetry is treated as sensitive data.
- [ ] Accessibility includes manual verification.
- [ ] Documentation matches the delivered repository.

## 26.8 Deliberately not adopted

The following FicNexus-style behaviors are excluded or materially constrained:

### XP and rank-gated core functionality

No 100-level progression system, ability tree, or reading/download XP gates.

Widgets, themes, saved views, recipe creation, reading, and publishing are not unlocked through activity grinding.

### Posting-volume reputation

No global or forum-local score that rewards raw posting volume or becomes a proxy for trust.

Use reviewed reliability, scoped expertise, appointed roles, and ordinary appreciation.

### A promised source count

No “107+ supported sources” claim in advance. Counts follow verified adapter support.

### A permanent archive of every fetched body

No automatic public corpus created from private imports.

Shared infrastructure may deduplicate bytes, but public preservation requires its own permission, review, and retention workflow.

### Credentials as an SSRF exception

Authenticated fetching remains subject to the same network restrictions. Internal-network integrations require separate explicit administrative configuration.

### Popularity-based moderation verdicts

Community process feedback is advisory. It does not replace evidence, independent review, or appeals.

### Invasive or exclusionary bot detection

No default fingerprinting, blanket headless-browser bans, compulsory JavaScript for reading, or expensive universal proof-of-work.

### Coercive reading analytics

No mandatory streaks, loss warnings, public reading leaderboards, or rewards tied to surveillance of reading behavior.

### Automatic publication of suggestions or translations

Metadata suggestions, AI outputs, preservation imports, and translations remain reviewable drafts until the appropriate authorized action.

---

The resulting project should be judged by these working behaviors—not by the number of screens, lines of code, imported feature names, or claims in a README.
