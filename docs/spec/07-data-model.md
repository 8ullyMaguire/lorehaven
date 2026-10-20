# 7. Data model

**Status:** reserved
**Milestone:** M1
**Source:** the corresponding section of the Darkhold specification

### 7.1 Principles

1. **No JSON blobs where the data is searched, filtered, or joined.**
2. **Every mutable table carries `id, created_at, updated_at, version`.** `version` is the
   optimistic-concurrency counter; a mismatched `expected_version` returns `409 REVISION_CONFLICT`.
3. **Timestamps are RFC 3339 UTC `TEXT` in both engines.** `TIMESTAMPTZ` was rejected because it
   forces a per-dialect decode on *every* query touching a timestamp.
4. **Every PostgreSQL column read as `i64` is `BIGINT`.** 38 columns were fixed in lorehaven after
   the first live PG run found 8 defects.
5. **`ON CONFLICT` targets name the index and repeat its predicate.**
6. **Repository statements come in pairs**: `db.sql(sqlite_sql, postgres_sql)`.
7. **Foreign keys are real in both engines**, with a `KNOWN_FK_DIVERGENCES` allowlist whose entries
   must not name a divergence that no longer exists.
8. **Derived values are derived.** Counts and aggregates are computed from rows, never stored as
   counters that can drift.

### 7.2 Table inventory

| group | tables |
|---|---|
| identity | `accounts` · `pseuds` · `public_pseud_links` · `password_credentials` · `sessions` · `recovery_tokens` · `second_factors` · `api_tokens` · `invitations` · `registration_applications` · `age_assessments` · `guardian_authorizations` · `supporter_subscriptions` |
| settings | `privacy_settings` · `content_settings` · `search_settings` · `content_filters` · `notification_preferences` · `notification_routes` · `blocks` · `mutes` |
| content | `works` · `work_contributors` · `units` · `revisions` · `publication_events` · `series` · `series_entries` · `work_relations` · `media_assets` · `media_references` · `media_editions` · `media_rights` · `quality_signals` · `collaboration_invites` · `work_feedback_preferences` |
| sources | `source_sites` · `work_sources` · `work_identities` · `work_identity_members` · `merge_proposals` · `split_proposals` · `proposal_votes` · `source_credentials` · `credential_audit_events` · `adapter_versions` |
| taxonomy | `canonical_entities` · `entity_aliases` · `entity_edges` · `entity_closure` · `work_tags` · `work_tag_votes` · `work_characters` · `work_character_attributes` · `work_character_roles` · `relationships` · `relationship_participants` · `work_relationships` · `work_relationship_dynamics` · `moods` · `work_moods` · `content_warnings` · `work_warnings` · `metadata_suggestions` · `metadata_corrections` |
| themes & channels | `themes` · `theme_tags` · `theme_works` · `channels` · `channel_tags` · `channel_works` · `channel_authors` · `channel_subscriptions` · `theme_ballots` · `theme_ballot_choices` · `theme_weight_snapshots` · `theme_vote_periods` |
| instance theme | `instance_theme` · `instance_theme_version` · `instance_theme_works` · `instance_theme_authors` · `instance_theme_tags` · `house_taste_vector` · `theme_oppositions` |
| reading | `bookmarks` · `notes` · `shelves` · `shelf_entries` · `reading_progress` · `reading_events` · `reading_aggregates` · `reading_sessions` · `did_not_finish` · `saved_searches` · `search_alerts` · `author_watches` · `reading_goals` · `typography_preferences` · `reader_sort_preferences` |
| feedback | `ratings` · `reviews` · `review_revisions` · `work_metric_aggregates` · `quick_reactions` · `appreciation_notes` · `cheer_events` · `work_kudos` · `work_highlights` · `work_reactions` · `rating_anomaly_events` · `feedback_holds` · `comment_classifications` |
| community | `forum_categories` · `forum_topics` · `forum_posts` · `polls` · `poll_votes` · `forum_votes` · `forum_meta_votes` · `topic_read_states` · `topic_tags` · `topic_work_links` · `critique_queue` · `prompt_posts` · `groups` · `group_members` · `conversations` · `messages` · `collections` · `collection_entries` · `challenges` · `prompts` · `signups` · `assignments` · `claims` · `fulfillments` · `mentorships` · `sprints` · `wishlist_items` · `requests` · `request_answers` · `request_votes` · `bounties` · `bounty_contributions` |
| governance | `trust_levels` · `trust_events` · `trust_metrics` · `role_assignments` · `reports` · `cases` · `jury_panels` · `jury_panel_members` · `jury_ballots` · `sanctions` · `appeals` · `audit_events` · `quorum_records` · `process_feedback` · `shadowban_actions` · `security_events` |
| taste | `taste_profiles` · `taste_profile_versions` · `taste_vectors` · `work_taste_vectors` · `arena_comparisons` · `arena_ratings` · `arena_user_tag_weights` · `arena_sessions` · `exposure_events` · `work_coordinates` · `permitted_signals` · `demand_weights` · `demand_weight_history` · `demand_diversity_state` · `recipes` · `recipe_versions` · `editorial_picks` · `resonance_scores` · `mood_vectors` · `mood_training_events` · `reader_blocks` |
| extension | `extensions` · `extension_versions` · `extension_installs` · `extension_ratings` · `extension_grants` · `extension_usage` · `extension_submissions` · `adapter_reviews` |
| exchange | `exchange_signals` · `exchange_signal_entities` · `exchange_settings` · `exchange_canonical` · `exchange_latent_demand` |
| jobs & storage | `jobs` · `job_attempts` · `outbox_events` · `content_blobs` · `content_references` · `encryption_keys` · `secrets` · `import_snapshots` · `import_jobs` · `import_attempts` · `provenance_records` |
| ops | `feature_flags` · `ab_test_assignments` · `usage_counters` · `stat_snapshots` · `sitemap_state` · `retention_runs` · `decision_audit` · `announcements` |

### 7.3 The catalogue export

A **catalogue** is a read-only, versioned, self-describing bundle the archive exports for the
taste service and for peers. It is a *projection*, never a copy of the truth:

```
catalogue/
  manifest.json      version, built_at, counts, content_hash, source_site list
  works.jsonl        id, title, author, summary, word_count, chapter_count, completion,
                     language, rating, published_at, updated_at, source_count, tag_ids
  entities.jsonl     id, kind, name, slug, aliases, parent_id, review_status
  work_tags.jsonl    work_id, entity_id, source, confidence, role
  themes.jsonl       theme_id, tag_ids, seed_work_ids
  signals.jsonl      per-reader signal summaries, already anonymised (§15.3)
```

Content-addressed and append-only per version. The taste service records the catalogue version it
ingested, so a ranking is always explainable against a data version. **The archive's database is
never opened by the taste service** (D-7).

---
