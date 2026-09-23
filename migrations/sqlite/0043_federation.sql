-- M36 — Instance theme, fingerprint, ActivityPub federation (SQLite)
-- Private theme preferences that evolve from bookmarks, used for similarity-based federation

CREATE TABLE IF NOT EXISTS instance_themes (
    instance_id     TEXT PRIMARY KEY,
    theme_vector    TEXT NOT NULL DEFAULT '{}',
    public          BOOLEAN NOT NULL DEFAULT FALSE,
    computed_at     TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_instance_themes_public ON instance_themes(public);

CREATE TABLE IF NOT EXISTS instance_fingerprints (
    id                  TEXT PRIMARY KEY,
    instance_host       TEXT NOT NULL UNIQUE,
    fingerprint_version INTEGER NOT NULL DEFAULT 1,
    fingerprint_json    TEXT NOT NULL,
    theme_vector        TEXT,
    cultural_signals    TEXT NOT NULL DEFAULT '{}',
    content_signals     TEXT NOT NULL DEFAULT '{}',
    signature           TEXT NOT NULL,
    valid_until         TEXT NOT NULL,
    created_at          TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_instance_fingerprints_host ON instance_fingerprints(instance_host, valid_until DESC);

CREATE TABLE IF NOT EXISTS ap_actors (
    id              TEXT PRIMARY KEY,
    actor_type      TEXT NOT NULL,
    user_id         TEXT,
    instance_host   TEXT,
    ap_id           TEXT NOT NULL UNIQUE,
    inbox_url       TEXT NOT NULL,
    outbox_url      TEXT NOT NULL,
    followers_url   TEXT,
    following_url   TEXT,
    public_key      TEXT NOT NULL,
    created_at      TEXT NOT NULL
);
CREATE INDEX IF NOT EXISTS idx_ap_actors_user ON ap_actors(user_id);
CREATE INDEX IF NOT EXISTS idx_ap_actors_host ON ap_actors(instance_host);

CREATE TABLE IF NOT EXISTS ap_activities (
    id              TEXT PRIMARY KEY,
    activity_type   TEXT NOT NULL,
    actor_id        TEXT NOT NULL REFERENCES ap_actors(id),
    object_id       TEXT,
    object_type     TEXT,
    payload         TEXT NOT NULL DEFAULT '{}',
    created_at      TEXT NOT NULL,
    published_at    TEXT
);
CREATE INDEX IF NOT EXISTS idx_ap_activities_actor ON ap_activities(actor_id, created_at DESC);
CREATE INDEX IF NOT EXISTS idx_ap_activities_type ON ap_activities(activity_type, created_at DESC);

CREATE TABLE IF NOT EXISTS ap_follows (
    id                  TEXT PRIMARY KEY,
    follower_actor_id   TEXT NOT NULL REFERENCES ap_actors(id),
    followed_actor_id   TEXT NOT NULL REFERENCES ap_actors(id),
    accepted            BOOLEAN NOT NULL DEFAULT FALSE,
    created_at          TEXT NOT NULL,
    UNIQUE (follower_actor_id, followed_actor_id)
);
CREATE INDEX IF NOT EXISTS idx_ap_follows_followed ON ap_follows(followed_actor_id, accepted);
CREATE INDEX IF NOT EXISTS idx_ap_follows_follower ON ap_follows(follower_actor_id, accepted);

CREATE TABLE IF NOT EXISTS federation_queue (
    id              TEXT PRIMARY KEY,
    activity_id     TEXT NOT NULL REFERENCES ap_activities(id),
    target_inbox    TEXT NOT NULL,
    attempts        INTEGER NOT NULL DEFAULT 0,
    status          TEXT NOT NULL DEFAULT 'pending',
    created_at      TEXT NOT NULL,
    processed_at    TEXT
);
CREATE INDEX IF NOT EXISTS idx_federation_queue_status ON federation_queue(status, created_at);

CREATE TABLE IF NOT EXISTS federation_peers_v2 (
    id              TEXT PRIMARY KEY,
    peer_host       TEXT NOT NULL UNIQUE,
    similarity      REAL NOT NULL DEFAULT 0.0,
    state           TEXT NOT NULL DEFAULT 'unknown',
    last_checked    TEXT,
    auto_federate   BOOLEAN NOT NULL DEFAULT FALSE,
    set_by          TEXT,
    set_at          TEXT
);
CREATE INDEX IF NOT EXISTS idx_federation_peers_v2_state ON federation_peers_v2(state, similarity DESC);
