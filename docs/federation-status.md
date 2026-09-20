# Lorehaven Federation — Progress Update

## What's Done

### New migration: `0043_federation.sql` (postgres + sqlite)
- `instance_themes` — per-instance theme vector (JSON), `public` flag (default FALSE), timestamps
- `instance_fingerprints` — signed 30-day validity, theme_vector/cultural/content signals
- `ap_actors` — AP actors linked to users/instances
- `ap_activities` — activity log with JSON payloads
- `ap_follows` — follow relationships with accepted flag
- `federation_queue` — delivery queue with retry counter
- `federation_peers_v2` — similarity scores + state machine (unknown/pending/friendly/muted)

### DB layer: `crates/db/src/instance_theme.rs`
- `compute_theme_from_bookmarks()` — aggregates tags from bookmarks + private_tags + reading_status
- `upsert_theme()` / `get_local_theme()` / `list_public_themes()` / `tag_weights()`
- **Private by default** — only appears in `list_public_themes()` if `public = TRUE`

### DB layer: `crates/db/src/federation.rs`
- `ApActor`, `ApActivity`, `ApFollow`, `InstanceFingerprint`, `FederationPeer`, `SimilarityBreakdown`
- CRUD for actors/activities/follows
- `regenerate_fingerprint()` — builds signed JSON doc from theme + cultural + content signals
- `compute_similarity()` — multi-dimensional (60% Jaccard on theme tags, 25% cultural, 15% content)
- `upsert_peer()` / `list_similar_peers()` / `list_all_peers()` / queue management

### Routes: `crates/app/src/routes/federation.rs`
- `GET /api/v1/federation/theme` — admin: view theme vector + visibility
- `POST /api/v1/federation/theme/recompute` — admin: recompute from bookmarks
- `PUT /api/v1/federation/theme` — admin: set visibility (public/private)
- `GET /api/v1/federation/themes` — public: list public instance themes (discovery)
- `GET /api/v1/federation/peers` — admin: all peers with similarity
- `GET /api/v1/federation/peers/similar?threshold=30&limit=50` — admin
- `POST /api/v1/federation/peers` — admin: set peer state
- `POST /api/v1/federation/inbox` — public: ActivityPub inbox
- `GET /api/v1/federation/actor` — public: local AP actor profile
- `GET /api/v1/federation/outbox` — admin: outgoing activities

### Integration
- Registered in `crates/db/src/lib.rs` as `pub mod federation;` and `pub mod instance_theme;`
- Registered in `crates/app/src/routes/mod.rs` as `pub mod federation;`
- Added to router in `crates/app/src/server.rs` via `routes::federation::routes()`

### Theme Vector Computation
Weights (admin instance, evolves over time):
- Tags from bookmarks: 3.0 each
- Tags from private_tags: 2.0 each
- Tags from completed reading_status: 1.0 each
- Sorted desc, capped at top 100 tags

## Your Private Preferences (admin instance)
Your themes are **dark fics, dark protagonist**. These will emerge automatically as you bookmark/tag works — the system reads from your `bookmarks`, `private_tags`, and `reading_status` tables. The theme vector starts empty and refines over time. It stays private unless you explicitly call `PUT /api/v1/federation/theme` with `{"public": true}`.

## Still To Do
1. Build + deploy binary
2. Run migrations against lorehaven DB
3. Configure instance fingerprints (initial cultural/content signals)
4. Seed initial peer list for discovery
5. Test ActivityPub inbox with another instance
6. Add scheduled job to recompute theme vector periodically
7. Add similarity sweep against known peers

## Build Status
Build started in background — will notify when done.
