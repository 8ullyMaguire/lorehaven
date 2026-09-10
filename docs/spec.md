# Lorehaven: Implementation Plan

This is a dependency-ordered plan for building the entire platform—not a prototype. It specifies architecture, data structures, workflows, implementation milestones, and verification requirements.

The three foundational priorities are:

1. **Reliable importing and private-library management.**
2. **Safe, low-friction writing and publishing.**
3. **Excellent reading, downloading, and offline access.**

Community, recommendations, governance, credits, and extensions build on those foundations.

The administrator’s tastes remain private. They influence discovery automatically through an opt-out recommendation layer; the administrator does not have to curate lists or publicly explain personal preferences.

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

## 1.3 Build vertical slices

Complete a small end-to-end workflow before expanding it.

For example:

> Create draft → save chapter → publish → read → edit → read updated version.

Do not build fifty backend endpoints before connecting the first frontend page.

## 1.4 Use architectural decision records

Store decisions in:

```text
docs/adr/0001-stack.md
docs/adr/0002-content-model.md
docs/adr/0003-pseud-isolation.md
...
```

Each record contains:

- Problem.
- Decision.
- Alternatives.
- Consequences.
- Conditions that would justify revisiting it.

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

Rust satisfies the native-compilation, memory-efficiency, expressive-type-system, and ARM64 requirements. The developer is assumed to know Rust basics, so its learning curve is acceptable.

### Why Svelte and TypeScript

This combination keeps interactive frontend code reasonably concise and works well with browser APIs. The production application does not need a Node.js server.

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
Browser / Installed PWA
          |
        HTTPS
          |
        Caddy
          |
    Lorehaven executable
      ├── HTTP/API
      ├── Embedded frontend
      ├── Job scheduler
      ├── Import workers
      ├── Export workers
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
- Redis.

Core reading, writing, publishing, and private importing must remain functional without optional cloud services.

## 2.3 Repository structure

```text
lorehaven/
├── Cargo.toml
├── Cargo.lock
├── crates/
│   ├── app/                 # CLI, startup, composition
│   ├── domain/              # IDs, entities, policies, errors
│   ├── db/                  # Repositories and migrations
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
│   │   └── styles/
│   └── tests/
├── migrations/
│   ├── sqlite/
│   └── postgres/
├── fixtures/
│   ├── importers/
│   ├── documents/
│   └── malicious-inputs/
├── scripts/
├── packaging/
│   ├── systemd/
│   └── caddy/
└── docs/
    ├── adr/
    ├── tutorial/
    ├── operator/
    ├── api/
    ├── verification.md
    └── requirements.csv
```

Do not create a crate for every small feature. The listed boundaries are the maximum initial decomposition; closely related modules can share a crate.

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

Use stable machine-readable error codes.

Important codes include:

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
JOB_FAILED
INSUFFICIENT_CREDITS
EXTENSION_PERMISSION_DENIED
```

Use `404` rather than revealing the existence of inaccessible private objects where appropriate.

## 3.4 Concurrency

Every editable resource gets a monotonically increasing `version`.

Updates include:

```json
{
  "expected_version": 12,
  "changes": {}
}
```

If the version differs, return `409 REVISION_CONFLICT`.

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
authenticate
→ resolve active pseud
→ load resource
→ evaluate policy
→ perform operation
→ record required audit event
```

Put authorization rules in policy functions, not scattered frontend checks.

Example:

```rust
fn can_edit_work(
    actor: &Actor,
    work: &Work,
    contributors: &[Contributor],
) -> Decision;
```

Frontend visibility is convenience, not security.

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

Use JSON for flexible documents and extension manifests, not as a substitute for searchable relationship data.

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
| `api_tokens` | account_id, token_hash, scopes, expiry |

Only specifically authorized staff may retrieve private pseud ownership.

## 4.3 Content tables

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

The structured editor document is the source of truth. Sanitized HTML and plain text are derived representations.

## 4.4 Private import tables

Do not reuse public `works` ownership semantics for external copies.

| Table | Important fields |
|---|---|
| `sources` | domain, adapter_id, enabled, rate_policy |
| `adapter_versions` | adapter_id, version, verification_status |
| `external_records` | source_id, normalized_source_key, public metadata |
| `library_items` | owner_pseud_id, origin_type, work_id/external_record_id |
| `import_snapshots` | library_item_id, source_revision, metadata, checksum |
| `import_chapters` | snapshot_id, source_chapter_key, order, content_reference |
| `import_jobs` | owner_pseud_id, source_url, destination, status |
| `import_attempts` | job_id, stage, result, redacted_error |
| `provenance_records` | item_id, source_url, author_label, import_time, permission_state |

