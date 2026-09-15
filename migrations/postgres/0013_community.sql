-- Drop the inert placeholder tables from migration 0001: no code ever wrote
-- them, and 0013 replaces them with the real community schema (twin of the
-- SQLite file, whose header states the design).
DROP TABLE IF EXISTS mutes;
DROP TABLE IF EXISTS blocks;

CREATE TABLE comments (
    id TEXT PRIMARY KEY,
    subject_type TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    author_pseud TEXT NOT NULL,
    body TEXT NOT NULL,
    body_version TEXT NOT NULL,
    classification_id TEXT,
    created_at TEXT NOT NULL,
    edited_at TEXT,
    deleted_at TEXT
);
CREATE INDEX comments_subject ON comments (subject_type, subject_id, created_at);
CREATE INDEX comments_author ON comments (author_pseud);

CREATE TABLE comment_threads (
    id TEXT PRIMARY KEY,
    subject_type TEXT NOT NULL,
    subject_id TEXT NOT NULL,
    root_comment TEXT NOT NULL,
    reply_count BIGINT NOT NULL DEFAULT 0
);

CREATE TABLE forum_categories (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    position BIGINT NOT NULL,
    min_trust BIGINT NOT NULL DEFAULT 0
);

CREATE TABLE forum_topics (
    id TEXT PRIMARY KEY,
    category_id TEXT NOT NULL,
    author_pseud TEXT NOT NULL,
    title TEXT NOT NULL,
    created_at TEXT NOT NULL,
    last_post_at TEXT,
    locked BOOLEAN NOT NULL DEFAULT FALSE
);
CREATE INDEX forum_topics_category ON forum_topics (category_id, last_post_at);

CREATE TABLE forum_posts (
    id TEXT PRIMARY KEY,
    topic_id TEXT NOT NULL,
    author_pseud TEXT NOT NULL,
    body TEXT NOT NULL,
    created_at TEXT NOT NULL,
    deleted_at TEXT
);
CREATE INDEX forum_posts_topic ON forum_posts (topic_id, created_at);

CREATE TABLE groups (
    id TEXT PRIMARY KEY,
    name TEXT NOT NULL,
    privacy TEXT NOT NULL,
    owner TEXT NOT NULL,
    created_at TEXT NOT NULL
);

CREATE TABLE group_members (
    group_id TEXT NOT NULL,
    account TEXT NOT NULL,
    role TEXT NOT NULL,
    joined_at TEXT NOT NULL,
    PRIMARY KEY (group_id, account)
);

CREATE TABLE conversations (
    id TEXT PRIMARY KEY,
    created_at TEXT NOT NULL
);

CREATE TABLE conversation_participants (
    conversation_id TEXT NOT NULL,
    account TEXT NOT NULL,
    last_read_at TEXT,
    muted_until TEXT,
    PRIMARY KEY (conversation_id, account)
);

CREATE TABLE messages (
    id TEXT PRIMARY KEY,
    conversation_id TEXT NOT NULL,
    sender TEXT NOT NULL,
    body TEXT NOT NULL,
    sent_at TEXT NOT NULL,
    deleted_at TEXT
);
CREATE INDEX messages_conversation ON messages (conversation_id, sent_at);

CREATE TABLE blocks (
    blocker TEXT NOT NULL,
    blocked TEXT NOT NULL,
    scope TEXT NOT NULL,
    created_at TEXT NOT NULL,
    note TEXT,
    PRIMARY KEY (blocker, blocked, scope)
);

CREATE TABLE mutes (
    muter TEXT NOT NULL,
    muted TEXT NOT NULL,
    until TEXT,
    created_at TEXT NOT NULL,
    PRIMARY KEY (muter, muted)
);

CREATE TABLE presence (
    account TEXT PRIMARY KEY,
    last_seen_at TEXT NOT NULL,
    typing_until TEXT,
    enabled BOOLEAN NOT NULL DEFAULT false
);
