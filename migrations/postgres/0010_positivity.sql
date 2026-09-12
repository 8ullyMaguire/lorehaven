-- 0010 — positivity filter and feedback delivery (spec §12).
--
-- Dialect: PostgreSQL.
--
-- Twin of `migrations/sqlite/0010_positivity.sql`; the header there states the
-- design, the deletion rule and the retention rule. Only the declarations
-- differ here, per ADR 0004:
--
-- * identifiers that reference UUID primary keys are UUID, not TEXT;
-- * integers the repository decodes as `i64` are BIGINT;
-- * booleans are BOOLEAN, read back through `::int::bigint`;
-- * every `?` placeholder that carries a UUID is written `?::uuid`;
-- * table, column and index *names* are identical to the SQLite twin — the
--   drift test compares names, and a name present on one engine and absent on
--   the other is the defect it exists to catch.

CREATE TABLE feedback_preferences (
    account_id             UUID    PRIMARY KEY REFERENCES accounts (id) ON DELETE CASCADE,
    accept_constructive    BOOLEAN NOT NULL DEFAULT FALSE,
    ambiguous_auto_deliver BOOLEAN NOT NULL DEFAULT FALSE,
    comments_enabled       BOOLEAN NOT NULL DEFAULT TRUE,
    author_note            TEXT    NOT NULL DEFAULT '',
    created_at             TEXT    NOT NULL,
    updated_at             TEXT    NOT NULL,
    version                BIGINT  NOT NULL DEFAULT 1
);

CREATE TABLE work_feedback_preferences (
    work_id                UUID    PRIMARY KEY REFERENCES works (id) ON DELETE CASCADE,
    accept_constructive    BOOLEAN,
    ambiguous_auto_deliver BOOLEAN,
    comments_enabled       BOOLEAN,
    author_note            TEXT,
    updated_at             TEXT    NOT NULL,
    version                BIGINT  NOT NULL DEFAULT 1
);

CREATE TABLE review_classifications (
    review_id      UUID    PRIMARY KEY REFERENCES review (id) ON DELETE CASCADE,
    class          TEXT    NOT NULL CHECK (class IN ('positive', 'constructive', 'ambiguous', 'negative')),
    confidence_bp  BIGINT  NOT NULL,
    signals        TEXT    NOT NULL DEFAULT '[]',
    outcome        TEXT    NOT NULL CHECK (outcome IN ('delivered', 'held')),
    classified_at  TEXT    NOT NULL
);

CREATE INDEX review_classifications_outcome ON review_classifications (outcome, classified_at);

CREATE TABLE feedback_allowlist (
    author_account_id UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    trusted_pseud_id  UUID NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    created_at        TEXT NOT NULL,
    PRIMARY KEY (author_account_id, trusted_pseud_id)
);

CREATE TABLE feedback_denylist (
    author_account_id UUID NOT NULL REFERENCES accounts (id) ON DELETE CASCADE,
    refused_pseud_id  UUID NOT NULL REFERENCES pseuds (id) ON DELETE CASCADE,
    created_at        TEXT NOT NULL,
    PRIMARY KEY (author_account_id, refused_pseud_id)
);