Physical deduplication, if used, must never imply shared authorization.

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
```

Important constraints:

- Alias uniqueness is scoped by type and namespace.
- Relationship participant sets have a stable canonical signature.
- A work-character row identifies the character to which attributes belong.
- Relationship prominence belongs to the work’s assertion, not the global relationship.
- Spoiler status belongs to the assertion or displayed metadata item.
- Merges preserve redirects and history.

## 4.6 Other entity groups

Implement these as their milestones arrive:

| Module | Tables |
|---|---|
| Library | bookmarks, notes, shelves, shelf_entries, reading_progress, saved_searches |
| Jobs | jobs, job_attempts, job_events, outbox_events |
| Community | comments, reactions, follows, groups, memberships, boards, topics, posts, polls, poll_votes |
| Messaging | conversations, conversation_members, messages, chat_rooms |
| Collections | collections, collection_roles, collection_submissions, collection_entries |
| Writing events | challenges, prompts, signups, assignments, claims, fulfillments, mentorships, sprints |
| Governance | trust_policies, trust_history, expertise, role_assignments, reports, cases, proposals, votes, sanctions, appeals, audit_events |
| Economy | wallets, ledger_transactions, ledger_entries, credit_holds, subscriptions, payment_events, bounties |
| Extensions | packages, package_versions, manifests, installations, grants, reviews, approvals, execution_usage, revocations |
| Discovery | user_preferences, taste_profiles, taste_profile_versions, permitted_signals, exposure_events, aggregate_affinities |
| Integrations | notifications, push_subscriptions, feed_tokens, federation_actors, federation_deliveries |

For each migration, document deletion and retention behavior.

---

# 5. Milestone 0: Repository, Tooling, and Running Application

## Implement

1. Create the Rust workspace and frontend.
2. Add development configuration.
3. Add structured logging with request IDs.
4. Add embedded frontend assets.
5. Add database selection.
6. Add migration commands.
7. Add a health endpoint.
8. Add a development seed command.
9. Add continuous integration.

Commands:

```text
lorehaven serve
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

Never log secrets or full authenticated URLs.

## Acceptance

- A clean checkout builds.
- The application starts with SQLite.
- The application starts with PostgreSQL.
- A frontend page loads from the Rust executable.
- `/health/live` checks process liveness.
- `/health/ready` checks essential dependencies.
- Production startup rejects unsafe development configuration.

## Tutorial chapter

Explain the browser/backend boundary, configuration, migrations, and embedded assets.

---

# 6. Milestone 1: Design System and Navigation

## Implement reusable components

- Buttons and links.
- Inputs and labels.
- Selects and comboboxes.
- Dialogs.
- Drawers.
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

Reference visual direction:

- Warm neutral surfaces.
- Deep plum accent.
- Restrained teal secondary accent.
- Generous spacing.
- Light, dark, and system modes.

## Navigation

Desktop:

```text
Discover | Search | Library | Write | Community | Notifications | Pseud
```

Mobile:

```text
Discover | Search | Library | Write | More
```

Reader pages may use a reduced navigation shell.

## Acceptance

- All controls work with a keyboard.
- Focus is visible.
- Dialog focus is trapped and restored.
- Layout works at 320 CSS pixels.
- Pages remain usable at 200% zoom.
- Reduced-motion preference is respected.
- Errors are announced accessibly.

Do this before building dozens of pages.

---

# 7. Milestone 2: Accounts, Pseuds, Privacy, and Age Policy

## Implement

- Registration and login.
- Password reset.
- Email verification.
- Session listing and revocation.
- TOTP and recovery codes.
- Pseud creation and switching.
- Privacy settings.
- Block and mute primitives.
- Age-policy state.

## Pseud behavior

Each pseud has separate:

- Public profile.
- Works.
- Follows.
- Messages.
- Recommendation settings.
- Public bookmarks.
- Notification preferences.

The account shares:

- Credentials.
- Security state.
- Private wallet.
- Trust eligibility.

Do not publicly reveal shared ownership.

## Age implementation

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
- Spain generally uses 14 as the relevant threshold for a child’s own data-processing consent; this does not settle all age-related obligations.
- If an under-threshold authorization workflow has not been legally and operationally established, do not enable unrestricted child registration merely because a checkbox exists.

Avoid collecting full birth dates unless necessary.

## Content eligibility service

Implement one shared service:

```text
can_access_content(actor, content_rating, visibility, policy)
```

Use it for:

- Reader.
- Search.
- Downloads.
- Feeds.
- Notifications.
- Recommendations.
- APIs.
- Extensions.

