-- Migration 0010 — positivity filter and feedback delivery (spec §12).
--
-- Dialect: SQLite.
-- Timestamps are RFC 3339 UTC text; identifiers are canonical UUID text.
--
-- What this adds:
--
-- * `feedback_preferences`: the account-level default (spec §8.4, §12.3).
--   Constructive critique is opt-in (default off); ambiguous auto-delivery is
--   off by default so rules-only mode holds more for review (§12.2); comments
--   are enabled by default and pausable per work (§12.10).
-- * `work_feedback_preferences`: per-work overrides; NULL means "inherit the
--   account default", so changing the default moves every work that did not
--   opt out explicitly. The effective policy is computed in
--   `lorehaven-domain::positivity::effective`.
-- * `review_classifications`: one row per review — the classifier's verdict
--   and the delivery outcome. Signals name matched categories; they never
--   quote the reviewer's text.
-- * `feedback_allowlist` / `feedback_denylist`: the author's trusted and
--   refused commenters (§12.6). An allowlisted pseud skips classification;
--   a denylisted one is always held.
--
-- Deletion and retention:
--
-- * Preferences cascade with the account; work overrides cascade with the work.
-- * Classifications cascade with the review. A held review stays held until
--   reclassified; nothing here is swept — moderation history is audit value.
-- * Allow/deny rows cascade with both ends (account and pseud).
--
-- Integers the repository decodes as `i64` are INTEGER here and BIGINT in the
-- postgres twin; booleans are INTEGER 0/1 here and BOOLEAN there, read back
-- as integers (ADR 0004). Confidence is basis points (0..10000), never REAL.

CREATE TABLE feedback_preferences (
    account_id             TEXT    PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    accept_constructive    INTEGER NOT NULL DEFAULT 0,
    ambiguous_auto_deliver INTEGER NOT NULL DEFAULT 0,
    comments_enabled       INTEGER NOT NULL DEFAULT 1,
    author_note            TEXT    NOT NULL DEFAULT '',
    created_at             TEXT    NOT NULL,
    updated_at             TEXT    NOT NULL,
    version                INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE work_feedback_preferences (
    work_id                TEXT    PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    accept_constructive    INTEGER,
    ambiguous_auto_deliver INTEGER,
    comments_enabled       INTEGER,
    author_note            TEXT,
    updated_at             TEXT    NOT NULL,
    version                INTEGER NOT NULL DEFAULT 1
);

CREATE TABLE review_classifications (
    review_id      TEXT    PRIMARY KEY REFERENCES review (id) ON DELETE CASCADE,
    class          TEXT    NOT NULL CHECK (class IN ('positive', 'constructive', 'ambiguous', 'negative')),
    confidence_bp  INTEGER NOT NULL,
    signals        TEXT    NOT NULL DEFAULT '[]',
    outcome        TEXT    NOT NULL CHECK (outcome IN ('delivered', 'held')),
    classified_at  TEXT    NOT NULL
);

CREATE INDEX review_classifications_outcome ON review_classifications (outcome, classified_at);

CREATE TABLE feedback_allowlist (
    author_account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    trusted_pseud_id  TEXT NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    created_at        TEXT NOT NULL,
    PRIMARY KEY (author_account_id, trusted_pseud_id)
);

CREATE TABLE feedback_denylist (
    author_account_id TEXT NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    refused_pseud_id  TEXT NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    created_at        TEXT NOT NULL,
    PRIMARY KEY (author_account_id, refused_pseud_id)
);
