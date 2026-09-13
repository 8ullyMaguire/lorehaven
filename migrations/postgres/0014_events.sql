-- Migration 0014 — events (spec §18).
--
-- Dialect: PostgreSQL.
-- Columns mirror the SQLite file exactly in name and order (see
-- docs/adr/0004-timestamp-and-identifier-storage.md for the UUID-as-text
-- convention). Types may differ by dialect; the migration test asserts
-- columns and indexes match, not types.

CREATE TABLE collections (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    description TEXT,
    owner TEXT NOT NULL,
    item_policy TEXT NOT NULL,
    is_public INTEGER NOT NULL DEFAULT 1,
    created_at TEXT NOT NULL
);
CREATE INDEX collections_owner ON collections (owner, created_at);

CREATE TABLE collection_items (
    collection_id TEXT NOT NULL,
    work_id TEXT NOT NULL,
    added_by TEXT NOT NULL,
    added_at TEXT NOT NULL,
    note TEXT,
    PRIMARY KEY (collection_id, work_id)
);
CREATE INDEX collection_items_work ON collection_items (work_id);

CREATE TABLE challenges (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    rules TEXT NOT NULL,
    schedule TEXT NOT NULL,
    created_by TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE challenge_entries (
    challenge_id TEXT NOT NULL,
    work_id TEXT NOT NULL,
    entered_at TEXT NOT NULL,
    constraint_check TEXT NOT NULL,
    PRIMARY KEY (challenge_id, work_id)
);

CREATE TABLE requests (
    id TEXT PRIMARY KEY,
    requester TEXT NOT NULL,
    prompt TEXT NOT NULL,
    anonym_until TEXT,
    created_at TEXT NOT NULL
);
CREATE INDEX requests_created ON requests (created_at);

CREATE TABLE claims (
    request_id TEXT NOT NULL,
    claimant TEXT NOT NULL,
    claimed_at TEXT NOT NULL,
    fulfilled_by_work TEXT,
    fulfilled_at TEXT,
    PRIMARY KEY (request_id, claimant)
);

CREATE TABLE wishlists (
    account TEXT PRIMARY KEY,
    is_public INTEGER NOT NULL DEFAULT 0
);

CREATE TABLE wishlist_items (
    wishlist TEXT NOT NULL,
    node_id TEXT,
    work_id TEXT,
    note TEXT,
    added_at TEXT NOT NULL
);
CREATE INDEX wishlist_items_node ON wishlist_items (node_id);
CREATE INDEX wishlist_items_work ON wishlist_items (work_id);

CREATE TABLE events (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    document TEXT NOT NULL,
    created_by TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE event_participation (
    event_id TEXT NOT NULL,
    account TEXT NOT NULL,
    joined_at TEXT NOT NULL,
    PRIMARY KEY (event_id, account)
);