## API

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
```

## Acceptance

- An account cannot edit another account’s pseud.
- Pseud linkage is absent from public API responses.
- Session revocation takes effect.
- Restricted content cannot be retrieved by bypassing the frontend.
- Logs contain no passwords, reset tokens, or hidden linkage.
- Minor-protective messaging defaults are stored from onboarding.

---

# 8. Milestone 3: Drafts, Chapters, Publishing, and Revisions

## First vertical slice

1. Create a draft.
2. Enter a title.
3. Add a chapter.
4. Save.
5. Preview.
6. Publish.
7. Read publicly.
8. Edit and republish.

## Work states

Separate lifecycle from completion:

```text
Lifecycle:
draft | scheduled | published | withdrawn | deleted

Visibility:
public | unlisted | restricted

Completion:
in_progress | complete | hiatus | abandoned
```

Unlisted does not mean encrypted or secret. Explain that anyone with the URL may access it under applicable restrictions.

## Editor implementation

Use a restricted Tiptap schema supporting:

- Paragraphs.
- Headings.
- Emphasis.
- Strong text.
- Lists.
- Blockquotes.
- Links.
- Scene breaks.

Do not initially allow arbitrary HTML, scripts, iframes, or custom embedded objects.

Autosave:

1. Debounce edits.
2. Save locally for recovery.
3. Send an update with `expected_version`.
4. Display saved/saving/offline/conflict state.
5. Preserve local text when the server rejects a save.

## Publishing transaction

In one transaction:

- Validate required metadata.
- Verify contributor permissions.
- Update publication state.
- Record publication event.
- Insert outbox events for notifications and indexing.

Do not send email inside the database transaction.

## API

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

- Concurrent edits return a conflict.
- Restoring a revision creates a new revision.
- Public readers never receive unpublished chapter revisions.
- Publishing twice with the same idempotency key does not duplicate notifications.
- Changing the active pseud does not change existing work ownership.
- Coauthor invitations clearly identify the exposed pseud.

---

# 9. Milestone 4: Reader and Work Pages

## Routes

```text
/works/:id
/works/:id/chapters/:chapterId
/works/:id/download
/works/:id/comments
/series/:id
```

## Implement

- Chapter navigation.
- Whole-work mode.
- Table of contents.
- Reader typography settings.
- Light, dark, and sepia reader themes.
- Width and line-height controls.
- Distraction-free mode.
- Spoiler reveal.
- Progress.
- Private notes.
- End-of-work actions.

## Progress representation

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

Use stable paragraph anchors when possible. Fall back to approximate fraction after content changes.

When two devices disagree, present a choice rather than always selecting the furthest point.

## End-of-work page

Show:

- Appreciation.
- Bookmark.
- Comment.
- Next in series.
- Another work by the author.
- Configurable next reads.
- Optional writing opportunity.

## Acceptance

- Progress survives refresh.
- Reader preferences survive login/logout as documented.
- A chapter update does not crash resume.
- Long works remain responsive.
- Sanitized content cannot execute scripts.
- Public work pages provide meaningful metadata for sharing; public eligible pages should also have useful non-JavaScript content rather than a blank shell.

---

# 10. Milestone 5: Job System and Storage

Build this before importers and converters.

## Job model

```text
queued
→ running
→ succeeded

running
→ retry_wait
→ queued

running
→ failed

queued/running
→ canceled
```

Fields include:

```text
kind
owner_account_id
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

## Worker behavior

- Claim jobs transactionally.
- Use leases.
- Renew leases for long tasks.
- Retry transient errors with backoff.
- Limit attempts.
- Recover abandoned jobs.
- Honor cancellation between safe checkpoints.

Delivery is at least once. Job handlers must therefore be idempotent.

## Storage

Use content-addressed or generated storage keys, not user-supplied filenames.

Implement:

- Temporary upload directory.
- Atomic finalization.
- Checksums.
- Quotas.
- Orphan cleanup.
- Disk-pressure warnings.
- Safe path resolution.

## Acceptance

- Killing a worker does not permanently lose a job.
- Two workers do not charge or publish twice.
- Low disk space produces a useful error.
- Path traversal uploads fail.
- Job errors are redacted before display.

---

# 11. Milestone 6: Import Framework and Real Source Adapters

## Import abstraction

```rust
trait SourceAdapter {
    fn identify(&self, url: &Url) -> Option<SourceKey>;
    async fn fetch_metadata(&self, request: ImportRequest)
        -> Result<SourceMetadata, ImportError>;
    async fn fetch_chapter(&self, chapter: SourceChapter)
        -> Result<ImportedChapter, ImportError>;
}
```

Use a shared safe HTTP client rather than letting adapters make arbitrary requests.

## Import destinations

Require an explicit choice:

- Private library.
- Own draft.
- Republication with permission.

Default URL imports to private library.

## Workflow

```text
URL/file
→ source detection
→ metadata preview
→ destination selection
→ confirmation
→ queued import
→ chapter fetching
→ sanitation
→ duplicate/update handling
→ completed library item
```

