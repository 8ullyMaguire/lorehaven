-- M36 — Instance theme, fingerprint, ActivityPub federation
-- Private theme preferences that evolve from bookmarks, used for similarity-based federation

-- Theme preference vectors per instance (kept private by default)
CREATE TABLE instance_themes (
    instance_id     TEXT PRIMARY KEY,
    theme_vector    TEXT NOT NULL DEFAULT '{}',  -- JSON: {tag: weight, ...}
    public          BOOLEAN NOT NULL DEFAULT FALSE,
    computed_at     TEXT NOT NULL,
    updated_at      TEXT NOT NULL
);
CREATE INDEX idx_instance_themes_public ON instance_themes(public) WHERE public = TRUE;

-- Instance fingerprints for federation similarity
CREATE TABLE instance_fingerprints (
    id                  TEXT PRIMARY KEY,
    instance_host       TEXT NOT NULL UNIQUE,
    fingerprint_version INTEGER NOT NULL DEFAULT 1,
    fingerprint_json    TEXT NOT NULL,              -- full JSON document
    theme_vector        TEXT,                       -- JSON, may be NULL (Jaccard fallback)
    cultural_signals    TEXT NOT NULL DEFAULT '{}', -- JSON
    content_signals     TEXT NOT NULL DEFAULT '{}', -- JSON
    signature           TEXT NOT NULL,
    valid_until         TEXT NOT NULL,
    created_at          TEXT NOT NULL
);
CREATE INDEX idx_instance_fingerprints_host ON instance_fingerprints(instance_host, valid_until DESC);
CREATE INDEX idx_instance_fingerprints_valid ON instance_fingerprints(valid_until) WHERE valid_until > datetime('now');

-- ActivityPub actors (mapped to lorehaven users/instances)
CREATE TABLE ap_actors (
    id              TEXT PRIMARY KEY,
    actor_type      TEXT NOT NULL,          -- 'person' | 'service' | 'instance'
    user_id         TEXT,                   -- NULL for instance-level actors
    instance_host   TEXT,                   -- NULL for local users
    ap_id           TEXT NOT NULL UNIQUE,   -- the @user@host or https://host/actors/...
    inbox_url       TEXT NOT NULL,
    outbox_url      TEXT NOT NULL,
    followers_url   TEXT,
    following_url   TEXT,
    public_key      TEXT NOT NULL,
    created_at      TEXT NOT NULL
);
CREATE INDEX idx_ap_actors_user ON ap_actors(user_id) WHERE user_id IS NOT NULL;
CREATE INDEX idx_ap_actors_host ON ap_actors(instance_host) WHERE instance_host IS NOT NULL;

-- ActivityPub activities (outgoing + incoming)
CREATE TABLE ap_activities (
    id              TEXT PRIMARY KEY,
    activity_type   TEXT NOT NULL,          -- 'Create' | 'Follow' | 'Accept' | 'Announce' | 'Like'
    actor_id        TEXT NOT NULL REFERENCES ap_actors(id),
    object_id       TEXT,                   -- target work/activity/actor
    object_type     TEXT,                   -- 'Work' | 'Actor' | 'Activity'
    payload         TEXT NOT NULL DEFAULT '{}', -- JSON
    created_at      TEXT NOT NULL,
    published_at    TEXT
);
CREATE INDEX idx_ap_activities_actor ON ap_activities(actor_id, created_at DESC);
CREATE INDEX idx_ap_activities_type ON ap_activities(activity_type, created_at DESC);

-- ActivityPub follow relationships
CREATE TABLE ap_follows (
    id                  TEXT PRIMARY KEY,
    follower_actor_id   TEXT NOT NULL REFERENCES ap_actors(id),
    followed_actor_id   TEXT NOT NULL REFERENCES ap_actors(id),
    accepted            BOOLEAN NOT NULL DEFAULT FALSE,
    created_at          TEXT NOT NULL,
    UNIQUE (follower_actor_id, followed_actor_id)
);
CREATE INDEX idx_ap_follows_followed ON ap_follows(followed_actor_id, accepted);
CREATE INDEX idx_ap_follows_follower ON ap_follows(follower_actor_id, accepted);

-- Federation delivery queue
CREATE TABLE federation_queue (
    id              TEXT PRIMARY KEY,
    activity_id     TEXT NOT NULL REFERENCES ap_activities(id),
    target_inbox    TEXT NOT NULL,
    attempts        INTEGER NOT NULL DEFAULT 0,
    status          TEXT NOT NULL DEFAULT 'pending',  -- 'pending' | 'sent' | 'failed'
    created_at      TEXT NOT NULL,
    processed_at    TEXT
);
CREATE INDEX idx_federation_queue_status ON federation_queue(status, created_at);

-- Federation peer states (similarity + decision)
CREATE TABLE federation_peers_v2 (
    id              TEXT PRIMARY KEY,
    peer_host       TEXT NOT NULL UNIQUE,
    similarity      REAL NOT NULL DEFAULT 0.0,
    state           TEXT NOT NULL DEFAULT 'unknown',  -- 'unknown' | 'pending' | 'friendly' | 'muted'
    last_checked    TEXT,
    auto_federate   BOOLEAN NOT NULL DEFAULT FALSE,
    set_by          TEXT,
    set_at          TEXT
);
CREATE INDEX idx_federation_peers_v2_state ON federation_peers_v2(state, similarity DESC);