## Security

The shared fetcher must:

- Reject unsupported URL schemes.
- Reject loopback, link-local, and private-network targets unless explicitly permitted for a trusted administrative integration.
- Validate redirects.
- Address DNS-rebinding risks.
- Limit bytes, time, redirects, and decompression.
- Remove embedded trackers and unsafe resources.
- Redact source credentials.

## Initial adapter work

Implement a generic document/file importer first:

- EPUB.
- HTML.
- Plain text.
- DOCX.

Then implement real website adapters one at a time.

For each adapter:

1. Document recognized URLs.
2. Add representative fixtures.
3. Implement metadata parsing.
4. Implement chapter enumeration.
5. Implement content extraction.
6. Add malformed-page tests.
7. Add update tests.
8. Perform live verification if permitted and available.
9. Record support status.

Do not promise 107 sources in advance.

## Update policy

Never overwrite a private imported copy destructively.

Create a new snapshot, compare chapters, and preserve:

- Notes.
- Shelf membership.
- Bookmarks.
- Progress where mappable.

If chapters were removed or reordered, show a review notice.

## Acceptance

- Reimporting the same URL does not create accidental duplicates.
- Failed chapter fetching can resume.
- External content never becomes public automatically.
- Malicious EPUB paths cannot escape extraction storage.
- Source failure is distinguishable from parser failure.
- Unsupported adapters are excluded from support counts.

---

# 12. Milestone 7: Exports and Offline Reading

## Exports

Implement in this order:

1. Plain text.
2. Sanitized standalone HTML.
3. EPUB.
4. PDF converter integration.
5. MOBI converter integration.

For converters:

- Discover executable availability at startup.
- Use fixed argument construction.
- Run with time, memory, and output limits.
- Do not invoke through unsafe shell interpolation.
- Disable unavailable formats with clear installation guidance.

## EPUB acceptance

Validate:

- ZIP structure.
- MIME declaration.
- Package metadata.
- Navigation.
- Chapter ordering.
- Unicode.
- Attribution.
- Source provenance.

## PWA implementation

Create:

- Web manifest.
- Icons.
- Service worker.
- IndexedDB schema.
- Download manager.
- Offline reader route.
- Update notification.

Caching:

| Data | Strategy |
|---|---|
| Versioned assets | Cache-first |
| Public dynamic data | Network/revalidation |
| Explicitly downloaded works | IndexedDB |
| Private authenticated responses | No blanket shared caching |
| Reading progress | Local first, queued sync |

## Offline privacy

On logout, ask whether to remove local private downloads, with a privacy-protective default on shared devices.

Explain that:

- Browser storage may be evicted.
- A downloaded copy cannot always be remotely revoked.
- Device access can expose offline content.
- Service-worker caching is not encryption.

## Acceptance

- Downloaded chapters open with the network disabled.
- Missing chapters show an actionable offline screen.
- Interrupted downloads resume or restart cleanly.
- Storage quota errors do not destroy existing downloads.
- App upgrades migrate IndexedDB safely.
- Private data does not appear in another account’s library.
- Background-sync absence falls back to foreground synchronization.

---

# 13. Milestone 8: Library, Bookmarks, Shelves, and Updates

## Routes

```text
/library
/library/imports
/library/imports/new
/library/imports/:id
/library/shelves
/library/shelves/:id
/library/bookmarks
/library/downloads
/library/history
```

## Implement

- Shelves.
- Reading statuses.
- Private tags.
- Bookmark notes.
- Batch actions.
- Source filters.
- Update checking.
- Duplicate review.
- Storage usage.

Default bookmarks to private. Public visibility requires explicit selection.

Batch operations return per-item outcomes:

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

- Public bookmark lists never include private entries.
- Deleting a shelf does not delete its works.
- Removing a library item clearly distinguishes removing a reference from deleting a private stored copy.
- Update checking respects source rate limits.

---

# 14. Milestone 9: Structured Taxonomy and Advanced Search

This is a major feature, not a cosmetic tag redesign.

## Character assertions

```text
Character A:
  prominence = protagonist
  roles = [mentor]
  attributes = [vampire, BAMF]
```

## Relationship assertions

```text
Participants = [A, B]
Kind = romantic
Prominence = central
Dynamics = [enemies_to_lovers]
```

Support relationships with more than two participants.

## Query representation

Use a typed search AST:

```text
And
Or
Not
Text
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

Compile bound predicates into SQL `EXISTS` clauses. Do not use joins that accidentally let one character satisfy another character’s attributes.

## Unknown metadata

Negative filters need three modes:

- Match declared metadata.
- Require sufficiently complete metadata.
- Include uncertain results separately.

A missing ship assertion is not proof that the story contains no ship.

## Search filters

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

Bound query depth, clause count, and execution time.

## Canonicalization

Workflow:

```text
proposal
→ review
→ quorum
→ apply
→ reindex affected works
→ preserve redirect/history
```

Aliases are fandom/type scoped. Ambiguous aliases require disambiguation.

“Enemies to Lovers implies Slow Burn” is a suggestion, not a universal logical rule.

## Ranking

Only primary tags add tag-ranking boosts. Secondary tags remain filterable.

Exact search is not taste-steered.

## Acceptance fixture

Create deliberately contrasting works:

- A protagonist/B vampire.
- A vampire protagonist.
- A supporting vampire.
- A/B central.
- A/B background.
- A with no declared relationship metadata.
- A with confirmed no romantic/sexual relationship.

Write tests proving each advanced query returns the intended subset.

---

# 15. Milestone 10: Recommendations and Private Taste Influence

## Baseline engines

Implement first:

- Recent.
- Trending by time window.
- Content similarity.
- Also bookmarked.
- Similar by bookmarks.
- Blind Date.
- User-preference matching.

Each implements:

```text
generate_candidates(context, limit)
→ candidate IDs and baseline scores
```

All candidates pass shared eligibility rules.

## Private administrator profile

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

Use configurable decay and profile versions.

## Influence layer

Reference scoring:

```text
score =
  0.80 × user_relevance
  + 0.12 × administrator_affinity
  + 0.08 × bridge_relevance
```

These are initial tunable values.

Add:

- Relevance floor.
- Author concentration limits.
- Repetition limits.
- Negative-feedback cooldowns.
- Diversity reranking.
- Completion preference.

## Meaningful opt-out

Setting:

> Include this instance’s evolving discovery preferences alongside your own interests.

Turning it off removes the administrator profile from:

- Candidate generation.
- Ranking.
- Reranking.
- Prompts.
- Challenges.
- Notifications.
- Cached recommendations.

Administrator preferences remain private; individual recommendations do not need administrator-specific labels.

Do not fabricate recommendation explanations.

## Automatic writing opportunities

Generate optional:

- Trope combinations.
- Weekend prompts.
- Response-fic opportunities.
- Challenges.
- “Write next” suggestions.

Use deterministic prompt pools first. AI is optional.

Combine writer interests with instance affinity only when enabled.

## Discovery admin page

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
- Reset.
- Rollback.
- Aggregate evaluation.

## Acceptance

- Opt-out changes both candidates and scores.
- Exact chronological sorts remain chronological.
- Private tastes do not appear in public APIs.
- Missing taste data falls back to baseline.
- Blocked content never enters recommendations.
- Minors receive policy-appropriate discovery rather than default behavioral steering.
- Taste alignment never affects moderation or trust.

---

# 16. Milestone 11: Comments, Forums, Groups, and Messaging

## Comments

Implement:

- Work/chapter comments.
- Replies.
- Appreciation.
- Spoiler formatting.
- Edit history where appropriate.
- Author locking.
- Reporting.
- Block/mute enforcement.

Limit nesting depth; flatten deeper replies with clear references.

## Forums

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

## Groups

Roles:

```text
owner | manager | member
```

Visibility:

```text
public | approval_required | private
```

## Messaging

Implement:

- Conversation invitations.
- Direct messages.
- Group conversations.
- Chat rooms.
- Leave/mute/block/report.
- Unsolicited-message restrictions.
- Minor-protective defaults.

Use ordinary persisted messages plus WebSocket or SSE delivery. Persistence is authoritative; live transport is an optimization.

Do not claim end-to-end encryption.

## Acceptance

- Blocking is enforced consistently.
- A removed group member loses access.
- Private topics cannot be fetched by ID.
- Message reporting gives moderators only appropriately scoped evidence.
- Reconnection does not duplicate messages.
- Rate limits prevent obvious flooding.

---

# 17. Milestone 12: Collections, Challenges, Requests, and Writing Events

## Collections

States:

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

## Challenges

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

Represent challenge variants as configuration over a shared workflow rather than separate unrelated systems.

## Mentorship

Implement:

- Mentor applications.
- Availability.
- Interest matching.
- Invitations.
- Session/task completion.
- Reporting.
- Private participation settings.

## Sprints

Implement:

- Start/end time.
- Optional shared room.
- Private or public counters.
- Manual word-count updates.
- No publication requirement.
- No compulsory streaks.

## Requests and bounties

Build request handling now; attach credit escrow after the ledger milestone.

## Acceptance

- Anonymous challenge identities remain hidden until reveal.
- Owners cannot bypass permission boundaries through collections.
- Missed deadlines have defined outcomes.
- Generated challenges never invent human sponsors or community demand.

---

# 18. Milestone 13: Trust, Reports, Quorum, and Appeals

## Trust model

Separate:

1. Account reliability TL0–TL6.
2. Scoped expertise.
3. Appointed staff roles.

Reference eligibility:

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

## Trust effects

Trust may increase:

- Rate limits.
- Batch sizes.
- Proposal eligibility.
- Curator eligibility.
- Extension quotas.
- Gift/bounty limits.

It does not automatically grant private-message access or administrator powers.

## Report workflow

```text
submitted
→ triaged
→ investigating
→ proposal
→ decision
→ appealed
→ closed/reversed
```

Trust weights prioritize review; they do not establish guilt.

Use unique account identity internally to prevent multiple pseuds counting as independent reporters or voters.

## Quorum defaults

- Routine tag change: two independent approvals.
- High-impact tag change: three.
- Routine sanction: proposer plus independent reviewer.
- Permanent ban: three eligible reviewers where staffing permits.
- Appeal: reviewers independent of the original decision.
- Emergency containment: one authorized moderator, followed by review.

## Emergency actions

- Temporarily hide.
- Freeze replies.
- Restrict messaging.
- Suspend posting.
- Disable extension.

Require reason, review deadline, escalation, and audit entry.

## Bootstrap mode

A small instance may have only one administrator. Label this honestly; do not call a single-person decision quorum.

## Public modlog

Publish redacted decision summaries, not private evidence.

## Acceptance

- A reviewer cannot approve their own proposal.
- Two pseuds from one account count as one participant.
- Appeals exclude original decision-makers.
- Temporary actions trigger review deadlines.
- Public logs omit private identities and evidence.
- Credits cannot change trust eligibility.

---

# 19. Milestone 14: Credits, Fair Queues, and Billing

## Ledger

Use balanced transaction entries rather than updating a wallet balance without history.

Each transaction records:

```text
transaction_id
type
idempotency_key
reference
entries[]
created_at
```

Maintain balances transactionally or derive them with verified cached totals.

Separate:

- Earned credits.
- Subscription grants.
- Purchased credits, if enabled.
- Held credits.

## Job charging

```text
quote
→ reserve credits
→ submit job
→ complete
→ capture actual charge
```

On failure:

```text
release hold or apply documented partial charge
```

## Initial economy

| Action | Credits |
|---|---:|
| Daily regeneration | 10, capped regenerated balance |
| First publication | 20, once and abuse-reviewed |
| Accepted challenge completion | 10, monthly cap |
| Reviewed mentorship | 10, monthly cap |
| Recognized constructive review | 2, strict caps |
| Reviewed governance contribution | 5 per approved batch, not per sanction |

Priority costs:

| Job | Credits |
|---|---:|
| Import priority | 2 base |
| Additional 20-chapter batch | 1 |
| EPUB/HTML/text priority | 1 |
| PDF/MOBI priority | 3 |
| Background report | 5 |
| AI | Explicit estimate |

Standard jobs remain free within fair-use limits.

## Scheduling

Use weighted queues with aging.

Reserve capacity for standard jobs. Paid demand must not starve free users.

## Bounties

States:

```text
funded
→ claimed
→ submitted
→ accepted
→ paid
```

Alternative outcomes:

```text
expired | disputed | refunded | canceled
```

## Billing

Implement optional configurable plans:

- Supporter: €3/month.
- Creator: €8/month.
- Patron: €15/month.

No unlimited compute. No purchased trust. No search-ranking advantage.

Webhook handlers must:

- Verify signatures.
- Store event IDs.
- Be idempotent.
- Handle out-of-order events.
- Reconcile subscription state.

## Acceptance

- Concurrent spending cannot overdraw.
- Retried webhooks do not duplicate grants.
- Failed jobs release holds.
- Standard jobs eventually execute under paid load.
- Billing-disabled operation retains the whole core archive.

---

# 20. Milestone 15: Marketplace and Extension Isolation

## Package manifest

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

## Implement extension categories

- Reader widgets.
- Themes.
- Recommendation engines.
- Search helpers.
- Writing tools.
- Challenge variants.
- Integrations.

## Security model

Use capability grants:

- Public metadata access.
- Selected preference access.
- User-selected draft access.
- Approved network domains.
- Specific widget slots.

No implicit access to all drafts, messages, hidden pseud linkage, or arbitrary network destinations.

## WASM execution

Start with `wasmi` to favor a smaller, simpler interpreter-based WASM execution component inside the native Rust application. The application backend remains natively compiled; plugin bytecode execution is a separate subsystem.

Use:

- Fuel budgets.
- Memory ceilings.
- Output limits.
- Host-call limits.
- Bounded concurrency.
- Process isolation for server-side workers where practical.
- Worker termination for hard wall-clock enforcement.
- Timeouts on host I/O.

Trust changes ceilings, not permissions.

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

Benchmark execution budgets rather than assuming fuel maps directly to milliseconds.

## Review workflow

```text
submitted
→ automated checks
→ permission review
→ security review
→ independent approval
→ published
```

Updates that request new permissions require renewed user consent.

## Themes

Use tokens, layout slots, and scoped styling. Provide:

- Preview.
- Reset.
- Safe mode.
- Accessibility validation.
- No arbitrary JavaScript disguised as CSS.

## Paid marketplace

Implement separately from credits:

- Purchases.
- Refunds.
- Developer revenue accounting.
- Optional processor-backed payouts.
- Configurable reference split of 85/15.
- Operator tax/invoicing setup.

## Acceptance

- Infinite-loop plugins terminate.
- Memory growth beyond limits fails safely.
- Permission denial is enforced by host APIs.
- Revoked packages stop running.
- Uninstall removes grants.
- A theme cannot permanently hide the reset interface.
- Custom engines honor opt-out contracts.

---

# 21. Milestone 16: Feeds, Push, Federation, and Optional AI

## Notifications

Use outbox-driven delivery:

```text
domain event
→ notification eligibility
→ in-app record
→ optional email/push job
```

Apply privacy and rating restrictions before generating notification text.

## Push

- Request permission only after a relevant user action.
- Store subscriptions per device.
- Remove invalid subscriptions.
- Provide generic lock-screen text by default.
- Treat delivery as best-effort.

## RSS, Atom, OPDS

Provide public feeds and scoped revocable private feeds.

Never log private feed tokens.

## ActivityPub

Initial real scope:

- Public opted-in pseud actors.
- Follow/unfollow.
- Work announcements.
- Updates.
- Deletion notices.

Implement:

- Signature verification.
- Delivery retries.
- Replay/abuse controls.
- SSRF protections.
- Remote actor cache.
- Instance blocks.
- Opt-out and deletion documentation.

Do not federate private imports, reading histories, or hidden account linkage.

## AI

Define a provider interface for:

- Summarization.
- Translation.
- Grammar assistance.
- Prompt assistance.
- Embeddings.

Requirements:

- Disabled without configuration.
- Explicit private-text consent.
- Quoted costs.
- Cancellation.
- No generated text published automatically.
- AI output clearly distinguished from author text.
- Semantic search respects the same permissions and exclusions as ordinary search.

---

# 22. Milestone 17: Administration, Privacy Tools, Backups, and Upgrades

## Administration

Routes include:

```text
/admin
/admin/users
/admin/policies
/admin/discovery
/admin/importers
/admin/jobs
/admin/storage
/admin/billing
/admin/extensions
/admin/federation
/admin/audit
/admin/health
/admin/backups
```

Show:

- Memory and disk pressure.
- Job backlog.
- Failed imports.
- Converter availability.
- Backup age.
- Moderation backlog.
- Database health.
- Integration status.

Do not expose individual reading histories in ordinary operational dashboards.

## Data export

Export:

- Account settings.
- Pseuds.
- Authored works.
- Private library metadata.
- Bookmarks.
- Notes.
- Relevant messages.
- Credit history.
- Consent/authorization records where appropriate.

Use a job and expiring authenticated download.

## Deletion

Distinguish:

- Delete account.
- Delete pseud.
- Delete work.
- Orphan eligible work.
- Delete private library copy.
- Delete local offline data.

Document lawful retention exceptions and backup expiry.

## Backups

SQLite:

- Use the SQLite backup API or a consistent snapshot method.
- Do not simply copy a live database file while ignoring WAL state.

PostgreSQL:

- Use supported logical/physical backup tooling.
- Include file-storage consistency strategy.

Restore tests must verify actual reading of restored works, not just successful archive extraction.

## Upgrades

```text
check compatibility
→ maintenance/coordination
→ backup
→ install verified executable
→ migrate
→ health check
→ resume
```

Do not assume replacing the old binary reverses database migrations.

---

# 23. Milestone 18: Hardening and Release

## Functional browser journeys

Automate:

1. Register → create pseud → publish → read.
2. Import → update → bookmark → download.
3. Download → disconnect → read offline.
4. Search protagonist attribute → exclude ship.
5. Change recommendation engine → opt out.
6. Report → quorum → appeal.
7. Earn credits → reserve → spend/refund.
8. Install extension → deny permission → uninstall.
9. Export account → delete account.
10. Backup → restore to a clean instance.

## Security tests

Cover:

- Stored XSS.
- CSRF.
- SSRF.
- File traversal.
- Malicious archives.
- Object-level authorization.
- Pseud isolation.
- Session revocation.
- Credit races.
- Webhook replay.
- Plugin resource exhaustion.
- Private cache leakage.

## Accessibility

Automate what can be automated, then manually test:

- Keyboard-only navigation.
- Screen-reader reading and forms.
- Dialog behavior.
- Editor usability.
- Reader settings.
- Zoom and mobile layouts.
- Reduced motion.
- Contrast.

## Performance

Create a reproducible seed dataset:

- 10,000 works.
- 1,000 accounts.
- Realistic chapters and metadata.
- Large works.
- Dense tags.
- Forum and bookmark activity.

Benchmark:

- Idle memory.
- Read latency.
- Search latency.
- Progress updates.
- Concurrent browsing.
- One heavy conversion.
- Import backlog.
- Extension execution.
- Disk growth.

Define “100 concurrent users” as a reproducible request mix and think time.

Record actual results. If the 1 GB profile fails, reduce concurrency/cache budgets or identify the unsupported workload rather than claiming success.

---

# 24. Route and API Ownership

Assign each frontend feature to one backend module.

| Surface | Frontend routes | API owner |
|---|---|---|
| Public discovery | `/`, `/discover`, `/blind-date` | discovery |
| Search | `/search`, `/search/advanced`, `/find-fic` | search |
| Reader | `/works/*`, `/series/*` | content/library |
| Writing | `/write/*` | content |
| Imports/library | `/library/*` | imports/library |
| Profiles | `/u/:handle` | identity/content |
| Community | `/forum/*`, `/groups/*`, `/messages/*` | community |
| Events | `/challenges/*`, `/requests/*` | community |
| Trust/moderation | `/trust/*`, `/moderation/*`, `/curation/*` | governance |
| Credits/billing | `/credits/*`, `/billing` | economy |
| Marketplace | `/marketplace/*` | extensions |
| Settings | `/settings/*` | relevant module |
| Admin | `/admin/*` | restricted administration |

Generate an endpoint inventory from implemented routes and compare it against requirements in CI.

For every endpoint document:

- Authentication.
- Authorization.
- Request schema.
- Response schema.
- Error codes.
- Rate limit.
- Idempotency.
- Privacy classification.

---

# 25. Tutorial Delivery Plan

The tutorial should follow the milestones above.

Each chapter contains:

1. **Starting checkpoint.**
2. **What will work by the end.**
3. **Concepts introduced.**
4. **Commands.**
5. **Exact file changes.**
6. **Explanation of important code.**
7. **Tests.**
8. **Expected UI behavior.**
9. **Troubleshooting.**
10. **Commit/checkpoint.**

Suggested checkpoints:

```text
v0.01-running-app
v0.02-design-system
v0.03-identity
v0.04-publishing
v0.05-reader
v0.06-jobs
v0.07-importing
v0.08-offline
v0.09-library
v0.10-search
v0.11-discovery
v0.12-community
v0.13-events
v0.14-governance
v0.15-economy
v0.16-marketplace
v0.17-integrations
v0.18-operations
v1.0-release
```

No required functionality should be left as “an exercise for the reader.”

---

# 26. Final Completion Checklist

Before calling the platform complete, verify:

### Core product
- [ ] Authors can create, revise, publish, and complete works.
- [ ] Readers can read publicly without unnecessary registration.
- [ ] Private imports remain private.
- [ ] Supported importers have documented evidence.
- [ ] Downloads are valid.
- [ ] Downloaded reading works offline.
- [ ] Draft conflicts never silently destroy text.

### Search and discovery
- [ ] Main/supporting distinctions work.
- [ ] Character attributes bind to the correct character.
- [ ] Negative ship filtering does not remove characters.
- [ ] Unknown metadata is handled explicitly.
- [ ] Exact search remains exact.
- [ ] Administrator tastes remain private.
- [ ] Influence updates automatically.
- [ ] Opt-out works across all recommendation and writing surfaces.

### Community and governance
- [ ] Pseuds remain compartmentalized.
- [ ] Reports, sanctions, appeals, and quorum are implemented.
- [ ] Emergency actions receive review.
- [ ] Trust cannot be purchased.
- [ ] Public logs are redacted.
- [ ] Child-related privacy and content controls operate server-side.

### Economy and extensibility
- [ ] Credits cannot double-spend.
- [ ] Free queues cannot starve.
- [ ] Billing can be disabled.
- [ ] Plugins have bounded execution at every trust level.
- [ ] Permissions are enforceable.
- [ ] Revocation and rollback work.

### Operations
- [ ] SQLite and PostgreSQL pass integration tests.
- [ ] Backups restore successfully.
- [ ] Upgrades have migration safeguards.
- [ ] ARM64 and x86-64 builds are addressed.
- [ ] Resource claims are measured.
- [ ] Accessibility has manual as well as automated verification.
- [ ] Documentation and tutorial match the delivered repository.

The resulting project should be judged by these working behaviors, not by the number of screens, lines of code, or features named in a README.
